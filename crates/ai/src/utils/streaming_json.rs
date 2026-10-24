//! Incremental, tolerant parsing of JSON text that arrives in pieces, such as streamed tool-call
//! arguments (utils/streaming-json.ts).
//!
//! StreamingJsonParser::append reads each piece once, so a stream costs time and memory linear in its
//! length, and snapshot() returns the value of the text read so far. Snapshots are plain JSON values
//! and never change once returned: the parser copies the containers that are still open (the "open
//! path") into every new snapshot and shares completed containers. The rules for incomplete text
//! follow parse_streaming_json():
//!
//! - An unterminated string shows the characters read so far; an unterminated escape is held back
//!   until complete.
//! - An unterminated container shows the members and elements read so far.
//! - A member whose key or value has not started is omitted.
//! - An unterminated literal shows its value once it is a prefix of true, false or null; an
//!   unterminated number shows its value when the digits read so far form a number (an exponent
//!   without digits is ignored).
//!
//! Malformed text is read leniently, the way repair_json() and parse_streaming_json() treat it: raw
//! control characters in strings are kept, an invalid escape stays literal text, missing commas and
//! colons are tolerated, values in invalid positions and unknown words are dropped, and text after
//! the root value is ignored. The parser never fails. Callers that need the exact value of the
//! finished text parse it once, e.g. with parse_streaming_json().
//!
//! Differences from parse_streaming_json() on incomplete text: trailing whitespace inside an
//! unterminated string is kept (it was trimmed), a string that contains a raw control character or an
//! invalid escape stays visible while it streams (it was dropped until the document was complete), and
//! NaN and Infinity words are dropped. A lone surrogate escape, which a Rust String cannot hold,
//! shows as U+FFFD.
//!
//! append() also returns the changes the piece made, for a writer that keeps its own copy of the
//! value and applies them instead of re-assigning snapshots: Set when a key, an array item or a
//! finished value appears (a new string is set once with its first text, a container empty), Append
//! when the piece extended an open string. Unterminated numbers and literals are not reported until
//! they end. A container nested more than 32 levels deep is reported as one Set of its whole value
//! when it closes, so paths stay short.
//!
//! Cost: copying the open path costs its member and element count. Each append() earns copy credit
//! proportional to its length plus a constant; snapshot() spends it and returns the previous snapshot
//! while the open path costs more than the credit. So a document that keeps a very large container
//! open (thousands of elements) shows a preview that lags a little behind the text, and no input makes
//! the total work grow faster than the text.

use serde_json::{Map, Number, Value};

use crate::json_parse::parse_streaming_json;

type JsonObject = Map<String, Value>;

/// One step of a path from the root value: a member key or an array index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonPathSegment {
    Key(String),
    Index(usize),
}

/// A path from the root value.
pub type JsonPath = Vec<JsonPathSegment>;

/// One change a piece of text made to the value; see the module comment.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonChange {
    Set { path: JsonPath, value: Value },
    Append { path: JsonPath, text: String },
}

