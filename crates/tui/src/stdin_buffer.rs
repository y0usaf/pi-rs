//! Buffers raw stdin and splits it into complete key/mouse/reply sequences and bracketed pastes
//! (upstream `stdin-buffer.ts`).
//!
//! Upstream emits events and arms `setTimeout`s. Here there is no runtime: [`StdinBuffer::process`]
//! returns the events, [`StdinBuffer::deadline`] says when a flush is due, and the caller calls
//! [`StdinBuffer::tick`] once that time has passed. Times are milliseconds on any monotonic clock.
//!
//! Upstream splits plain text by UTF-16 code unit, so an astral character arrives as two lone
//! surrogate events; here it arrives as one event.

const ESC: &str = "\x1b";
pub const DEFAULT_SEQUENCE_TIMEOUT_MS: u64 = 50;
pub const DEFAULT_ESCAPE_TIMEOUT_MS: u64 = 10;
const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// What the buffer produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdinEvent {
    /// One complete sequence or character.
    Data(String),
    /// The content of a bracketed paste, without its markers.
    Paste(String),
}

/// Upstream `TERMINAL_REPLY_HEADER_PATTERN`:
/// `^\x1b(?:\](?:1[01];|4;\d{1,3};|7501;\?)|P>\||_G(?:[a-z]=\d+,)*[a-z]=\d+;)`.
fn has_terminal_reply_header(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(ESC) else {
        return false;
    };
    if let Some(osc) = rest.strip_prefix(']') {
        if osc.starts_with("10;") || osc.starts_with("11;") || osc.starts_with("7501;?") {
            return true;
        }
        if let Some(p) = osc.strip_prefix("4;") {
            let n = p.bytes().take_while(u8::is_ascii_digit).count();
            return (1..=3).contains(&n) && p.as_bytes().get(n) == Some(&b';');
        }
        return false;
    }
    if rest.starts_with("P>|") {
        return true;
    }
    if let Some(g) = rest.strip_prefix("_G") {
        let b = g.as_bytes();
        let mut i = 0;
        loop {
            if !(i < b.len() && b[i].is_ascii_lowercase()) {
                return false;
            }
            i += 1;
            if b.get(i) != Some(&b'=') {
                return false;
            }
            i += 1;
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return false;
            }
            match b.get(i) {
                Some(b';') => return true,
                Some(b',') => i += 1,
                _ => return false,
            }
        }
    }
    false
}

fn is_control_string(s: &str) -> bool {
    s.starts_with("\x1b]") || s.starts_with("\x1bP") || s.starts_with("\x1b_")
}

#[derive(PartialEq)]
enum Status {
    Complete,
    Incomplete,
    NotEscape,
}

