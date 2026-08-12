//! Minimal JS object-literal parser for Pi's `models.generated.ts`.
//!
//! Pi's upstream model catalog source is a TypeScript module shaped like
//!
//! ```ts,ignore
//! export const MODELS = {
//!   "z-provider": { "z-model": { id: "z-model", cost: { input: 0, output: 1 }, /* ... */ } },
//!   /* ... */
//! } as const;
//! ```
//!
//! The values are a plain data literal (objects, arrays, strings, numbers,
//! booleans, `null`), so a small recursive-descent parser over the object/array
//! grammar is sufficient — no general JS engine is needed. This keeps the A.3
//! generator-scope port on exactly the sub-language Pi emits.

use std::collections::BTreeSet;

use serde_json::{Map, Number, Value};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("JS literal: {0}")]
    Message(String),
}

/// Find `export const NAME =` and return the trailing expression, optionally
/// ` as const`.
pub fn parse_exported_const(source: &str, name: &str) -> Result<Value, ParseError> {
    let needle = format!("export const {name} =");
    let pos = source
        .find(&needle)
        .ok_or_else(|| ParseError::Message(format!("missing `{needle}`")))?;
    let mut p = Parser::new(&source[pos + needle.len()..]);
    let value = p.expr()?;
    p.skip_ws();
    // Optional `as const` tail.
    if p.rest().starts_with("as const") {
        let _ = p.take("as const");
    }
    p.skip_ws();
    // Optional statement terminator.
    p.take(";");
    p.skip_ws();
    if !p.rest().is_empty() {
        return Err(ParseError::Message(format!(
            "unexpected trailing content: {:?}",
            &p.rest()[..p.rest().len().min(20)]
        )));
    }
    Ok(value)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser { src: s.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn rest(&self) -> &'a str {
        std::str::from_utf8(&self.src[self.pos..]).unwrap_or_default()
    }

    fn take(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), ParseError> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(ParseError::Message(format!("expected {:?} at {}", b as char, self.pos)))
        }
    }

    fn expr(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') | Some(b'\'') => self.string(),
            Some(b't') => {
                self.take("true");
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.take("false");
                Ok(Value::Bool(false))
            }
            Some(b'n') => {
                self.take("null");
                Ok(Value::Null)
            }
            Some(b'-') | Some(b'.') | Some(b'0'..=b'9') => self.number(),
            other => Err(ParseError::Message(format!(
                "unexpected byte {other:?} at {}",
                self.pos
            ))),
        }
    }

    fn object(&mut self) -> Result<Value, ParseError> {
        self.expect(b'{')?;
        let mut map = Map::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            // Key: identifier or quoted string.
            let key = match self.peek() {
                Some(b'"') | Some(b'\'') => self.string()?,
                Some(b'0'..=b'9') | Some(b'.') | Some(b'-') => {
                    // Number keys (rare); capture as-is via string.
                    let n = self.number()?;
                    Value::String(key_string(&n))
                }
                Some(b) if is_ident_start(b) => self.identifier()?,
                other => {
                    return Err(ParseError::Message(format!(
                        "bad object key {other:?} at {}",
                        self.pos
                    )))
                }
            };
            let key = key.as_str().unwrap_or_default().to_owned();
            if map.contains_key(&key) {
                return Err(ParseError::Message(format!("duplicate key {key:?}")));
            }
            self.skip_ws();
            self.expect(b':')?;
            let value = self.expr()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        break;
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                other => {
                    return Err(ParseError::Message(format!(
                        "expected , or }} got {other:?} at {}",
                        self.pos
                    )))
                }
            }
        }
        Ok(Value::Object(map))
    }

    fn array(&mut self) -> Result<Value, ParseError> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(out));
        }
        loop {
            let value = self.expr()?;
            out.push(value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        break;
                    }
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                other => {
                    return Err(ParseError::Message(format!(
                        "expected , or ] got {other:?} at {}",
                        self.pos
                    )))
                }
            }
        }
        Ok(Value::Array(out))
    }

    fn string(&mut self) -> Result<Value, ParseError> {
        let quote = self.peek().ok_or_else(|| ParseError::Message("unexpected eof".into()))?;
        self.pos += 1;
        let mut buf = Vec::new();
        loop {
            let b = self.peek().ok_or_else(|| ParseError::Message("unterminated string".into()))?;
            if b == quote {
                self.pos += 1;
                break;
            }
            if b == b'\\' {
                self.pos += 1;
                let esc = self.peek().ok_or_else(|| ParseError::Message("bad escape".into()))?;
                self.pos += 1;
                match esc {
                    b'n' => buf.push(b'\n'),
                    b't' => buf.push(b'\t'),
                    b'r' => buf.push(b'\r'),
                    b'b' => buf.push(8),
                    b'f' => buf.push(12),
                    b'\\' => buf.push(b'\\'),
                    b'"' => buf.push(b'"'),
                    b'\'' => buf.push(b'\''),
                    b'/' => buf.push(b'/'),
                    b'u' => {
                        if self.pos + 4 > self.src.len() {
                            return Err(ParseError::Message("bad \\u escape".into()));
                        }
                        let hex = &self.src[self.pos..self.pos + 4];
                        let text = std::str::from_utf8(hex).map_err(|_| {
                            ParseError::Message("bad \\u escape".into())
                        })?;
                        let code = u16::from_str_radix(text, 16)
                            .map_err(|_| ParseError::Message("bad \\u escape".into()))?;
                        self.pos += 4;
                        buf.extend(code.to_string().as_bytes());
                    }
                    other => return Err(ParseError::Message(format!("bad escape \\{}", other as char))),
                }
            } else {
                buf.push(b);
                self.pos += 1;
            }
        }
        String::from_utf8(buf)
            .map(Value::String)
            .map_err(|_| ParseError::Message("non-UTF8 string".into()))
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+' || b == b'e' || b == b'E'
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = String::from_utf8_lossy(&self.src[start..self.pos]);
        let text = text.trim().to_owned();
        if text.contains('.') || text.contains('e') || text.contains('E') {
            let f: f64 = text.parse().map_err(|_| {
                ParseError::Message(format!("bad number {text:?}"))
            })?;
            match Number::from_f64(f) {
                Some(n) => Ok(Value::Number(n)),
                None => Err(ParseError::Message(format!("bad number {text:?}"))),
            }
        } else {
            let i: i64 = text
                .parse()
                .map_err(|_| ParseError::Message(format!("bad integer {text:?}")))?;
            Ok(Value::Number(i.into()))
        }
    }

    fn identifier(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_ident_continue(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        // Identifiers used as object keys are strings.
        match text.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            "null" => Ok(Value::Null),
            _ => Ok(Value::String(text)),
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn key_string(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A helper used by the model catalog to allow-list known keys.
pub fn unknown_keys<'a>(map: &'a Map<String, Value>, accepted: &BTreeSet<&'a str>) -> Vec<&'a str> {
    map.keys()
        .filter(|k| !accepted.contains(k.as_str()))
        .map(|k| k.as_str())
        .collect()
}