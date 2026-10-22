//! Terminal I/O (upstream `terminal.ts`): raw mode, bracketed paste, Kitty keyboard protocol
//! negotiation with modifyOtherKeys fallback, program status and progress reports.
//!
//! Upstream pushes input to callbacks from Node's event loop. Here the caller polls:
//! [`Terminal::poll`] waits up to a timeout and returns [`TerminalEvent`]s. The negotiation logic
//! lives in [`InputRouter`], which is pure and tested on its own.

use crate::keys::set_kitty_protocol_active;
use crate::program_status::{ProgramStatus, PROGRAM_STATUS_QUERY, format_program_status, is_program_status_reply};
use crate::stdin_buffer::{StdinBuffer, StdinEvent};

const PROGRESS_KEEPALIVE_MS: u64 = 1000;
const PROGRESS_ACTIVE: &str = "\x1b]9;4;3\x07";
const PROGRESS_CLEAR: &str = "\x1b]9;4;0\x07";
const NATIVE_SHIFT_ENTER: &str = "\x1b[13;2u";
const KEYBOARD_FRAGMENT_TIMEOUT_MS: u64 = 150;
const KITTY_QUERY: &str = "\x1b[>7u\x1b[?u";
const DEVICE_ATTRIBUTES_QUERY: &str = "\x1b[c";
const DEFAULT_ESCAPE_TIMEOUT_MS: u64 = 10;
const DEFAULT_SSH_ESCAPE_TIMEOUT_MS: u64 = 100;

/// A reply to the keyboard protocol queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiationReply {
    KittyFlags(u64),
    DeviceAttributes,
}

/// Parse `CSI ? flags u` or a DA1 reply `CSI ? ... c`.
pub fn parse_keyboard_protocol_negotiation_sequence(seq: &str) -> Option<NegotiationReply> {
    let body = seq.strip_prefix("\x1b[?")?;
    if let Some(d) = body.strip_suffix('u') {
        if !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) {
            return Some(NegotiationReply::KittyFlags(d.parse().unwrap_or(u64::MAX)));
        }
        return None;
    }
    let d = body.strip_suffix('c')?;
    d.bytes().all(|b| b.is_ascii_digit() || b == b';').then_some(NegotiationReply::DeviceAttributes)
}

fn is_negotiation_prefix(seq: &str) -> bool {
    seq == "\x1b["
        || seq.strip_prefix("\x1b[?").is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit() || b == b';'))
}

/// Apple Terminal on macOS (it sends plain CR for Shift+Enter).
pub fn is_apple_terminal_session() -> bool {
    cfg!(target_os = "macos") && std::env::var("TERM_PROGRAM").as_deref() == Ok("Apple_Terminal")
}

/// Turn CR into the Kitty Shift+Enter sequence when native detection says Shift is held.
pub fn normalize_native_shift_enter_input(data: &str, detect: bool, shift_pressed: bool) -> String {
    if detect && data == "\r" && shift_pressed { NATIVE_SHIFT_ENTER.into() } else { data.into() }
}

/// How long to wait after ESC before it counts as Escape: `PI_TUI_ESC_TIMEOUT`, longer over SSH.
pub fn resolve_escape_timeout_ms(env: &dyn Fn(&str) -> Option<String>) -> u64 {
    if let Some(v) = env("PI_TUI_ESC_TIMEOUT") {
        if let Ok(n) = v.trim().parse::<f64>() {
            if n.is_finite() && n > 0.0 {
                return n.ceil() as u64;
            }
        }
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if set("SSH_CONNECTION") || set("SSH_TTY") { DEFAULT_SSH_ESCAPE_TIMEOUT_MS } else { DEFAULT_ESCAPE_TIMEOUT_MS }
}

fn process_env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// What the terminal produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// One key, mouse or reply sequence, or a whole bracketed paste re-wrapped in its markers.
    Input(String),
    /// The terminal size changed.
    Resize,
}