fn is_complete_sequence(data: &str) -> Status {
    let Some(after) = data.strip_prefix(ESC) else {
        return Status::NotEscape;
    };
    if after.is_empty() {
        return Status::Incomplete;
    }
    let done = |b: bool| {
        if b {
            Status::Complete
        } else {
            Status::Incomplete
        }
    };
    if after.starts_with('[') {
        if after.starts_with("[M") {
            return done(data.chars().count() >= 6);
        }
        return is_complete_csi(data);
    }
    if after.starts_with(']') {
        return done(data.ends_with("\x1b\\") || data.ends_with('\x07'));
    }
    if after.starts_with('P') || after.starts_with('_') {
        return done(data.ends_with("\x1b\\"));
    }
    if after.starts_with('O') {
        return done(after.chars().count() >= 2);
    }
    Status::Complete
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_complete_csi(data: &str) -> Status {
    if data.chars().count() < 3 {
        return Status::Incomplete;
    }
    let payload = &data[2..];
    let last = payload.chars().last().unwrap();
    if !('\x40'..='\x7e').contains(&last) {
        return Status::Incomplete;
    }
    if let Some(mouse) = payload.strip_prefix('<') {
        if last == 'M' || last == 'm' {
            let parts: Vec<&str> = mouse[..mouse.len() - 1].split(';').collect();
            if parts.len() == 3 && parts.iter().all(|p| all_digits(p)) {
                return Status::Complete;
            }
        }
        return Status::Incomplete;
    }
    Status::Complete
}

/// `^\x1b\[(\d+)(?::\d*)?(?::\d+)?u$`, codepoint when >= 32.
fn unmodified_kitty_printable(seq: &str) -> Option<u32> {
    let body = seq.strip_prefix("\x1b[")?.strip_suffix('u')?;
    let mut parts = body.split(':');
    let cp = parts.next()?;
    if !all_digits(cp) {
        return None;
    }
    let p2 = parts.next();
    let p3 = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if let Some(p) = p2
        && !p.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    if let Some(p) = p3
        && !all_digits(p)
    {
        return None;
    }
    let cp: u64 = cp.parse().unwrap_or(u64::MAX);
    (cp >= 32).then(|| cp.min(u32::MAX as u64) as u32)
}

fn extract_complete_sequences(buffer: &str) -> (Vec<String>, String) {
    let mut seqs = Vec::new();
    let mut pos = 0;
    while pos < buffer.len() {
        let remaining = &buffer[pos..];
        if !remaining.starts_with(ESC) {
            let c = remaining.chars().next().unwrap();
            seqs.push(c.to_string());
            pos += c.len_utf8();
            continue;
        }
        let reply = is_control_string(remaining) && has_terminal_reply_header(remaining);
        let bounds: Vec<usize> = remaining
            .char_indices()
            .map(|(i, _)| i)
            .skip(1)
            .chain([remaining.len()])
            .collect();
        let mut k = 0;
        let mut finished = false;
        while k < bounds.len() {
            let end = bounds[k];
            if reply && k + 1 > 2 {
                let prev_start = if k == 0 { 0 } else { bounds[k - 1] };
                let ch = &remaining[prev_start..end];
                if ch == "\x18" || ch == "\x1a" {
                    pos += end;
                    finished = true;
                    break;
                }
                if ch == ESC && end < remaining.len() && !remaining[end..].starts_with('\\') {
                    pos += prev_start;
                    finished = true;
                    break;
                }
            }
            let candidate = &remaining[..end];
            match is_complete_sequence(candidate) {
                Status::Complete => {
                    if candidate == "\x1b\x1b" {
                        let next = remaining[end..].chars().next();
                        if matches!(next, Some('[' | ']' | 'O' | 'P' | '_')) {
                            seqs.push(ESC.to_string());
                            pos += 1;
                            finished = true;
                            break;
                        }
                    }
                    seqs.push(candidate.to_string());
                    pos += end;
                    finished = true;
                    break;
                }
                Status::Incomplete => k += 1,
                Status::NotEscape => {
                    seqs.push(candidate.to_string());
                    pos += end;
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            return (seqs, remaining.to_string());
        }
    }
    (seqs, String::new())
}

/// Splits stdin chunks into complete sequences. See the module docs for timing.
#[derive(Debug)]
pub struct StdinBuffer {
    buffer: String,
    deadline: Option<u64>,
    timeout_ms: u64,
    escape_timeout_ms: u64,
    paste_mode: bool,
    paste_buffer: String,
    pending_kitty_printable: Option<u32>,
}

impl Default for StdinBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_SEQUENCE_TIMEOUT_MS, DEFAULT_ESCAPE_TIMEOUT_MS)
    }
}

impl StdinBuffer {
    /// `timeout_ms` bounds the wait for an incomplete sequence; `escape_timeout_ms` the wait after
    /// a lone ESC before it counts as Escape.
    pub fn new(timeout_ms: u64, escape_timeout_ms: u64) -> Self {
        StdinBuffer {
            buffer: String::new(),
            deadline: None,
            timeout_ms,
            escape_timeout_ms,
            paste_mode: false,
            paste_buffer: String::new(),
            pending_kitty_printable: None,
        }
    }

    /// Feed raw bytes. A single byte above 127 is the legacy meta form: ESC + (byte - 128).
    pub fn process_bytes(&mut self, data: &[u8], now: u64) -> Vec<StdinEvent> {
        if data.len() == 1 && data[0] > 127 {
            let s = format!("\x1b{}", char::from(data[0] - 128));
            return self.process(&s, now);
        }
        self.process(&String::from_utf8_lossy(data), now)
    }

