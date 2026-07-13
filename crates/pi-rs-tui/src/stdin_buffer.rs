//! Protocol-aware buffering for terminal input.
//! Complete control sequences are emitted atomically; bracketed paste is emitted separately.

const ESC: u8 = 0x1b;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputLimits {
    pub max_pending_bytes: usize,
}

impl Default for InputLimits {
    fn default() -> Self {
        Self {
            max_pending_bytes: 1_048_576,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InputDecodeError {
    #[error("terminal input has {actual} pending bytes; limit is {limit}")]
    PendingBytes { actual: usize, limit: usize },
    #[error("terminal input is not valid UTF-8")]
    InvalidUtf8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StdinEvent {
    Data(String),
    Paste(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SequenceStatus {
    Complete,
    Incomplete,
}

/// Stateful stdin parser. `flush` is the deterministic equivalent of pi's
/// incomplete-sequence timeout.
#[derive(Debug)]
pub struct StdinBuffer {
    buffer: Vec<u8>,
    paste: bool,
    paste_buffer: Vec<u8>,
    pending_kitty_printable: Option<u32>,
    limits: InputLimits,
}

impl Default for StdinBuffer {
    fn default() -> Self {
        Self::with_limits(InputLimits::default())
    }
}

impl StdinBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: InputLimits) -> Self {
        Self {
            buffer: Vec::new(),
            paste: false,
            paste_buffer: Vec::new(),
            pending_kitty_printable: None,
            limits,
        }
    }

    pub fn process(&mut self, data: &str) -> Vec<StdinEvent> {
        self.process_bytes(data.as_bytes())
    }

    /// Compatibility entry point for event loops that cannot surface decoder
    /// errors. Malformed input is rejected and all partial state is discarded.
    /// A lone high byte retains the conventional meta-key conversion (`byte`
    /// becomes `ESC`, `byte - 128`).
    pub fn process_bytes(&mut self, data: &[u8]) -> Vec<StdinEvent> {
        let converted;
        let data = if data.len() == 1 && data[0] > 127 {
            converted = [ESC, data[0] - 128];
            &converted[..]
        } else {
            data
        };
        self.try_process_bytes(data).unwrap_or_default()
    }

    /// Process a raw byte batch atomically with explicit malformed/bound
    /// errors. Incomplete UTF-8 and terminal sequences remain buffered.
    pub fn try_process_bytes(&mut self, data: &[u8]) -> Result<Vec<StdinEvent>, InputDecodeError> {
        let actual = self
            .buffer
            .len()
            .saturating_add(self.paste_buffer.len())
            .saturating_add(data.len());
        if actual > self.limits.max_pending_bytes {
            self.clear();
            return Err(InputDecodeError::PendingBytes {
                actual,
                limit: self.limits.max_pending_bytes,
            });
        }
        if data.is_empty() && self.buffer.is_empty() {
            return Ok(vec![StdinEvent::Data(String::new())]);
        }
        self.buffer.extend_from_slice(data);
        let mut events = Vec::new();
        if let Err(error) = self.consume(&mut events) {
            self.clear();
            return Err(error);
        }
        Ok(events)
    }

    fn consume(&mut self, events: &mut Vec<StdinEvent>) -> Result<(), InputDecodeError> {
        if self.paste {
            self.paste_buffer.append(&mut self.buffer);
            self.finish_paste(events)?;
            return Ok(());
        }
        if let Some(start) = find_bytes(&self.buffer, PASTE_START) {
            let before = self.buffer[..start].to_vec();
            self.emit_complete(&before, events)?;
            self.pending_kitty_printable = None;
            self.paste_buffer = self.buffer[start + PASTE_START.len()..].to_vec();
            self.buffer.clear();
            self.paste = true;
            self.finish_paste(events)?;
            return Ok(());
        }
        let source = std::mem::take(&mut self.buffer);
        let consumed = self.emit_complete(&source, events)?;
        self.buffer.extend_from_slice(&source[consumed..]);
        Ok(())
    }

    fn finish_paste(&mut self, events: &mut Vec<StdinEvent>) -> Result<(), InputDecodeError> {
        let Some(end) = find_bytes(&self.paste_buffer, PASTE_END) else {
            return Ok(());
        };
        let content = std::str::from_utf8(&self.paste_buffer[..end])
            .map_err(|_| InputDecodeError::InvalidUtf8)?
            .to_owned();
        let remaining = self.paste_buffer[end + PASTE_END.len()..].to_vec();
        self.paste = false;
        self.paste_buffer.clear();
        self.pending_kitty_printable = None;
        events.push(StdinEvent::Paste(content));
        self.buffer = remaining;
        self.consume(events)
    }

    fn emit_complete(
        &mut self,
        source: &[u8],
        events: &mut Vec<StdinEvent>,
    ) -> Result<usize, InputDecodeError> {
        let mut pos = 0;
        while pos < source.len() {
            if source[pos] != ESC {
                let width = utf8_char_width(source[pos]).max(1);
                if width > source.len() - pos {
                    break;
                }
                self.emit_data(&source[pos..pos + width], events)?;
                pos += width;
                continue;
            }
            let remaining = &source[pos..];
            let mut end = 1;
            let mut found = false;
            while end <= remaining.len() {
                if sequence_status(&remaining[..end]) == SequenceStatus::Complete {
                    // WezTerm may concatenate raw Escape press and Kitty release.
                    if end == 2
                        && remaining[..2] == [ESC, ESC]
                        && remaining.get(2).is_some_and(|b| b"[]OP_".contains(b))
                    {
                        self.emit_data(&remaining[..1], events)?;
                        pos += 1;
                    } else {
                        self.emit_data(&remaining[..end], events)?;
                        pos += end;
                    }
                    found = true;
                    break;
                }
                end += 1;
            }
            if !found {
                break;
            }
        }
        Ok(pos)
    }

    fn emit_data(
        &mut self,
        bytes: &[u8],
        events: &mut Vec<StdinEvent>,
    ) -> Result<(), InputDecodeError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| InputDecodeError::InvalidUtf8)?
            .to_owned();
        let raw = if text.chars().count() == 1 {
            text.chars().next().map(u32::from)
        } else {
            None
        };
        if raw.is_some() && raw == self.pending_kitty_printable {
            self.pending_kitty_printable = None;
            return Ok(());
        }
        self.pending_kitty_printable = kitty_printable(&text);
        events.push(StdinEvent::Data(text));
        Ok(())
    }

    pub fn flush(&mut self) -> Vec<StdinEvent> {
        self.try_flush().unwrap_or_default()
    }

    pub fn try_flush(&mut self) -> Result<Vec<StdinEvent>, InputDecodeError> {
        if self.buffer.is_empty() {
            return Ok(Vec::new());
        }
        let result = std::str::from_utf8(&self.buffer)
            .map(str::to_owned)
            .map_err(|_| InputDecodeError::InvalidUtf8);
        self.buffer.clear();
        self.pending_kitty_printable = None;
        match result {
            Ok(data) => Ok(vec![StdinEvent::Data(data)]),
            Err(error) => {
                self.clear();
                Err(error)
            }
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.paste = false;
        self.paste_buffer.clear();
        self.pending_kitty_printable = None;
    }

    pub fn buffered(&self) -> &[u8] {
        &self.buffer
    }
}

fn sequence_status(data: &[u8]) -> SequenceStatus {
    if data.first() != Some(&ESC) || data.len() == 1 {
        return SequenceStatus::Incomplete;
    }
    match data[1] {
        b'[' => {
            if data.starts_with(b"\x1b[M") {
                return if data.len() >= 6 {
                    SequenceStatus::Complete
                } else {
                    SequenceStatus::Incomplete
                };
            }
            if data.len() < 3 {
                return SequenceStatus::Incomplete;
            }
            let payload = &data[2..];
            let last = payload[payload.len() - 1];
            if !(0x40..=0x7e).contains(&last) {
                return SequenceStatus::Incomplete;
            }
            if payload.first() == Some(&b'<') {
                let body = &payload[1..payload.len() - 1];
                let valid = matches!(last, b'M' | b'm')
                    && body.split(|b| *b == b';').count() == 3
                    && body
                        .split(|b| *b == b';')
                        .all(|p| !p.is_empty() && p.iter().all(u8::is_ascii_digit));
                if !valid {
                    return SequenceStatus::Incomplete;
                }
            }
            SequenceStatus::Complete
        }
        b']' => {
            if data.ends_with(b"\x1b\\") || data.ends_with(b"\x07") {
                SequenceStatus::Complete
            } else {
                SequenceStatus::Incomplete
            }
        }
        b'P' | b'_' => {
            if data.ends_with(b"\x1b\\") {
                SequenceStatus::Complete
            } else {
                SequenceStatus::Incomplete
            }
        }
        b'O' => {
            if data.len() >= 3 {
                SequenceStatus::Complete
            } else {
                SequenceStatus::Incomplete
            }
        }
        _ => SequenceStatus::Complete,
    }
}

fn kitty_printable(s: &str) -> Option<u32> {
    let body = s.strip_prefix("\x1b[")?.strip_suffix('u')?;
    let cp = body.split([':', ';']).next()?.parse().ok()?;
    (cp >= 32).then_some(cp)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn utf8_char_width(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn data(events: Vec<StdinEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|e| match e {
                StdinEvent::Data(s) => Some(s),
                StdinEvent::Paste(_) => None,
            })
            .collect()
    }

    #[test]
    fn buffers_every_protocol_family_and_mouse_forms() {
        for sequence in [
            "\x1b[1;5A",
            "\x1b]0;hi\x07",
            "\x1bP>|x\x1b\\",
            "\x1b_Gi=1;OK\x1b\\",
            "\x1bOP",
            "\x1b[<35;20;5m",
            "\x1b[Mabc",
        ] {
            let mut b = StdinBuffer::new();
            let mid = sequence.len() / 2;
            assert!(b.process(&sequence[..mid]).is_empty(), "{sequence:?}");
            assert_eq!(data(b.process(&sequence[mid..])), [sequence]);
        }
    }

    #[test]
    fn splits_batch_and_preserves_unicode() {
        let mut b = StdinBuffer::new();
        assert_eq!(data(b.process("é\x1b[A界")), ["é", "\x1b[A", "界"]);
    }

    #[test]
    fn paste_can_be_split_and_restarts_parser_after_end() {
        let mut b = StdinBuffer::new();
        assert_eq!(data(b.process("x\x1b[200~hello")), ["x"]);
        assert_eq!(
            b.process(" world\x1b[201~\x1b[A"),
            [
                StdinEvent::Paste("hello world".into()),
                StdinEvent::Data("\x1b[A".into())
            ]
        );
    }

    #[test]
    fn malformed_sgr_mouse_waits_then_flushes() {
        let mut b = StdinBuffer::new();
        assert!(b.process("\x1b[<1;2m").is_empty());
        assert_eq!(b.flush(), [StdinEvent::Data("\x1b[<1;2m".into())]);
    }

    #[test]
    fn suppresses_raw_printable_after_unmodified_kitty_sequence() {
        let mut b = StdinBuffer::new();
        assert_eq!(data(b.process("\x1b[97u")), ["\x1b[97u"]);
        assert!(b.process("a").is_empty());
        assert_eq!(data(b.process("b")), ["b"]);
        assert_eq!(data(b.process("\x1b[97;2uA")), ["\x1b[97;2u", "A"]);
    }

    #[test]
    fn wezterm_double_escape_restarts_sequence() {
        let mut b = StdinBuffer::new();
        assert_eq!(
            data(b.process("\x1b\x1b[27;1:3u")),
            ["\x1b", "\x1b[27;1:3u"]
        );
    }

    #[test]
    fn high_byte_and_empty_input_compatibility() {
        let mut b = StdinBuffer::new();
        assert_eq!(data(b.process_bytes(&[0xe1])), ["\x1ba"]);
        assert_eq!(data(b.process("")), [""]);
    }

    #[test]
    fn raw_decoder_buffers_split_utf8_and_rejects_malformed_data() {
        let mut b = StdinBuffer::new();
        assert!(b.try_process_bytes(&[0xc3]).unwrap().is_empty());
        assert_eq!(
            b.try_process_bytes(&[0xa9]).unwrap(),
            [StdinEvent::Data("é".into())]
        );
        assert_eq!(
            b.try_process_bytes(&[0xff, 0xff]),
            Err(InputDecodeError::InvalidUtf8)
        );
        assert!(b.buffered().is_empty());
        assert_eq!(
            b.try_process_bytes(b"ok").unwrap(),
            [StdinEvent::Data("o".into()), StdinEvent::Data("k".into())]
        );
    }

    #[test]
    fn pending_input_is_bounded_and_error_resets_partial_state() {
        let mut b = StdinBuffer::with_limits(InputLimits {
            max_pending_bytes: 8,
        });
        assert!(b.try_process_bytes(b"\x1b[200~abc").is_err());
        assert!(b.buffered().is_empty());
        assert_eq!(
            b.try_process_bytes(b"x").unwrap(),
            [StdinEvent::Data("x".into())]
        );
    }
}