/// Routes parsed stdin sequences: answers keyboard-protocol and program-status replies and passes
/// the rest on as input. Writes it needs to make go to `out`.
#[derive(Debug, Default)]
pub struct InputRouter {
    pub kitty_active: bool,
    pub modify_other_keys_active: bool,
    pub keyboard_protocol_pushed: bool,
    pending_device_attributes: u32,
    negotiation_buffer: String,
    negotiation_deadline: Option<u64>,
    pub program_status_supported: bool,
    pub program_status_query_pending: bool,
    pub program_status: Option<ProgramStatus>,
    /// Sends CR as Shift+Enter when this says Shift is held (Apple Terminal, Windows).
    pub native_shift: Option<fn() -> bool>,
}

impl InputRouter {
    /// Start negotiation: returns what to write to the terminal.
    pub fn begin(&mut self, program_status_override: Option<&str>) -> String {
        self.keyboard_protocol_pushed = true;
        self.pending_device_attributes += 1;
        self.clear_negotiation_buffer();
        self.program_status_supported = program_status_override == Some("1");
        self.program_status_query_pending = !matches!(program_status_override, Some("1") | Some("0"));
        let q = if self.program_status_query_pending { PROGRAM_STATUS_QUERY } else { "" };
        let mut out = format!("{KITTY_QUERY}{q}{DEVICE_ATTRIBUTES_QUERY}");
        out.push_str(&self.program_status_write());
        out
    }

    fn program_status_write(&self) -> String {
        match (&self.program_status, self.program_status_supported) {
            (Some(s), true) => format_program_status(s),
            _ => String::new(),
        }
    }

    fn clear_negotiation_buffer(&mut self) {
        self.negotiation_buffer.clear();
        self.negotiation_deadline = None;
    }

    /// Route one stdin event at time `now`. Input goes to `input`, terminal writes to `out`.
    pub fn route(&mut self, ev: StdinEvent, now: u64, input: &mut Vec<String>, out: &mut String) {
        let seq = match ev {
            StdinEvent::Paste(p) => {
                input.push(format!("\x1b[200~{p}\x1b[201~"));
                return;
            }
            StdinEvent::Data(d) => d,
        };
        if is_program_status_reply(&seq) {
            if self.program_status_query_pending {
                self.program_status_query_pending = false;
                self.program_status_supported = true;
                out.push_str(&self.program_status_write());
            }
            return;
        }
        let mut reply: Option<(NegotiationReply, String)> = None;
        if !self.negotiation_buffer.is_empty() {
            let buffered = format!("{}{seq}", self.negotiation_buffer);
            if let Some(r) = parse_keyboard_protocol_negotiation_sequence(&buffered) {
                self.clear_negotiation_buffer();
                reply = Some((r, buffered));
            } else if is_negotiation_prefix(&buffered) {
                self.negotiation_buffer = buffered;
                self.negotiation_deadline.get_or_insert(now + KEYBOARD_FRAGMENT_TIMEOUT_MS);
                return;
            } else {
                let b = std::mem::take(&mut self.negotiation_buffer);
                self.negotiation_deadline = None;
                self.forward(&b, input);
            }
        }
        if reply.is_none() {
            if let Some(r) = parse_keyboard_protocol_negotiation_sequence(&seq) {
                reply = Some((r, seq.clone()));
            } else if is_negotiation_prefix(&seq) {
                self.negotiation_buffer = seq;
                self.negotiation_deadline = Some(now + KEYBOARD_FRAGMENT_TIMEOUT_MS);
                return;
            }
        }
        match reply {
            Some((r, full)) => {
                if !self.handle_negotiation(r, out) {
                    self.forward(&full, input);
                }
            }
            None => self.forward(&seq, input),
        }
    }

    /// Flush a split negotiation fragment as input once its deadline passed.
    pub fn tick(&mut self, now: u64, input: &mut Vec<String>) {
        if self.negotiation_deadline.is_some_and(|d| now >= d) {
            let b = std::mem::take(&mut self.negotiation_buffer);
            self.negotiation_deadline = None;
            if !b.is_empty() {
                self.forward(&b, input);
            }
        }
    }

    pub fn deadline(&self) -> Option<u64> {
        self.negotiation_deadline
    }