/// Where a value sits in its parent; absent for a value in an invalid position, which is read but
/// dropped.
#[derive(Debug, Clone)]
enum Slot {
    Root,
    Member(String),
    Element(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Key,
    Colon,
    Value,
    Comma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringTarget {
    Key,
    Value,
}

/// An open object or array. Its container is never handed out while open; snapshots get copies.
struct Frame {
    container: Value,
    slot: Option<Slot>,
    /// Where the container sits, when its changes are reported.
    path: Option<JsonPath>,
    /// Whether changes inside it are reported; when not, its whole value is reported as it closes.
    reports_inside: bool,
    /// Objects: the key read last, where the member value goes.
    key: Option<String>,
    phase: Phase,
}

/// An unterminated string.
struct OpenString {
    target: StringTarget,
    slot: Option<Slot>,
    text: String,
    /// Escape read so far: empty for none, otherwise a backslash and up to five more characters.
    escape: String,
    /// A high surrogate escape whose low half has not arrived.
    pending_high: Option<u32>,
    /// Where a value string sits, when its changes are reported.
    path: Option<JsonPath>,
    /// The change this string extends, while it is the newest change of the piece.
    change: Option<usize>,
}

// Changes are reported at most this deep, so a path costs O(1) to build.
const MAX_REPORTED_DEPTH: usize = 32;

// Copy credit per append and per appended character. The floor keeps small documents fresh at every
// snapshot.
const CREDIT_PER_APPEND: usize = 256;
const CREDIT_PER_CHARACTER: usize = 8;
// An unterminated number longer than this is not shown until it ends, so snapshot() stays cheap.
const MAX_PENDING_SCALAR_LENGTH: usize = 64;

const LITERALS: [&str; 3] = ["true", "false", "null"];

fn literal_value(literal: &str) -> Value {
    match literal {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::Null,
    }
}

const REPLACEMENT_CHARACTER: char = '\u{fffd}';

/// Incremental, tolerant JSON parser; see the module comment for its rules and costs.
#[derive(Default)]
pub struct StreamingJsonParser {
    root: Option<Value>,
    done: bool,
    stack: Vec<Frame>,
    string: Option<OpenString>,
    scalar: String,
    scalar_slot: Option<Slot>,
    /// Bytes a fresh snapshot of the open path copies, kept current as the text grows.
    path_bytes: usize,
    credit: usize,
    dirty: bool,
    snapshot: Option<Value>,
    /// Changes of the piece being read.
    changes: Vec<JsonChange>,
}

impl StreamingJsonParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the next piece of text; returns the changes it made, in order. Costs time linear in the
    /// piece's length.
    pub fn append(&mut self, text: &str) -> Vec<JsonChange> {
        self.credit += CREDIT_PER_APPEND + CREDIT_PER_CHARACTER * text.chars().count();
        self.changes.clear();
        if let Some(open) = self.string.as_mut() {
            open.change = None;
        }
        let mut index = 0;
        while index < text.len() && !self.done {
            if self.string.is_some() {
                index = self.read_string(text, index);
            } else if !self.scalar.is_empty() {
                index = self.read_scalar(text, index);
            } else {
                let character = text[index..].chars().next().unwrap_or_default();
                index += character.len_utf8();
                self.structural(character);
            }
        }
        std::mem::take(&mut self.changes)
    }

    /// The value of the text read so far: None before a value starts, otherwise a plain JSON value
    /// that never changes afterwards. Returns the previous snapshot while the text read since did not
    /// change the value, or while a fresh copy of the open path is not yet paid for.
    pub fn snapshot(&mut self) -> Option<&Value> {
        if self.dirty {
            let pending = self.pending_scalar();
            let cost = self.snapshot_cost() + usize::from(pending.is_some());
            if cost <= self.credit {
                self.credit -= cost;
                self.dirty = false;
                self.snapshot = self.copy_open_path(pending);
            }
        }
        self.snapshot.as_ref()
    }

    /// Read string characters from start until the string ends or the text does; returns the next
    /// index.
    fn read_string(&mut self, text: &str, start: usize) -> usize {
        let Some(mut open) = self.string.take() else {
            return text.len();
        };
        let mut run_start = start;
        let mut index = start;
        while index < text.len() {
            let character = text[index..].chars().next().unwrap_or_default();
            let width = character.len_utf8();
            if !open.escape.is_empty() {
                if self.continue_escape(&mut open, character) {
                    run_start = index + width;
                    index += width;
                    continue;
                }
                // Not part of the escape: read it as a normal string character below.
                run_start = index;
            }
            if character == '"' {
                if index > run_start {
                    let piece = text[run_start..index].to_owned();
                    self.append_string(&mut open, &piece);
                }
                self.end_string(open);
                return index + 1;
            }
            if character == '\\' {
                if index > run_start {
                    let piece = text[run_start..index].to_owned();
                    self.append_string(&mut open, &piece);
                }
                open.escape = "\\".to_owned();
                run_start = index + 1;
            }
            index += width;
        }
        if text.len() > run_start {
            let piece = text[run_start..].to_owned();
            self.append_string(&mut open, &piece);
        }
        self.string = Some(open);
        text.len()
    }

    /// Read number or literal characters from start up to the next delimiter; returns the delimiter's
    /// index.
    fn read_scalar(&mut self, text: &str, start: usize) -> usize {
        let mut end = start;
        for (offset, character) in text[start..].char_indices() {
            if is_delimiter(character) {
                end = start + offset;
                break;
            }
            end = start + offset + character.len_utf8();
        }
        if end > start {
            self.scalar.push_str(&text[start..end]);
            if self.scalar_slot.is_some() {
                self.dirty = true;
            }
        }
        if end < text.len() {
            self.end_scalar();
        }
        end
    }

    /// Handle one character outside strings.
    fn structural(&mut self, character: char) {
        match character {
            ' ' | '\t' | '\n' | '\r' => {}
            '"' => self.start_string(),
            '{' => self.open_container(Value::Object(JsonObject::new())),
            '[' => self.open_container(Value::Array(Vec::new())),
            '}' | ']' => self.close_container(),
            ':' => {
                if let Some(frame) = self.stack.last_mut()
                    && matches!(frame.container, Value::Object(_))
                    && frame.phase == Phase::Colon
                {
                    frame.phase = Phase::Value;
                }
            }
            ',' => {
                if let Some(frame) = self.stack.last_mut() {
                    frame.phase = if matches!(frame.container, Value::Array(_)) {
                        Phase::Value
                    } else {
                        Phase::Key
                    };
                }
            }
            other => {
                self.scalar.clear();
                self.scalar.push(other);
                self.scalar_slot = self.value_slot();
                if self.scalar_slot.is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    /// Where a value starting now goes, or None when this position takes no value.
    fn value_slot(&self) -> Option<Slot> {
        let Some(frame) = self.stack.last() else {
            return self.root.is_none().then_some(Slot::Root);
        };
        match &frame.container {
            Value::Array(items) => Some(Slot::Element(items.len())),
            // A missing colon is tolerated: {"a" 1} reads as {"a": 1}.
            Value::Object(_) => match (&frame.phase, &frame.key) {
                (Phase::Value | Phase::Colon, Some(key)) => Some(Slot::Member(key.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    fn start_string(&mut self) {
        // A missing comma is tolerated: {"a": 1 "b": 2} reads the second member.
        let is_key = self.stack.last().is_some_and(|frame| {
            matches!(frame.container, Value::Object(_))
                && matches!(frame.phase, Phase::Key | Phase::Comma)
        });
        if is_key {
            self.string = Some(OpenString {
                target: StringTarget::Key,
                slot: None,
                text: String::new(),
                escape: String::new(),
                pending_high: None,
                path: None,
                change: None,
            });
            return;
        }
        let slot = self.value_slot();
        let path = self.reported_path(slot.as_ref());
        let change = path.as_ref().map(|path| {
            self.report(JsonChange::Set {
                path: path.clone(),
                value: Value::String(String::new()),
            })
        });
        self.place(slot.as_ref(), Value::String(String::new()));
        self.string = Some(OpenString {
            target: StringTarget::Value,
            slot,
            text: String::new(),
            escape: String::new(),
            pending_high: None,
            path,
            change,
        });
    }

    /// The path of a value at slot in the innermost open container, when its changes are reported.
    fn reported_path(&self, slot: Option<&Slot>) -> Option<JsonPath> {
        let slot = slot?;
        if matches!(slot, Slot::Root) {
            return Some(Vec::new());
        }
        let frame = self.stack.last()?;
        if !frame.reports_inside {
            return None;
        }
        let mut path = frame.path.clone()?;
        match slot {
            Slot::Member(key) => path.push(JsonPathSegment::Key(key.clone())),
            Slot::Element(index) => path.push(JsonPathSegment::Index(*index)),
            Slot::Root => {}
        }
        Some(path)
    }

    fn report(&mut self, change: JsonChange) -> usize {
        self.changes.push(change);
        self.changes.len() - 1
    }

    /// Continue an escape with character; returns false when it is not part of the escape and must be
    /// read normally.
    fn continue_escape(&mut self, open: &mut OpenString, character: char) -> bool {
        if open.escape.len() == 1 {
            if let Some(decoded) = simple_escape(character) {
                open.escape.clear();
                self.append_string(open, &decoded.to_string());
                return true;
            }
            if character == 'u' {
                open.escape.push('u');
                return true;
            }
            // Invalid escape: keep the backslash as text and read the character normally, as
            // repair_json() does.
            open.escape.clear();
            self.append_string(open, "\\");
            return false;
        }
        if character.is_ascii_hexdigit() {
            open.escape.push(character);
            if open.escape.len() < 6 {
                return true;
            }
            let unit = u32::from_str_radix(&open.escape[2..], 16).unwrap_or(0);
            open.escape.clear();
            self.append_code_unit(open, unit);
            return true;
        }
        // A backslash-u without four hex digits: keep it as text and read the character normally.
        let literal = std::mem::take(&mut open.escape);
        self.append_string(open, &literal);
        false
    }

    /// Append one UTF-16 code unit decoded from a backslash-u escape, joining surrogate halves.
    fn append_code_unit(&mut self, open: &mut OpenString, unit: u32) {
        if let Some(high) = open.pending_high.take() {
            if (0xdc00..=0xdfff).contains(&unit) {
                let scalar = 0x10000 + ((high - 0xd800) << 10) + (unit - 0xdc00);
                let decoded = char::from_u32(scalar).unwrap_or(REPLACEMENT_CHARACTER);
                self.append_string(open, &decoded.to_string());
                return;
            }
            self.append_string(open, &REPLACEMENT_CHARACTER.to_string());
        }
        if (0xd800..=0xdbff).contains(&unit) {
            open.pending_high = Some(unit);
            return;
        }
        let decoded = char::from_u32(unit).unwrap_or(REPLACEMENT_CHARACTER);
        self.append_string(open, &decoded.to_string());
    }

    fn append_string(&mut self, open: &mut OpenString, text: &str) {
        if open.pending_high.take().is_some() {
            self.append_decoded(open, &REPLACEMENT_CHARACTER.to_string());
        }
        self.append_decoded(open, text);
    }

    fn append_decoded(&mut self, open: &mut OpenString, text: &str) {
        if text.is_empty() {
            return;
        }
        if open.target == StringTarget::Key {
            open.text.push_str(text);
            return;
        }
        // A value string grows in its container, so a delta never copies the text read so far.
        self.path_bytes += text.len();
        self.push_text(open.slot.as_ref(), text);
        if let Some(path) = open.path.clone() {
            self.extend_change(open, &path, text);
        }
    }

    /// Append text to the string at `slot`; the string itself while it is open, so the cost of a
    /// delta is the delta's length.
    fn push_text(&mut self, slot: Option<&Slot>, text: &str) {
        let Some(slot) = slot else {
            return;
        };
        self.dirty = true;
        let Some(frame) = self.stack.last_mut() else {
            match &mut self.root {
                Some(Value::String(existing)) => existing.push_str(text),
                _ => self.root = Some(Value::String(text.to_owned())),
            }
            return;
        };
        let target = match (&mut frame.container, slot) {
            (Value::Array(items), Slot::Element(index)) => items.get_mut(*index),
            (Value::Object(members), Slot::Member(key)) => members.get_mut(key),
            _ => None,
        };
        match target {
            Some(Value::String(existing)) => existing.push_str(text),
            Some(other) => *other = Value::String(text.to_owned()),
            None => {}
        }
    }

    /// Extend the newest change of the piece while nothing came after it; otherwise report an append.
    fn extend_change(&mut self, open: &mut OpenString, path: &JsonPath, text: &str) {
        if let Some(index) = open.change
            && index + 1 == self.changes.len()
        {
            match &mut self.changes[index] {
                JsonChange::Set { value, .. } => {
                    if let Value::String(existing) = value {
                        existing.push_str(text);
                    }
                }
                JsonChange::Append { text: existing, .. } => existing.push_str(text),
            }
            return;
        }
        open.change = Some(self.report(JsonChange::Append {
            path: path.clone(),
            text: text.to_owned(),
        }));
    }

    fn end_string(&mut self, mut open: OpenString) {
        self.string = None;
        // A high surrogate whose low half never arrived cannot live in a Rust String.
        if open.pending_high.take().is_some() {
            self.append_decoded(&mut open, &REPLACEMENT_CHARACTER.to_string());
        }
        if open.target == StringTarget::Key {
            if let Some(frame) = self.stack.last_mut() {
                frame.key = Some(open.text);
                frame.phase = Phase::Colon;
            }
            return;
        }
        self.value_ended();
    }

    fn end_scalar(&mut self) {
        let token = std::mem::take(&mut self.scalar);
        let slot = self.scalar_slot.take();
        if let Some(value) = scalar_value(&token) {
            self.place(slot.as_ref(), value.clone());
            if let Some(path) = self.reported_path(slot.as_ref()) {
                self.report(JsonChange::Set { path, value });
            }
        }
        self.value_ended();
    }

    /// The value of the unterminated scalar at the end of the text, if it shows.
    fn pending_scalar(&self) -> Option<(Slot, Value)> {
        let slot = self.scalar_slot.as_ref()?;
        let token = self.scalar.as_str();
        if token.is_empty() || token.chars().count() > MAX_PENDING_SCALAR_LENGTH {
            return None;
        }
        let value = pending_scalar_value(token)?;
        Some((slot.clone(), value))
    }

    fn open_container(&mut self, container: Value) {
        let container_value = container.clone();
        let slot = self.value_slot();
        let path = self.reported_path(slot.as_ref());
        self.place(slot.as_ref(), container.clone());
        let phase = if matches!(container_value, Value::Array(_)) {
            Phase::Value
        } else {
            Phase::Key
        };
        let reports_inside = path
            .as_ref()
            .is_some_and(|path| path.len() < MAX_REPORTED_DEPTH);
        self.stack.push(Frame {
            container,
            slot,
            path: path.clone(),
            reports_inside,
            key: None,
            phase,
        });
        self.path_bytes += 1;
        self.dirty = true;
        // A fresh container: the working one keeps changing, and a change never does.
        if let Some(path) = path {
            let empty = if matches!(container_value, Value::Array(_)) {
                Value::Array(Vec::new())
            } else {
                Value::Object(JsonObject::new())
            };
            self.report(JsonChange::Set { path, value: empty });
        }
    }

    fn close_container(&mut self) {
        let Some(frame) = self.stack.pop() else {
            // A stray closer outside any container is ignored.
            return;
        };
        let Frame {
            container,
            slot,
            path,
            reports_inside,
            ..
        } = frame;
        // The frame's own contents leave the open path; placing the finished container into its parent
        // puts them back there, so only the frame itself is gone.
        self.path_bytes = self
            .path_bytes
            .saturating_sub(1 + value_cost(Some(&container), 0));
        self.dirty = true;
        // Too deep to report inside: the whole value, final now that it is closed.
        if let Some(path) = path
            && !reports_inside
        {
            self.report(JsonChange::Set {
                path,
                value: container.clone(),
            });
        }
        // The container owns its value while open; a copy of it sits in the parent, so the finished
        // value takes its place there.
        self.place(slot.as_ref(), container);
        self.value_ended();
    }

    /// A value in the innermost open frame, or the root value when there is none, has ended.
    fn value_ended(&mut self) {
        match self.stack.last_mut() {
            // Text after the root value is ignored, but only once a root value exists.
            None => {
                if self.root.is_some() {
                    self.done = true;
                }
            }
            Some(frame) => frame.phase = Phase::Comma,
        }
    }

    /// Set the value at slot in the open path; nothing for a value in an invalid position.
    fn place(&mut self, slot: Option<&Slot>, value: Value) {
        let Some(slot) = slot else {
            return;
        };
        self.dirty = true;
        let cost = value_cost(Some(&value), 0);
        match slot {
            Slot::Root => self.root = Some(value),
            Slot::Element(index) => {
                let index = *index;
                let Some(frame) = self.stack.last_mut() else {
                    return;
                };
                if let Value::Array(items) = &mut frame.container {
                    if index == items.len() {
                        self.path_bytes += cost;
                        items.push(value);
                    } else if let Some(existing) = items.get_mut(index) {
                        let previous = value_cost(Some(existing), 0);
                        *existing = value;
                        self.path_bytes = self.path_bytes + cost - previous.min(self.path_bytes);
                    }
                }
            }
            Slot::Member(key) => {
                let Some(frame) = self.stack.last_mut() else {
                    return;
                };
                if let Value::Object(members) = &mut frame.container {
                    if members.contains_key(key) {
                        let previous = value_cost(members.get(key), 0);
                        self.path_bytes = self.path_bytes + cost - previous.min(self.path_bytes);
                    } else {
                        self.path_bytes += cost;
                    }
                    members.insert(key.clone(), value);
                }
            }
        }
    }

    /// What a fresh snapshot copies: the open containers and the values they hold, in bytes.
    fn snapshot_cost(&self) -> usize {
        if self.stack.is_empty() {
            value_cost(self.root.as_ref(), 0)
        } else {
            self.path_bytes
        }
    }

    /// A fresh snapshot: copies of the open containers, innermost first, each holding the copy of its
    /// open child.
    fn copy_open_path(&self, pending: Option<(Slot, Value)>) -> Option<Value> {
        if self.stack.is_empty() {
            return match pending {
                Some((Slot::Root, value)) => Some(value),
                _ => self.root.clone(),
            };
        }
        let mut child: Option<Value> = None;
        let mut child_slot: Option<Slot> = None;
        for depth in (0..self.stack.len()).rev() {
            let frame = &self.stack[depth];
            let mut copy = frame.container.clone();
            if depth + 1 == self.stack.len()
                && let Some((slot, value)) = &pending
            {
                set_slot(&mut copy, slot, value.clone());
            }
            if let (Some(child_value), Some(slot)) = (&child, &child_slot) {
                set_slot(&mut copy, slot, child_value.clone());
            }
            child = Some(copy);
            child_slot = frame.slot.clone();
        }
        match child_slot {
            Some(Slot::Root) => child,
            _ => self.root.clone(),
        }
    }
}

/// The arguments of one streaming tool call: the raw text, for the one full parse when the call ends,
/// and a StreamingJsonParser for the partial arguments in between. Providers keep one per call in
/// their own stream state, never on the content block, so the raw text never reaches partial messages
/// or transcripts.
#[derive(Default)]
pub struct ToolCallArgumentsStream {
    text: String,
    parser: StreamingJsonParser,
    snapshot: Option<Value>,
    arguments: JsonObject,
}

impl ToolCallArgumentsStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// The raw argument text received so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Append a piece of argument text; returns the partial arguments, unchanged when they did not
    /// change.
    pub fn append(&mut self, delta: &str) -> JsonObject {
        self.text.push_str(delta);
        self.parser.append(delta);
        self.partial()
    }

    /// Take the provider's complete argument text. When it continues the text so far, returns the
    /// missing piece as delta; otherwise starts over with text. Either way returns the partial
    /// arguments of text.
    pub fn replace(&mut self, text: &str) -> Replaced {
        if let Some(delta) = text.strip_prefix(self.text.as_str()) {
            let delta = delta.to_owned();
            let arguments = if delta.is_empty() {
                self.partial()
            } else {
                self.append(&delta)
            };
            return Replaced {
                delta: (!delta.is_empty()).then_some(delta),
                arguments,
            };
        }
        self.text.clear();
        self.parser = StreamingJsonParser::new();
        self.snapshot = None;
        self.arguments = JsonObject::new();
        let arguments = self.append(text);
        Replaced {
            delta: None,
            arguments,
        }
    }

    /// The final arguments: one full parse of the complete text. pi-rs carries no argumentsError, so
    /// malformed arguments read as none.
    pub fn finish(&self) -> JsonObject {
        match parse_streaming_json(Some(&self.text)) {
            Value::Object(members) => members,
            _ => JsonObject::new(),
        }
    }

    fn partial(&mut self) -> JsonObject {
        if self.parser.snapshot() == self.snapshot.as_ref() {
            return self.arguments.clone();
        }
        // Tool arguments are an object; any other partial value shows as no arguments yet.
        self.arguments = match self.parser.snapshot() {
            Some(Value::Object(members)) => members.clone(),
            _ => JsonObject::new(),
        };
        self.snapshot = self.parser.snapshot().cloned();
        self.arguments.clone()
    }
}

/// What ToolCallArgumentsStream::replace did.
pub struct Replaced {
    pub delta: Option<String>,
    pub arguments: JsonObject,
}

fn simple_escape(character: char) -> Option<char> {
    Some(match character {
        '"' => '"',
        '\\' => '\\',
        '/' => '/',
        'b' => '\u{08}',
        'f' => '\u{0c}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        _ => return None,
    })
}

fn is_delimiter(character: char) -> bool {
    matches!(
        character,
        ' ' | '\t' | '\n' | '\r' | ',' | ':' | '{' | '}' | '[' | ']' | '"'
    )
}

/// How much of the text a number at its start covers: 0 when there is none.
fn number_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut index = 0;
    if bytes.first() == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(byte) if byte.is_ascii_digit() => {
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return 0,
    }
    if bytes.get(index) == Some(&b'.') && bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        let mut after = index + 1;
        if matches!(bytes.get(after), Some(b'+' | b'-')) {
            after += 1;
        }
        let digits = after;
        while bytes.get(after).is_some_and(u8::is_ascii_digit) {
            after += 1;
        }
        if after > digits {
            index = after;
        }
    }
    index
}

fn is_number(token: &str) -> bool {
    !token.is_empty() && number_len(token) == token.len()
}

/// The mantissa of a token cut after its exponent marker, such as -1.25e+.
fn number_without_exponent_digits(token: &str) -> Option<&str> {
    let mantissa = &token[..number_len(token)];
    let rest = token[mantissa.len()..].strip_prefix(['e', 'E'])?;
    if rest.is_empty() || rest == "+" || rest == "-" {
        return is_number(mantissa).then_some(mantissa);
    }
    None
}

fn json_number(token: &str) -> Option<Value> {
    // The crate's own number parse, so a value read here equals the same text parsed whole.
    let number: Number = serde_json::from_str(token).ok()?;
    Some(Value::Number(number))
}

/// The value of a complete number or literal token; None for anything else, which is dropped.
fn scalar_value(token: &str) -> Option<Value> {
    if let Some(literal) = LITERALS.iter().find(|literal| **literal == token) {
        return Some(literal_value(literal));
    }
    if is_number(token) {
        return json_number(token);
    }
    None
}

/// The value an unterminated number or literal token shows.
fn pending_scalar_value(token: &str) -> Option<Value> {
    if let Some(literal) = LITERALS.iter().find(|literal| literal.starts_with(token)) {
        return Some(literal_value(literal));
    }
    if is_number(token) {
        return json_number(token);
    }
    number_without_exponent_digits(token).and_then(json_number)
}

/// How many bytes a copy of `value` holds; `depth` stops the walk in very deep values.
fn value_cost(value: Option<&Value>, depth: usize) -> usize {
    match value {
        None => 0,
        Some(Value::String(text)) => text.len(),
        Some(Value::Array(items)) if depth < 32 => items
            .iter()
            .map(|item| value_cost(Some(item), depth + 1))
            .sum(),
        Some(Value::Object(members)) if depth < 32 => members
            .values()
            .map(|item| value_cost(Some(item), depth + 1))
            .sum(),
        Some(_) => 8,
    }
}

fn set_slot(container: &mut Value, slot: &Slot, value: Value) {
    match slot {
        Slot::Element(index) => {
            if let Value::Array(items) = container {
                if *index == items.len() {
                    items.push(value);
                } else if let Some(existing) = items.get_mut(*index) {
                    *existing = value;
                }
            }
        }
        Slot::Member(key) => {
            if let Value::Object(members) = container {
                members.insert(key.clone(), value);
            }
        }
        Slot::Root => {}
    }
}
