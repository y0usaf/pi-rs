//! Terminal protocol state, deterministic parsing, and process lifecycle helpers.
use crate::stdin_buffer::{StdinBuffer, StdinEvent};
use std::io::{self, Write};
use std::time::Duration;

pub const KITTY_FLAGS: u32 = 7;
pub const START_SEQUENCE: &str = "\x1b[?2004h\x1b[>7u\x1b[?u\x1b[c";
pub const STOP_SEQUENCE: &str = "\x1b[?2004l\x1b[<u";
pub const PROGRESS_ACTIVE_SEQUENCE: &str = "\x1b]9;4;3\x07";
pub const PROGRESS_CLEAR_SEQUENCE: &str = "\x1b]9;4;0;\x07";
pub const KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT: Duration = Duration::from_millis(150);
pub const STDIN_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(10);
const MODIFY_OTHER_KEYS_ENABLE: &str = "\x1b[>4;2m";
const MODIFY_OTHER_KEYS_DISABLE: &str = "\x1b[>4;0m";
const APPLE_TERMINAL_SHIFT_ENTER_SEQUENCE: &str = "\x1b[13;2u";

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("terminal I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("failed to enable terminal raw mode: {0}")]
    EnableRawMode(#[source] io::Error),
    #[error("failed to restore terminal raw mode: {0}")]
    RestoreRawMode(#[source] io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyboardNegotiation {
    KittyFlags(u32),
    DeviceAttributes,
}

pub fn parse_keyboard_negotiation(sequence: &str) -> Option<KeyboardNegotiation> {
    if let Some(flags) = sequence
        .strip_prefix("\x1b[?")
        .and_then(|s| s.strip_suffix('u'))
    {
        return flags.parse().ok().map(KeyboardNegotiation::KittyFlags);
    }
    let body = sequence.strip_prefix("\x1b[?")?.strip_suffix('c')?;
    if body.chars().all(|c| c.is_ascii_digit() || c == ';') {
        Some(KeyboardNegotiation::DeviceAttributes)
    } else {
        None
    }
}

fn is_keyboard_negotiation_prefix(sequence: &str) -> bool {
    if sequence == "\x1b[" {
        return true;
    }
    let Some(body) = sequence.strip_prefix("\x1b[?") else {
        return false;
    };
    body.chars().all(|c| c.is_ascii_digit() || c == ';')
}

/// Apple Terminal reports Shift+Enter as a plain carriage return. The caller
/// supplies platform/session and native modifier detection so this stays pure.
pub fn normalize_apple_terminal_input(
    data: &str,
    is_apple_terminal: bool,
    is_shift_pressed: bool,
) -> String {
    if is_apple_terminal && is_shift_pressed && data == "\r" {
        APPLE_TERMINAL_SHIFT_ENTER_SEQUENCE.to_owned()
    } else {
        data.to_owned()
    }
}

fn positive(value: Option<u16>) -> Option<u16> {
    value.filter(|value| *value > 0)
}

fn env_dimension(value: Option<&str>) -> Option<u16> {
    value
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
}

/// Resolve each dimension independently: explicit value, live terminal value,
/// environment fallback, then the portable 80x24 default.
pub fn resolve_terminal_dimensions(
    explicit: (Option<u16>, Option<u16>),
    live: (Option<u16>, Option<u16>),
    environment: (Option<&str>, Option<&str>),
) -> (u16, u16) {
    (
        positive(explicit.0)
            .or_else(|| positive(live.0))
            .or_else(|| env_dimension(environment.0))
            .unwrap_or(80),
        positive(explicit.1)
            .or_else(|| positive(live.1))
            .or_else(|| env_dimension(environment.1))
            .unwrap_or(24),
    )
}

fn environment_dimensions() -> (Option<String>, Option<String>) {
    (std::env::var("COLUMNS").ok(), std::env::var("LINES").ok())
}

/// Pure terminal state machine. A negotiation prefix has its own 150ms
/// deadline; callers retain the ordinary stdin parser's 10ms flush timeout.
#[derive(Debug)]
pub struct TerminalState {
    parser: StdinBuffer,
    negotiation_buffer: String,
    output: Vec<u8>,
    columns: u16,
    rows: u16,
    started: bool,
    draining: bool,
    protocol_pushed: bool,
    kitty_active: bool,
    modify_other_keys_active: bool,
    progress_active: bool,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl TerminalState {
    pub fn new(columns: Option<u16>, rows: Option<u16>) -> Self {
        let environment = environment_dimensions();
        let (columns, rows) = resolve_terminal_dimensions(
            (columns, rows),
            (None, None),
            (environment.0.as_deref(), environment.1.as_deref()),
        );
        Self {
            parser: StdinBuffer::new(),
            negotiation_buffer: String::new(),
            output: Vec::new(),
            columns,
            rows,
            started: false,
            draining: false,
            protocol_pushed: false,
            kitty_active: false,
            modify_other_keys_active: false,
            progress_active: false,
        }
    }

    pub fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.draining = false;
        self.protocol_pushed = true;
        self.output.extend_from_slice(START_SEQUENCE.as_bytes());
    }

    pub fn feed_input(&mut self, bytes: &[u8]) -> Vec<String> {
        let events = self.parser.process_bytes(bytes);
        self.process_events(events)
    }

    fn process_events(&mut self, events: Vec<StdinEvent>) -> Vec<String> {
        let mut input = Vec::new();
        for event in events {
            let sequence = match event {
                StdinEvent::Data(sequence) => sequence,
                StdinEvent::Paste(content) => format!("\x1b[200~{content}\x1b[201~"),
            };
            self.process_sequence(sequence, &mut input);
        }
        input
    }

    fn process_sequence(&mut self, sequence: String, input: &mut Vec<String>) {
        if !self.negotiation_buffer.is_empty() {
            let joined = format!("{}{sequence}", self.negotiation_buffer);
            if let Some(response) = parse_keyboard_negotiation(&joined) {
                self.negotiation_buffer.clear();
                self.handle_negotiation(response);
                return;
            }
            if is_keyboard_negotiation_prefix(&joined) {
                self.negotiation_buffer = joined;
                return;
            }
            input.push(std::mem::take(&mut self.negotiation_buffer));
        }
        if let Some(response) = parse_keyboard_negotiation(&sequence) {
            self.handle_negotiation(response);
        } else if is_keyboard_negotiation_prefix(&sequence) {
            self.negotiation_buffer = sequence;
        } else if !self.draining {
            input.push(sequence);
        }
    }

    /// Flush the ordinary 10ms stdin parser timeout. A possible protocol
    /// response prefix remains buffered for its separate 150ms deadline.
    pub fn flush_input(&mut self) -> Vec<String> {
        if self.draining {
            self.parser.flush();
            return Vec::new();
        }
        let events = self.parser.flush();
        self.process_events(events)
    }

    /// Flush a pending negotiation prefix when the 150ms deadline expires.
    pub fn flush_keyboard_negotiation(&mut self) -> Vec<String> {
        if self.draining || self.negotiation_buffer.is_empty() {
            self.negotiation_buffer.clear();
            return Vec::new();
        }
        vec![std::mem::take(&mut self.negotiation_buffer)]
    }

    pub fn keyboard_negotiation_pending(&self) -> bool {
        !self.negotiation_buffer.is_empty()
    }

    fn handle_negotiation(&mut self, response: KeyboardNegotiation) {
        match response {
            KeyboardNegotiation::KittyFlags(flags) if flags != 0 => {
                self.disable_modify_other_keys();
                self.kitty_active = true;
            }
            KeyboardNegotiation::KittyFlags(_) => self.enable_modify_other_keys(),
            KeyboardNegotiation::DeviceAttributes if !self.kitty_active => {
                self.enable_modify_other_keys()
            }
            KeyboardNegotiation::DeviceAttributes => {}
        }
    }
    fn enable_modify_other_keys(&mut self) {
        if self.kitty_active || self.modify_other_keys_active {
            return;
        }
        self.output
            .extend_from_slice(MODIFY_OTHER_KEYS_ENABLE.as_bytes());
        self.modify_other_keys_active = true;
    }
    fn disable_modify_other_keys(&mut self) {
        if !self.modify_other_keys_active {
            return;
        }
        self.output
            .extend_from_slice(MODIFY_OTHER_KEYS_DISABLE.as_bytes());
        self.modify_other_keys_active = false;
    }
    pub fn begin_drain(&mut self) {
        self.disable_keyboard_protocol();
        self.draining = true;
        self.negotiation_buffer.clear();
    }
    fn disable_keyboard_protocol(&mut self) {
        if self.protocol_pushed || self.kitty_active {
            self.output.extend_from_slice(b"\x1b[<u");
            self.protocol_pushed = false;
            self.kitty_active = false;
        }
        self.disable_modify_other_keys();
    }
    pub fn stop(&mut self) {
        if self.progress_active {
            self.output
                .extend_from_slice(PROGRESS_CLEAR_SEQUENCE.as_bytes());
            self.progress_active = false;
        }
        if self.started {
            self.output.extend_from_slice(b"\x1b[?2004l");
        }
        self.disable_keyboard_protocol();
        self.parser.clear();
        self.negotiation_buffer.clear();
        self.started = false;
        self.draining = false;
    }
    pub fn write(&mut self, data: &str) {
        self.output.extend_from_slice(data.as_bytes());
    }
    pub fn columns(&self) -> u16 {
        self.columns
    }
    pub fn rows(&self) -> u16 {
        self.rows
    }
    pub fn kitty_protocol_active(&self) -> bool {
        self.kitty_active
    }
    pub fn modify_other_keys_active(&self) -> bool {
        self.modify_other_keys_active
    }
    pub fn resize(&mut self, columns: Option<u16>, rows: Option<u16>) {
        let environment = environment_dimensions();
        (self.columns, self.rows) = resolve_terminal_dimensions(
            (columns, rows),
            (None, None),
            (environment.0.as_deref(), environment.1.as_deref()),
        );
    }
    pub fn move_by(&mut self, lines: i32) {
        if lines > 0 {
            self.write(&format!("\x1b[{lines}B"));
        } else if lines < 0 {
            self.write(&format!("\x1b[{}A", lines.unsigned_abs()));
        }
    }
    pub fn hide_cursor(&mut self) {
        self.write("\x1b[?25l");
    }
    pub fn show_cursor(&mut self) {
        self.write("\x1b[?25h");
    }
    pub fn clear_line(&mut self) {
        self.write("\x1b[K");
    }
    pub fn clear_from_cursor(&mut self) {
        self.write("\x1b[J");
    }
    pub fn clear_screen(&mut self) {
        self.write("\x1b[2J\x1b[H");
    }
    pub fn set_title(&mut self, title: &str) {
        self.write(&format!("\x1b]0;{title}\x07"));
    }
    pub fn set_progress(&mut self, active: bool) {
        self.write(if active {
            PROGRESS_ACTIVE_SEQUENCE
        } else {
            PROGRESS_CLEAR_SEQUENCE
        });
        self.progress_active = active;
    }
    pub fn progress_keepalive(&mut self) {
        if self.progress_active {
            self.write(PROGRESS_ACTIVE_SEQUENCE);
        }
    }
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }
}

/// Generic output adapter only; it deliberately does not claim process raw-mode ownership.
pub struct ProcessTerminal<W: Write = io::Stdout> {
    state: TerminalState,
    writer: W,
}
impl ProcessTerminal<io::Stdout> {
    pub fn stdout(columns: Option<u16>, rows: Option<u16>) -> Self {
        let live = crossterm::terminal::size().ok();
        let environment = environment_dimensions();
        let dimensions = resolve_terminal_dimensions(
            (columns, rows),
            (live.map(|size| size.0), live.map(|size| size.1)),
            (environment.0.as_deref(), environment.1.as_deref()),
        );
        Self::new(io::stdout(), Some(dimensions.0), Some(dimensions.1))
    }
}
impl<W: Write> ProcessTerminal<W> {
    pub fn new(writer: W, columns: Option<u16>, rows: Option<u16>) -> Self {
        Self {
            state: TerminalState::new(columns, rows),
            writer,
        }
    }
    pub fn state(&self) -> &TerminalState {
        &self.state
    }
    pub fn state_mut(&mut self) -> &mut TerminalState {
        &mut self.state
    }
    pub fn start(&mut self) -> Result<(), TerminalError> {
        self.state.start();
        self.flush_output()
    }
    pub fn stop(&mut self) -> Result<(), TerminalError> {
        self.state.stop();
        self.flush_output()
    }
    pub fn feed_input(&mut self, bytes: &[u8]) -> Result<Vec<String>, TerminalError> {
        let events = self.state.feed_input(bytes);
        self.flush_output()?;
        Ok(events)
    }
    pub fn flush_output(&mut self) -> Result<(), TerminalError> {
        let bytes = self.state.take_output();
        self.writer.write_all(&bytes)?;
        self.writer.flush()?;
        Ok(())
    }
    pub fn into_writer(self) -> W {
        self.writer
    }
}

/// Owns the actual process TTY lifecycle. Construct this only for an
/// interactive process terminal, alongside (not inside) a generic adapter.
pub struct ProcessRawModeGuard {
    was_raw: bool,
    active: bool,
    protocol_output: bool,
}
impl ProcessRawModeGuard {
    /// Enter raw mode and own protocol setup/cleanup output.
    pub fn start() -> Result<Self, TerminalError> {
        let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
        if !was_raw {
            crossterm::terminal::enable_raw_mode().map_err(TerminalError::EnableRawMode)?;
        }
        let mut guard = Self {
            was_raw,
            active: true,
            protocol_output: true,
        };
        if let Err(error) = guard.write_sequence(START_SEQUENCE) {
            let _ = guard.restore();
            return Err(error);
        }
        guard.refresh_unix_dimensions();
        Ok(guard)
    }

    /// Enter raw mode while another terminal adapter owns protocol bytes.
    /// This is used by the integrated process driver so setup and cleanup are
    /// emitted exactly once through its deterministic `TerminalState`.
    pub fn start_raw_only() -> Result<Self, TerminalError> {
        let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
        if !was_raw {
            crossterm::terminal::enable_raw_mode().map_err(TerminalError::EnableRawMode)?;
        }
        let guard = Self {
            was_raw,
            active: true,
            protocol_output: false,
        };
        guard.refresh_unix_dimensions();
        Ok(guard)
    }
    fn write_sequence(&mut self, sequence: &str) -> Result<(), TerminalError> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(sequence.as_bytes())?;
        stdout.flush()?;
        Ok(())
    }
    #[cfg(unix)]
    fn refresh_unix_dimensions(&self) {
        unsafe {
            libc::raise(libc::SIGWINCH);
        }
    }
    #[cfg(not(unix))]
    fn refresh_unix_dimensions(&self) {}
    pub fn restore(&mut self) -> Result<(), TerminalError> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let output_result = if self.protocol_output {
            self.write_sequence(STOP_SEQUENCE)
        } else {
            Ok(())
        };
        let raw_result = if self.was_raw {
            Ok(())
        } else {
            crossterm::terminal::disable_raw_mode().map_err(TerminalError::RestoreRawMode)
        };
        output_result.and(raw_result)
    }
}
impl Drop for ProcessRawModeGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    fn out(t: &mut TerminalState) -> String {
        String::from_utf8(t.take_output()).unwrap()
    }
    #[test]
    fn start_and_kitty_negotiation_are_exact() {
        let mut t = TerminalState::default();
        t.start();
        assert_eq!(out(&mut t), START_SEQUENCE);
        assert!(t.feed_input(b"\x1b[?7u").is_empty());
        assert!(t.kitty_protocol_active());
    }
    #[test]
    fn da_or_zero_flags_enable_fallback_once() {
        for response in [b"\x1b[?1;2c".as_slice(), b"\x1b[?0u"] {
            let mut t = TerminalState::default();
            t.start();
            out(&mut t);
            t.feed_input(response);
            t.feed_input(b"\x1b[?1c");
            assert_eq!(out(&mut t), MODIFY_OTHER_KEYS_ENABLE);
        }
    }
    #[test]
    fn kitty_after_fallback_disables_fallback() {
        let mut t = TerminalState::default();
        t.start();
        out(&mut t);
        t.feed_input(b"\x1b[?1c");
        t.feed_input(b"\x1b[?7u");
        assert_eq!(out(&mut t), "\x1b[>4;2m\x1b[>4;0m");
    }
    #[test]
    fn parser_input_and_paste_are_forwarded() {
        let mut t = TerminalState::default();
        assert_eq!(
            t.feed_input(b"a\x1b[A\x1b[200~x\n\x1b[201~"),
            ["a", "\x1b[A", "\x1b[200~x\n\x1b[201~"]
        );
    }
    #[test]
    fn negotiation_prefix_uses_separate_deadline_flush() {
        let mut t = TerminalState::default();
        assert!(t.feed_input(b"\x1b[").is_empty());
        assert!(t.flush_input().is_empty());
        assert!(t.keyboard_negotiation_pending());
        assert_eq!(t.flush_keyboard_negotiation(), ["\x1b["]);
        assert_eq!(
            KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT,
            Duration::from_millis(150)
        );
        assert_eq!(STDIN_SEQUENCE_TIMEOUT, Duration::from_millis(10));
    }
    #[test]
    fn split_negotiation_is_consumed_after_parser_flush() {
        let mut t = TerminalState::default();
        t.feed_input(b"\x1b[");
        t.flush_input();
        assert!(t.feed_input(b"?7u").is_empty());
        assert!(t.kitty_protocol_active());
    }
    #[test]
    fn mismatched_negotiation_prefix_preserves_both_inputs() {
        let mut t = TerminalState::default();
        t.feed_input(b"\x1b[");
        t.flush_input();
        assert_eq!(t.feed_input(b"A"), ["\x1b[", "A"]);
    }
    #[test]
    fn apple_terminal_normalization_is_pure() {
        assert_eq!(
            normalize_apple_terminal_input("\r", true, true),
            "\x1b[13;2u"
        );
        assert_eq!(normalize_apple_terminal_input("\r", true, false), "\r");
        assert_eq!(normalize_apple_terminal_input("x", true, true), "x");
    }
    #[test]
    fn dimensions_follow_explicit_live_environment_default_order() {
        assert_eq!(
            resolve_terminal_dimensions(
                (Some(100), None),
                (Some(90), Some(30)),
                (Some("70"), Some("20"))
            ),
            (100, 30)
        );
        assert_eq!(
            resolve_terminal_dimensions((None, None), (None, None), (Some("120"), Some("40"))),
            (120, 40)
        );
        assert_eq!(
            resolve_terminal_dimensions((Some(0), None), (Some(0), None), (Some("bad"), Some("0"))),
            (80, 24)
        );
    }
    #[test]
    fn drain_and_stop_bytes_are_exact() {
        let mut t = TerminalState::default();
        t.start();
        out(&mut t);
        t.begin_drain();
        assert_eq!(out(&mut t), "\x1b[<u");
        t.stop();
        assert_eq!(out(&mut t), "\x1b[?2004l");
    }
    #[test]
    fn operations_and_progress_bytes_are_exact() {
        let mut t = TerminalState::default();
        t.move_by(-2);
        t.move_by(3);
        t.hide_cursor();
        t.show_cursor();
        t.clear_line();
        t.clear_from_cursor();
        t.clear_screen();
        t.set_title("pi-rs");
        t.set_progress(true);
        t.progress_keepalive();
        t.set_progress(false);
        assert_eq!(
            out(&mut t),
            "\x1b[2A\x1b[3B\x1b[?25l\x1b[?25h\x1b[K\x1b[J\x1b[2J\x1b[H\x1b]0;pi-rs\x07\x1b]9;4;3\x07\x1b]9;4;3\x07\x1b]9;4;0;\x07"
        );
    }
    #[test]
    fn process_adapter_writes_state_output_without_raw_mode_claim() {
        let mut t = ProcessTerminal::new(Vec::new(), Some(80), Some(24));
        t.start().unwrap();
        t.state_mut().hide_cursor();
        t.flush_output().unwrap();
        assert_eq!(
            t.into_writer(),
            format!("{START_SEQUENCE}\x1b[?25l").as_bytes()
        );
    }
}