    fn handle_negotiation(&mut self, r: NegotiationReply, out: &mut String) -> bool {
        self.clear_negotiation_buffer();
        match r {
            NegotiationReply::DeviceAttributes => {
                if self.pending_device_attributes == 0 {
                    return false;
                }
                self.pending_device_attributes -= 1;
                if self.pending_device_attributes == 0 {
                    self.program_status_query_pending = false;
                }
                if !self.kitty_active {
                    self.enable_modify_other_keys(out);
                }
                true
            }
            NegotiationReply::KittyFlags(flags) => {
                if flags != 0 {
                    self.disable_modify_other_keys(out);
                    if !self.kitty_active {
                        self.kitty_active = true;
                        set_kitty_protocol_active(true);
                    }
                } else {
                    self.enable_modify_other_keys(out);
                }
                true
            }
        }
    }

    fn forward(&self, seq: &str, input: &mut Vec<String>) {
        let detect = seq == "\r" && (is_apple_terminal_session() || cfg!(windows));
        let shift = detect && self.native_shift.is_some_and(|f| f());
        input.push(normalize_native_shift_enter_input(seq, detect, shift));
    }

    pub fn enable_modify_other_keys(&mut self, out: &mut String) {
        if self.kitty_active || self.modify_other_keys_active {
            return;
        }
        out.push_str("\x1b[>4;2m");
        self.modify_other_keys_active = true;
    }

    pub fn disable_modify_other_keys(&mut self, out: &mut String) {
        if !self.modify_other_keys_active {
            return;
        }
        out.push_str("\x1b[>4;0m");
        self.modify_other_keys_active = false;
    }

    /// Pop the Kitty flags and drop modifyOtherKeys (for stop and drain).
    pub fn disable_keyboard_protocols(&mut self, out: &mut String) {
        self.clear_negotiation_buffer();
        if self.keyboard_protocol_pushed || self.kitty_active {
            out.push_str("\x1b[<u");
            self.keyboard_protocol_pushed = false;
            self.kitty_active = false;
            set_kitty_protocol_active(false);
        }
        self.disable_modify_other_keys(out);
    }
}

/// The terminal the TUI draws on. Default methods write the usual control sequences.
pub trait Terminal {
    /// Enter raw mode, enable bracketed paste and start keyboard protocol negotiation.
    fn start(&mut self);
    /// Restore the terminal.
    fn stop(&mut self);
    /// Wait up to `timeout_ms` for input or resize; `None` waits indefinitely.
    fn poll(&mut self, timeout_ms: Option<u64>) -> Vec<TerminalEvent>;
    /// Drain stdin before exit so late Kitty key releases do not reach the shell.
    fn drain_input(&mut self, _max_ms: u64, _idle_ms: u64) {}
    fn write(&mut self, data: &str);
    fn columns(&self) -> usize;
    fn rows(&self) -> usize;
    fn kitty_protocol_active(&self) -> bool {
        false
    }
    /// Write `before`, then this terminal's own mode resets, then `after`, synchronously.
    fn restore_on_exit(&mut self, before: &str, after: &str) {
        self.write(&format!("{before}{after}"));
    }
    fn move_by(&mut self, lines: i64) {
        if lines > 0 {
            self.write(&format!("\x1b[{lines}B"));
        } else if lines < 0 {
            self.write(&format!("\x1b[{}A", -lines));
        }
    }
    fn hide_cursor(&mut self) {
        self.write("\x1b[?25l");
    }
    fn show_cursor(&mut self) {
        self.write("\x1b[?25h");
    }
    fn clear_line(&mut self) {
        self.write("\x1b[K");
    }
    fn clear_from_cursor(&mut self) {
        self.write("\x1b[J");
    }
    fn clear_screen(&mut self) {
        self.write("\x1b[2J\x1b[H");
    }
    fn set_title(&mut self, title: &str) {
        self.write(&format!("\x1b]0;{title}\x07"));
    }
    fn set_progress(&mut self, active: bool) {
        self.write(if active { PROGRESS_ACTIVE } else { PROGRESS_CLEAR });
    }
    fn set_program_status(&mut self, _status: ProgramStatus) {}
}

