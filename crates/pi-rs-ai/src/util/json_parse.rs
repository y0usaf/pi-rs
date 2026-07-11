//! Port of `utils/json-parse.ts` — JSON repair plus streaming (partial)
//! JSON parsing for tool-call arguments.
//!
//! The spec's `parseStreamingJson` falls back to the `partial-json` npm
//! package; here that fallback is [`parse_partial_json`], a hand-rolled
//! equivalent of that package's default `Allow.ALL` mode: partial
//! strings, numbers, literals, arrays and objects are completed with
//! what has arrived so far, and incomplete trailing members of
//! containers are dropped. `NaN`/`Infinity` are unrepresentable in
//! `serde_json` and parse as malformed — they do not occur in streamed
//! tool arguments.
//!
//! Divergences (mechanism, pinned by tests):
//! - JS strings tolerate unpaired UTF-16 surrogates; Rust strings do
//!   not. Unpaired surrogate escapes in partial strings are dropped —
//!   exactly what the spec's `sanitizeSurrogates` would later do.
//! - Raw control characters inside partial strings are accepted
//!   directly (the spec reaches the same value one fallback later, via
//!   `repairJson` + `partial-json`).

use serde_json::{Map, Value};

const VALID_JSON_ESCAPES: &[char] = &['"', '\\', '/', 'b', 'f', 'n', 'r', 't', 'u'];

fn escape_control_character(c: char) -> String {
    match c {
        '\u{8}' => "\\b".to_string(),
        '\u{c}' => "\\f".to_string(),
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        _ => format!("\\u{:04x}", c as u32),
    }
}

/// Spec: `repairJson` — repairs malformed JSON string literals by
/// escaping raw control characters inside strings and doubling
/// backslashes before invalid escape characters.
pub fn repair_json(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut repaired = String::with_capacity(json.len());
    let mut in_string = false;
    let mut index = 0;

    while index < chars.len() {
        let c = chars[index];

        if !in_string {
            repaired.push(c);
            if c == '"' {
                in_string = true;
            }
            index += 1;
            continue;
        }

        if c == '"' {
            repaired.push(c);
            in_string = false;
            index += 1;
            continue;
        }

        if c == '\\' {
            let Some(&next) = chars.get(index + 1) else {
                repaired.push_str("\\\\");
                index += 1;
                continue;
            };

            if next == 'u' {
                let digits: String = chars[index + 2..].iter().take(4).collect();
                if digits.len() == 4 && digits.chars().all(|d| d.is_ascii_hexdigit()) {
                    repaired.push_str("\\u");
                    repaired.push_str(&digits);
                    index += 6;
                    continue;
                }
            }

            if VALID_JSON_ESCAPES.contains(&next) {
                repaired.push('\\');
                repaired.push(next);
                index += 2;
                continue;
            }

            repaired.push_str("\\\\");
            index += 1;
            continue;
        }

        if (c as u32) <= 0x1f {
            repaired.push_str(&escape_control_character(c));
        } else {
            repaired.push(c);
        }
        index += 1;
    }

    repaired
}

/// Spec: `parseJsonWithRepair` — strict parse, then parse the repaired
/// text if repair changed anything, else the original error.
pub fn parse_json_with_repair(json: &str) -> Result<Value, serde_json::Error> {
    match serde_json::from_str(json) {
        Ok(value) => Ok(value),
        Err(error) => {
            let repaired = repair_json(json);
            if repaired != json {
                serde_json::from_str(&repaired)
            } else {
                Err(error)
            }
        }
    }
}

/// Failure of [`parse_partial_json`].
#[derive(Debug, thiserror::Error)]
pub enum PartialJsonError {
    /// Structurally invalid input (not merely truncated).
    #[error("malformed partial JSON at offset {0}")]
    Malformed(usize),
    /// Truncated input with no completable value (e.g. a bare `-`).
    #[error("incomplete partial JSON")]
    Incomplete,
}

/// Parse potentially truncated JSON, completing whatever has arrived
/// (the `partial-json` package's `Allow.ALL` behavior).
pub fn parse_partial_json(input: &str) -> Result<Value, PartialJsonError> {
    let chars: Vec<char> = input.chars().collect();
    let mut parser = Parser {
        chars: &chars,
        pos: 0,
    };
    parser.skip_ws();
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos < parser.chars.len() {
        return Err(PartialJsonError::Malformed(parser.pos));
    }
    Ok(value)
}

