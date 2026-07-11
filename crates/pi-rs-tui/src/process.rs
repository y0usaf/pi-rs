//! Live process ownership around the deterministic [`Tui`] lifecycle.
//!
//! The driver owns stdin/stdout, raw mode, resize and termination signals,
//! render pacing, protocol timeouts, input draining, and teardown.  Its callback
//! is deliberately policy-free: Lua decides what to render and when to exit.

use crate::terminal::{
    KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT, ProcessRawModeGuard, TerminalError,
};
use crate::tui::{Tui, TuiError};
use std::io::{self, Write};
use std::time::{Duration, Instant};

pub const MIN_RENDER_INTERVAL: Duration = Duration::from_millis(16);
pub const INPUT_DRAIN_MAX: Duration = Duration::from_millis(1000);
pub const INPUT_DRAIN_IDLE: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    Start { columns: u16, rows: u16 },
    Input(String),
    Resize { columns: u16, rows: u16 },
    Tick,
    Signal(i32),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessControl {
    pub lines: Option<Vec<String>>,
    pub force: bool,
    pub exit: bool,
    pub title: Option<String>,
    pub progress: Option<bool>,
    pub show_hardware_cursor: Option<bool>,
    pub clear_on_shrink: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessExit {
    Requested,
    Signal(i32),
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error(transparent)]
    Terminal(#[from] TerminalError),
    #[error(transparent)]
    Render(#[from] TuiError),
    #[error("process TUI callback failed: {0}")]
    Callback(String),
    #[error("live process TUI is unsupported on this platform")]
    Unsupported,
}

/// Live process driver. Constructing it has no process side effects; `run`
/// acquires and restores the terminal for exactly one session.
pub struct ProcessTui {
    tui: Tui,
    pending_lines: Option<Vec<String>>,
    last_render: Option<Instant>,
}

impl ProcessTui {
    pub fn new(show_hardware_cursor: bool) -> Self {
        let (columns, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        Self {
            tui: Tui::new(
                crate::terminal::TerminalState::new(Some(columns), Some(rows)),
                show_hardware_cursor,
            ),
            pending_lines: None,
            last_render: None,
        }
    }

    pub fn dimensions(&self) -> (u16, u16) {
        self.tui.dimensions()
    }

    fn apply_control(&mut self, control: ProcessControl) {
        if control.force {
            self.tui.request_render(true);
        }
        if let Some(title) = control.title {
            self.tui.set_title(&title);
        }
        if let Some(active) = control.progress {
            self.tui.set_progress(active);
        }
        if let Some(enabled) = control.show_hardware_cursor {
            self.tui.set_show_hardware_cursor(enabled);
        }
        if let Some(enabled) = control.clear_on_shrink {
            self.tui.set_clear_on_shrink(enabled);
        }
        if let Some(lines) = control.lines {
            self.pending_lines = Some(lines);
            self.tui.request_render(control.force);
        }
    }

    fn render_due(&mut self, now: Instant, immediate: bool) -> Result<(), ProcessError> {
        let due = immediate
            || self
                .last_render
                .is_none_or(|last| now.duration_since(last) >= MIN_RENDER_INTERVAL);
        if !due {
            return Ok(());
        }
        if let Some(lines) = self.pending_lines.take()
            && self.tui.render_if_requested(lines)?
        {
            self.last_render = Some(now);
        }
        self.flush_output()?;
        Ok(())
    }

    fn flush_output(&mut self) -> Result<(), ProcessError> {
        let bytes = self.tui.take_output();
        if bytes.is_empty() {
            return Ok(());
        }
        let mut stdout = io::stdout().lock();
        stdout.write_all(&bytes).map_err(TerminalError::from)?;
        stdout.flush().map_err(TerminalError::from)?;
        Ok(())
    }

    fn take_ready<Fut>(
        pending: &mut futures_util::stream::FuturesUnordered<Fut>,
    ) -> Vec<Fut::Output>
    where
        Fut: std::future::Future,
    {
        use futures_util::stream::StreamExt;
        let mut ready = Vec::new();
        loop {
            match futures_util::FutureExt::now_or_never(pending.next()) {
                Some(Some(output)) => ready.push(output),
                Some(None) | None => return ready,
            }
        }
    }

    /// Run the terminal session while allowing event handlers to suspend.
    ///
    /// Handler futures are polled concurrently on the same local executor. This
    /// is deliberate: a handler awaiting a provider stream must not prevent a
    /// later input or tick handler from running, but all Lua state remains on
    /// the single VM thread.
    #[cfg(unix)]
    pub async fn run<F, Fut>(&mut self, mut callback: F) -> Result<ProcessExit, ProcessError>
    where
        F: FnMut(ProcessEvent) -> Fut,
        Fut: std::future::Future<Output = Result<ProcessControl, ProcessError>>,
    {
        use futures_util::stream::FuturesUnordered;
        use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM, SIGWINCH};
        use signal_hook::iterator::Signals;
        use std::os::fd::AsRawFd;

        let mut raw = ProcessRawModeGuard::start_raw_only()?;
        self.tui.start();
        self.flush_output()?;

        let result = async {
            let mut signals =
                Signals::new([SIGWINCH, SIGINT, SIGTERM, SIGHUP]).map_err(TerminalError::from)?;
            let (columns, rows) = self.dimensions();
            let mut pending = FuturesUnordered::new();
            pending.push(callback(ProcessEvent::Start { columns, rows }));
            let mut exit_requested = false;
            let mut next_tick = Instant::now() + MIN_RENDER_INTERVAL;
            let mut negotiation_since: Option<Instant> = None;
            let stdin_fd = io::stdin().as_raw_fd();
            let mut exit = ProcessExit::Requested;

            while !exit_requested {
                let mut events = Vec::new();
                let mut pfd = libc::pollfd {
                    fd: stdin_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let polled = unsafe { libc::poll(&mut pfd, 1, 0) };
                if polled < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(ProcessError::Terminal(TerminalError::Io(error)));
                    }
                } else if polled > 0 && pfd.revents & libc::POLLIN != 0 {
                    let mut bytes = [0_u8; 4096];
                    let count = unsafe {
                        libc::read(
                            stdin_fd,
                            bytes.as_mut_ptr().cast::<libc::c_void>(),
                            bytes.len(),
                        )
                    };
                    if count > 0 {
                        events.extend(
                            self.tui
                                .feed_input(&bytes[..count as usize])
                                .into_iter()
                                .map(ProcessEvent::Input),
                        );
                    }
                } else {
                    events.extend(self.tui.flush_input().into_iter().map(ProcessEvent::Input));
                }

                if self.tui.keyboard_negotiation_pending() {
                    let since = negotiation_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT {
                        events.extend(
                            self.tui
                                .flush_keyboard_negotiation()
                                .into_iter()
                                .map(ProcessEvent::Input),
                        );
                        negotiation_since = None;
                    }
                } else {
                    negotiation_since = None;
                }

                for signal in signals.pending() {
                    if signal == SIGWINCH {
                        let (columns, rows) =
                            crossterm::terminal::size().unwrap_or(self.dimensions());
                        self.tui.resize(Some(columns), Some(rows));
                        events.push(ProcessEvent::Resize { columns, rows });
                    } else {
                        events.push(ProcessEvent::Signal(signal));
                        exit = ProcessExit::Signal(signal);
                        exit_requested = true;
                    }
                }

                let now = Instant::now();
                if now >= next_tick {
                    events.push(ProcessEvent::Tick);
                    next_tick = now + MIN_RENDER_INTERVAL;
                }
                for event in events {
                    pending.push(callback(event));
                }

                // Poll every ready handler result without waiting for suspended
                // handlers. A prompt and later Escape/tick handlers therefore
                // make progress as independent Lua coroutines.
                for control in Self::take_ready(&mut pending) {
                    let control = control?;
                    exit_requested |= control.exit;
                    self.apply_control(control);
                }
                self.render_due(now, false)?;
                if !exit_requested {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
            Ok(exit)
        }
        .await;

        // Teardown must run for callback/render/I/O failures as well as normal exit.
        self.tui.begin_drain();
        let _ = self.flush_output();
        drain_stdin(INPUT_DRAIN_MAX, INPUT_DRAIN_IDLE);
        self.tui.stop();
        let stop_result = self.flush_output();
        let raw_result = raw.restore().map_err(ProcessError::from);
        match result {
            Ok(exit) => {
                stop_result?;
                raw_result?;
                Ok(exit)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(not(unix))]
    pub async fn run<F, Fut>(&mut self, _callback: F) -> Result<ProcessExit, ProcessError>
    where
        F: FnMut(ProcessEvent) -> Fut,
        Fut: std::future::Future<Output = Result<ProcessControl, ProcessError>>,
    {
        Err(ProcessError::Unsupported)
    }
}

#[cfg(unix)]
fn drain_stdin(max: Duration, idle: Duration) {
    use std::os::fd::AsRawFd;
    let fd = io::stdin().as_raw_fd();
    let started = Instant::now();
    let mut bytes = [0_u8; 4096];
    while started.elapsed() < max {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = i32::try_from(idle.as_millis()).unwrap_or(50);
        let ready = unsafe { libc::poll(&mut pfd, 1, timeout) };
        if ready <= 0 || pfd.revents & libc::POLLIN == 0 {
            break;
        }
        let count =
            unsafe { libc::read(fd, bytes.as_mut_ptr().cast::<libc::c_void>(), bytes.len()) };
        if count <= 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_defaults_leave_policy_to_the_callback() {
        assert_eq!(
            ProcessControl::default(),
            ProcessControl {
                lines: None,
                force: false,
                exit: false,
                title: None,
                progress: None,
                show_hardware_cursor: None,
                clear_on_shrink: None,
            }
        );
        assert_eq!(MIN_RENDER_INTERVAL, Duration::from_millis(16));
        assert_eq!(INPUT_DRAIN_IDLE, Duration::from_millis(50));
    }

    #[test]
    fn ready_handler_runs_while_an_earlier_handler_is_suspended() {
        use futures_util::future::LocalBoxFuture;
        use futures_util::stream::FuturesUnordered;
        use std::cell::Cell;
        use std::rc::Rc;

        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
        else {
            return;
        };
        runtime.block_on(async {
            let released = Rc::new(Cell::new(false));
            let pending: FuturesUnordered<LocalBoxFuture<'_, &'static str>> =
                FuturesUnordered::new();
            let waiting = Rc::clone(&released);
            pending.push(Box::pin(async move {
                while !waiting.get() {
                    tokio::task::yield_now().await;
                }
                "prompt"
            }));
            let aborting = Rc::clone(&released);
            pending.push(Box::pin(async move {
                aborting.set(true);
                "escape"
            }));

            let mut pending = pending;
            assert_eq!(ProcessTui::take_ready(&mut pending), vec!["escape"]);
            tokio::task::yield_now().await;
            assert_eq!(ProcessTui::take_ready(&mut pending), vec!["prompt"]);
        });
    }

    #[test]
    fn applying_control_coalesces_latest_frame_and_terminal_policy_bytes() {
        let mut process = ProcessTui {
            tui: Tui::new(
                crate::terminal::TerminalState::new(Some(20), Some(4)),
                false,
            ),
            pending_lines: None,
            last_render: None,
        };
        process.tui.start();
        process.apply_control(ProcessControl {
            lines: Some(vec!["first".into()]),
            title: Some("pi-rs".into()),
            progress: Some(true),
            ..ProcessControl::default()
        });
        process.apply_control(ProcessControl {
            lines: Some(vec!["latest".into()]),
            ..ProcessControl::default()
        });
        let lines = process.pending_lines.take();
        assert_eq!(lines, Some(vec!["latest".into()]));
        let output = String::from_utf8_lossy(&process.tui.take_output()).into_owned();
        assert!(output.contains("\x1b]0;pi-rs\x07"));
        assert!(output.contains(crate::terminal::PROGRESS_ACTIVE_SEQUENCE));
    }
}