#[cfg(unix)]
mod sig {
    use std::sync::atomic::{AtomicI32, Ordering};
    pub static WINCH_FD: AtomicI32 = AtomicI32::new(-1);
    extern "C" fn on_winch(_: libc::c_int) {
        let fd = WINCH_FD.load(Ordering::Relaxed);
        if fd >= 0 {
            unsafe { libc::write(fd, b"w".as_ptr().cast(), 1) };
        }
    }
    /// Install the SIGWINCH handler; returns the read end of its self-pipe.
    pub fn install() -> Option<i32> {
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return None;
        }
        for fd in fds {
            unsafe {
                libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK);
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        WINCH_FD.store(fds[1], Ordering::Relaxed);
        unsafe { libc::signal(libc::SIGWINCH, on_winch as *const () as libc::sighandler_t) };
        Some(fds[0])
    }
    pub fn uninstall(read_fd: i32) {
        unsafe { libc::signal(libc::SIGWINCH, libc::SIG_DFL) };
        let w = WINCH_FD.swap(-1, Ordering::Relaxed);
        unsafe {
            if w >= 0 {
                libc::close(w);
            }
            libc::close(read_fd);
        }
    }
}

fn now_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// The process's own terminal (stdin/stdout).
#[cfg(unix)]
pub struct ProcessTerminal {
    saved: Option<libc::termios>,
    started: bool,
    buffer: StdinBuffer,
    router: InputRouter,
    winch_fd: Option<i32>,
    progress_next: Option<u64>,
    pending: std::collections::VecDeque<TerminalEvent>,
    log_path: Option<String>,
}

#[cfg(unix)]
impl Default for ProcessTerminal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
impl ProcessTerminal {
    pub fn new() -> Self {
        ProcessTerminal {
            saved: None,
            started: false,
            buffer: StdinBuffer::new(crate::stdin_buffer::DEFAULT_SEQUENCE_TIMEOUT_MS, resolve_escape_timeout_ms(&process_env)),
            router: InputRouter::default(),
            winch_fd: None,
            progress_next: None,
            pending: Default::default(),
            log_path: std::env::var("PI_TUI_WRITE_LOG").ok().filter(|s| !s.is_empty()).map(|p| {
                if std::path::Path::new(&p).is_dir() {
                    format!("{p}/tui-{}.log", std::process::id())
                } else {
                    p
                }
            }),
        }
    }

    pub fn modify_other_keys_active(&self) -> bool {
        self.router.modify_other_keys_active
    }

    fn raw_write(&self, data: &str) {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(data.as_bytes());
        let _ = out.flush();
    }

    fn size(&self) -> (usize, usize) {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
        let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(d);
        let cols = if ok && ws.ws_col > 0 { ws.ws_col as usize } else { env("COLUMNS", 80) };
        let rows = if ok && ws.ws_row > 0 { ws.ws_row as usize } else { env("LINES", 24) };
        (cols, rows)
    }

    fn read_stdin(&mut self, now: u64) -> bool {
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            return false;
        }
        let evs = self.buffer.process_bytes(&buf[..n as usize], now);
        self.route(evs, now);
        true
    }

    fn route(&mut self, evs: Vec<StdinEvent>, now: u64) {
        let mut input = Vec::new();
        let mut out = String::new();
        for e in evs {
            self.router.route(e, now, &mut input, &mut out);
        }
        if !out.is_empty() {
            self.raw_write(&out);
        }
        self.pending.extend(input.into_iter().map(TerminalEvent::Input));
    }

    fn next_deadline(&self) -> Option<u64> {
        [self.buffer.deadline(), self.router.deadline(), self.progress_next].into_iter().flatten().min()
    }

    fn run_timers(&mut self, now: u64) {
        if self.buffer.deadline().is_some_and(|d| now >= d) {
            let evs = self.buffer.tick(now);
            self.route(evs, now);
        }
        let mut input = Vec::new();
        self.router.tick(now, &mut input);
        self.pending.extend(input.into_iter().map(TerminalEvent::Input));
        if self.progress_next.is_some_and(|d| now >= d) {
            self.raw_write(PROGRESS_ACTIVE);
            self.progress_next = Some(now + PROGRESS_KEEPALIVE_MS);
        }
    }
}