/// Spec: `parseStreamingJson` — always yields a value; empty/unparseable
/// input becomes `{}`. (The spec types the result as a record; a valid
/// non-object JSON document is returned as-is there, and here.)
pub fn parse_streaming_json(partial_json: &str) -> Value {
    if partial_json.trim().is_empty() {
        return Value::Object(Map::new());
    }
    if let Ok(value) = parse_json_with_repair(partial_json) {
        return value;
    }
    if let Ok(value) = parse_partial_json(partial_json) {
        return value;
    }
    if let Ok(value) = parse_partial_json(&repair_json(partial_json)) {
        return value;
    }
    Value::Object(Map::new())
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Value, PartialJsonError> {
        match self.peek() {
            None => Err(PartialJsonError::Incomplete),
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Value::String(self.parse_string().0)),
            Some('t') => self.parse_literal("true", Value::Bool(true)),
            Some('f') => self.parse_literal("false", Value::Bool(false)),
            Some('n') => self.parse_literal("null", Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(_) => Err(PartialJsonError::Malformed(self.pos)),
        }
    }

    fn parse_literal(&mut self, literal: &str, value: Value) -> Result<Value, PartialJsonError> {
        let available = self.chars.len() - self.pos;
        let taken: String = self.chars[self.pos..].iter().take(literal.len()).collect();
        if taken == literal {
            self.pos += literal.len();
            return Ok(value);
        }
        // Truncated prefix at end of input completes to the literal.
        if available < literal.len() && literal.starts_with(&taken) {
            self.pos = self.chars.len();
            return Ok(value);
        }
        Err(PartialJsonError::Malformed(self.pos))
    }

    /// Returns the decoded string and whether it was terminated by a
    /// closing quote (`false` = truncated and completed).
    fn parse_string(&mut self) -> (String, bool) {
        self.pos += 1; // opening quote
        let mut out = String::new();
        // Unpaired surrogates are dropped wherever `pending_high` is
        // cleared without pairing (see module docs).
        let mut pending_high: Option<u16> = None;

        loop {
            let Some(c) = self.peek() else {
                return (out, false);
            };
            if c == '"' {
                self.pos += 1;
                return (out, true);
            }
            if c != '\\' {
                pending_high = None;
                out.push(c);
                self.pos += 1;
                continue;
            }

            // Escape sequence.
            let Some(&next) = self.chars.get(self.pos + 1) else {
                // Trailing backslash: drop the partial escape.
                self.pos += 1;
                return (out, false);
            };
            if next == 'u' {
                let digits: String = self.chars[self.pos + 2..].iter().take(4).collect();
                if digits.len() < 4 {
                    // Unicode escape truncated by the stream: drop it.
                    self.pos = self.chars.len();
                    return (out, false);
                }
                if !digits.chars().all(|d| d.is_ascii_hexdigit()) {
                    // Invalid escape: skip the backslash, keep the rest
                    // as literal characters.
                    pending_high = None;
                    self.pos += 1;
                    continue;
                }
                let unit = u32::from_str_radix(&digits, 16).unwrap_or(0) as u16;
                self.pos += 6;
                if let Some(high) = pending_high.take()
                    && (0xDC00..=0xDFFF).contains(&unit)
                {
                    let combined =
                        0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00);
                    if let Some(c) = char::from_u32(combined) {
                        out.push(c);
                    }
                    continue;
                }
                // An unconsumed pending high surrogate is dropped here.
                if (0xD800..=0xDBFF).contains(&unit) {
                    pending_high = Some(unit);
                } else if (0xDC00..=0xDFFF).contains(&unit) {
                    // Unpaired low surrogate: dropped.
                } else if let Some(c) = char::from_u32(u32::from(unit)) {
                    out.push(c);
                }
                continue;
            }
            pending_high = None;
            let decoded = match next {
                '"' => Some('"'),
                '\\' => Some('\\'),
                '/' => Some('/'),
                'b' => Some('\u{8}'),
                'f' => Some('\u{c}'),
                'n' => Some('\n'),
                'r' => Some('\r'),
                't' => Some('\t'),
                other => Some(other),
            };
            if let Some(c) = decoded {
                out.push(c);
            }
            self.pos += 2;
        }
    }

    fn parse_number(&mut self) -> Result<Value, PartialJsonError> {
        let start = self.pos;
        while matches!(self.peek(), Some('0'..='9' | '-' | '+' | '.' | 'e' | 'E')) {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let at_end = self.pos >= self.chars.len();
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            return Ok(value);
        }
        if at_end {
            // Truncated number: longest valid prefix.
            for len in (1..text.len()).rev() {
                if let Ok(value) = serde_json::from_str::<Value>(&text[..len]) {
                    return Ok(value);
                }
            }
            return Err(PartialJsonError::Incomplete);
        }
        Err(PartialJsonError::Malformed(start))
    }

    fn parse_object(&mut self) -> Result<Value, PartialJsonError> {
        self.pos += 1; // '{'
        let mut map = Map::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Ok(Value::Object(map)),
                Some('}') => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                Some('"') => {}
                Some(_) => return Err(PartialJsonError::Malformed(self.pos)),
            }
            let (key, key_complete) = self.parse_string();
            if !key_complete {
                // Truncated key: drop the pair, close the object.
                return Ok(Value::Object(map));
            }
            self.skip_ws();
            match self.peek() {
                None => return Ok(Value::Object(map)),
                Some(':') => self.pos += 1,
                Some(_) => return Err(PartialJsonError::Malformed(self.pos)),
            }
            self.skip_ws();
            match self.parse_value() {
                Ok(value) => {
                    map.insert(key, value);
                }
                Err(PartialJsonError::Incomplete) => return Ok(Value::Object(map)),
                Err(error) => return Err(error),
            }
            self.skip_ws();
            match self.peek() {
                None => return Ok(Value::Object(map)),
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                Some(_) => return Err(PartialJsonError::Malformed(self.pos)),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Value, PartialJsonError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Ok(Value::Array(items)),
                Some(']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                Some(_) => {}
            }
            match self.parse_value() {
                Ok(value) => items.push(value),
                Err(PartialJsonError::Incomplete) => return Ok(Value::Array(items)),
                Err(error) => return Err(error),
            }
            self.skip_ws();
            match self.peek() {
                None => return Ok(Value::Array(items)),
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                Some(_) => return Err(PartialJsonError::Malformed(self.pos)),
            }
        }
    }
}