    /// Feed text received at time `now`.
    pub fn process(&mut self, data: &str, now: u64) -> Vec<StdinEvent> {
        let mut out = Vec::new();
        self.process_into(data, now, &mut out);
        out
    }

    fn process_into(&mut self, data: &str, now: u64, out: &mut Vec<StdinEvent>) {
        self.deadline = None;
        if data.is_empty() && self.buffer.is_empty() {
            self.emit_data(String::new(), out);
            return;
        }
        self.buffer.push_str(data);
        if self.paste_mode {
            let b = std::mem::take(&mut self.buffer);
            self.paste_buffer.push_str(&b);
            self.finish_paste(now, out);
            return;
        }
        if let Some(start) = self.buffer.find(PASTE_START) {
            if start > 0 {
                let (seqs, _) = extract_complete_sequences(&self.buffer[..start]);
                for s in seqs {
                    self.emit_data(s, out);
                }
            }
            self.pending_kitty_printable = None;
            self.paste_buffer = self.buffer[start + PASTE_START.len()..].to_string();
            self.buffer.clear();
            self.paste_mode = true;
            self.finish_paste(now, out);
            return;
        }
        let (seqs, rest) = extract_complete_sequences(&self.buffer);
        self.buffer = rest;
        for s in seqs {
            self.emit_data(s, out);
        }
        if !self.buffer.is_empty() {
            if has_terminal_reply_header(&self.buffer) {
                return;
            }
            let wait = if self.buffer == ESC {
                self.escape_timeout_ms
            } else {
                self.timeout_ms
            };
            self.deadline = Some(now + wait);
        }
    }

    fn finish_paste(&mut self, now: u64, out: &mut Vec<StdinEvent>) {
        if let Some(end) = self.paste_buffer.find(PASTE_END) {
            let content = self.paste_buffer[..end].to_string();
            let remaining = self.paste_buffer[end + PASTE_END.len()..].to_string();
            self.paste_mode = false;
            self.paste_buffer.clear();
            self.pending_kitty_printable = None;
            out.push(StdinEvent::Paste(content));
            if !remaining.is_empty() {
                self.process_into(&remaining, now, out);
            }
        }
    }

    /// When the pending partial sequence should be flushed, if one is pending.
    pub fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    /// Flush a pending partial sequence once its deadline has passed.
    pub fn tick(&mut self, now: u64) -> Vec<StdinEvent> {
        let mut out = Vec::new();
        if self.deadline.is_some_and(|d| now >= d) {
            for seq in self.flush() {
                if seq.chars().count() > 2 && is_control_string(&seq) {
                    let split = seq.char_indices().nth(2).map_or(seq.len(), |(i, _)| i);
                    self.emit_data(seq[..split].to_string(), &mut out);
                    self.process_into(&seq[split..], now, &mut out);
                } else {
                    self.emit_data(seq, &mut out);
                }
            }
        }
        out
    }

    fn emit_data(&mut self, seq: String, out: &mut Vec<StdinEvent>) {
        let mut it = seq.chars();
        let raw = match (it.next(), it.next()) {
            (Some(c), None) => Some(c as u32),
            _ => None,
        };
        if raw.is_some() && raw == self.pending_kitty_printable {
            self.pending_kitty_printable = None;
            return;
        }
        self.pending_kitty_printable = unmodified_kitty_printable(&seq);
        out.push(StdinEvent::Data(seq));
    }

    /// Take whatever is buffered, unparsed.
    pub fn flush(&mut self) -> Vec<String> {
        self.deadline = None;
        if self.buffer.is_empty() {
            return Vec::new();
        }
        self.pending_kitty_printable = None;
        vec![std::mem::take(&mut self.buffer)]
    }

    /// Drop all state, including an unfinished paste.
    pub fn clear(&mut self) {
        self.deadline = None;
        self.buffer.clear();
        self.paste_mode = false;
        self.paste_buffer.clear();
        self.pending_kitty_printable = None;
    }

    /// The unparsed buffered text.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }
}