#[cfg(unix)]
impl Terminal for ProcessTerminal {
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                self.saved = Some(t);
                let mut raw = t;
                libc::cfmakeraw(&mut raw);
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            }
        }
        self.raw_write("\x1b[?2004h");
        self.winch_fd = sig::install();
        self.pending.push_back(TerminalEvent::Resize);
        self.buffer = StdinBuffer::new(crate::stdin_buffer::DEFAULT_SEQUENCE_TIMEOUT_MS, resolve_escape_timeout_ms(&process_env));
        let ov = std::env::var("PI_PROGRAM_STATUS").ok();
        let w = self.router.begin(ov.as_deref());
        self.raw_write(&w);
    }

    fn stop(&mut self) {
        if !self.started {
            return;
        }
        self.started = false;
        let mut out = String::new();
        if self.progress_next.take().is_some() {
            out.push_str(PROGRESS_CLEAR);
        }
        if self.router.program_status_supported && self.router.program_status.is_some() {
            out.push_str(&format_program_status(&ProgramStatus::new("clear")));
        }
        self.router.program_status_supported = false;
        self.router.program_status_query_pending = false;
        out.push_str("\x1b[?2004l");
        self.router.disable_keyboard_protocols(&mut out);
        self.raw_write(&out);
        self.buffer.clear();
        if let Some(fd) = self.winch_fd.take() {
            sig::uninstall(fd);
        }
        unsafe {
            libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
            if let Some(t) = self.saved.take() {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }

    fn poll(&mut self, timeout_ms: Option<u64>) -> Vec<TerminalEvent> {
        let start = now_ms();
        loop {
            if !self.pending.is_empty() {
                return self.pending.drain(..).collect();
            }
            let now = now_ms();
            self.run_timers(now);
            if !self.pending.is_empty() {
                continue;
            }
            let mut wait = timeout_ms.map(|t| (start + t).saturating_sub(now));
            if let Some(d) = self.next_deadline() {
                let until = d.saturating_sub(now);
                wait = Some(wait.map_or(until, |w| w.min(until)));
            }
            if timeout_ms.is_some_and(|t| now >= start + t) {
                return Vec::new();
            }
            let mut fds = vec![libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 }];
            if let Some(w) = self.winch_fd {
                fds.push(libc::pollfd { fd: w, events: libc::POLLIN, revents: 0 });
            }
            let t = wait.map_or(-1, |w| w.min(i32::MAX as u64) as i32);
            let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, t) };
            if r < 0 {
                continue;
            }
            let now = now_ms();
            if fds.len() > 1 && fds[1].revents & libc::POLLIN != 0 {
                let mut b = [0u8; 64];
                while unsafe { libc::read(fds[1].fd, b.as_mut_ptr().cast(), b.len()) } > 0 {}
                self.pending.push_back(TerminalEvent::Resize);
            }
            if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                self.read_stdin(now);
            }
        }
    }

    fn drain_input(&mut self, max_ms: u64, idle_ms: u64) {
        let mut out = String::new();
        self.router.disable_keyboard_protocols(&mut out);
        self.raw_write(&out);
        let end = now_ms() + max_ms;
        let mut last = now_ms();
        loop {
            let now = now_ms();
            if now >= end || now - last >= idle_ms {
                break;
            }
            let mut fd = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
            let t = idle_ms.min(end - now) as i32;
            if unsafe { libc::poll(&mut fd, 1, t) } > 0 {
                let mut buf = [0u8; 4096];
                if unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) } > 0 {
                    last = now_ms();
                }
            }
        }
    }

    fn write(&mut self, data: &str) {
        self.raw_write(data);
        if let Some(p) = &self.log_path {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = f.write_all(data.as_bytes());
            }
        }
    }

    fn columns(&self) -> usize {
        self.size().0
    }

    fn rows(&self) -> usize {
        self.size().1
    }

    fn kitty_protocol_active(&self) -> bool {
        self.router.kitty_active
    }

    fn restore_on_exit(&mut self, before: &str, after: &str) {
        let mut s = before.to_string();
        if self.progress_next.is_some() {
            s.push_str(PROGRESS_CLEAR);
        }
        s.push_str("\x1b[?2004l");
        if self.router.keyboard_protocol_pushed || self.router.kitty_active {
            s.push_str("\x1b[<u");
        }
        if self.router.modify_other_keys_active {
            s.push_str("\x1b[>4;0m");
        }
        s.push_str(after);
        self.raw_write(&s);
        if let Some(t) = self.saved.take() {
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t) };
        }
    }

    fn set_progress(&mut self, active: bool) {
        if active {
            self.raw_write(PROGRESS_ACTIVE);
            if self.progress_next.is_none() {
                self.progress_next = Some(now_ms() + PROGRESS_KEEPALIVE_MS);
            }
        } else {
            self.progress_next = None;
            self.raw_write(PROGRESS_CLEAR);
        }
    }

    fn set_program_status(&mut self, status: ProgramStatus) {
        let supported = self.router.program_status_supported;
        let s = format_program_status(&status);
        self.router.program_status = (status.state != "clear").then_some(status);
        if supported {
            self.raw_write(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route_all(r: &mut InputRouter, seqs: &[&str]) -> (Vec<String>, String) {
        let mut input = Vec::new();
        let mut out = String::new();
        for s in seqs {
            r.route(StdinEvent::Data(s.to_string()), 0, &mut input, &mut out);
        }
        (input, out)
    }

    #[test]
    fn kitty_reply_enables_kitty() {
        let mut r = InputRouter::default();
        let q = r.begin(Some("0"));
        assert_eq!(q, "\x1b[>7u\x1b[?u\x1b[c");
        let (input, out) = route_all(&mut r, &["\x1b[?7u", "\x1b[?62;4c", "a"]);
        assert!(r.kitty_active);
        assert_eq!(out, "");
        assert_eq!(input, vec!["a"]);
        set_kitty_protocol_active(false);
    }

    #[test]
    fn device_attributes_alone_enable_modify_other_keys() {
        let mut r = InputRouter::default();
        r.begin(Some("0"));
        let (input, out) = route_all(&mut r, &["\x1b[?62;4c", "\x1b[?1;2c"]);
        assert!(!r.kitty_active);
        assert_eq!(out, "\x1b[>4;2m");
        // A second DA nobody asked for is input.
        assert_eq!(input, vec!["\x1b[?1;2c"]);
    }

    #[test]
    fn split_negotiation_fragment_is_reassembled_or_flushed() {
        let mut r = InputRouter::default();
        r.begin(Some("0"));
        let (input, _) = route_all(&mut r, &["\x1b[", "?", "1u"]);
        assert!(input.is_empty());
        assert!(r.kitty_active);
        set_kitty_protocol_active(false);
        let mut r = InputRouter::default();
        r.begin(Some("0"));
        let mut input = Vec::new();
        let mut out = String::new();
        r.route(StdinEvent::Data("\x1b[".into()), 5, &mut input, &mut out);
        r.tick(154, &mut input);
        assert!(input.is_empty());
        r.tick(155, &mut input);
        assert_eq!(input, vec!["\x1b["]);
    }

    #[test]
    fn program_status_reply_confirms_support() {
        let mut r = InputRouter::default();
        let q = r.begin(None);
        assert!(q.contains(PROGRAM_STATUS_QUERY));
        r.program_status = Some(ProgramStatus::new("working"));
        let (input, out) = route_all(&mut r, &["\x1b]7501;?\x1b\\"]);
        assert!(input.is_empty());
        assert_eq!(out, "\x1b]7501;state=working\x1b\\");
    }

    #[test]
    fn escape_timeout() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(resolve_escape_timeout_ms(&env(&[])), 10);
        assert_eq!(resolve_escape_timeout_ms(&env(&[("SSH_TTY", "/dev/pts/1")])), 100);
        assert_eq!(resolve_escape_timeout_ms(&env(&[("PI_TUI_ESC_TIMEOUT", "25"), ("SSH_TTY", "x")])), 25);
        assert_eq!(resolve_escape_timeout_ms(&env(&[("PI_TUI_ESC_TIMEOUT", "-1")])), 10);
    }
}
