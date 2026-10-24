//! `streaming-json.test.ts`: incremental, tolerant parsing of JSON that arrives in pieces.

use pi_ai::json_parse::parse_streaming_json;
use pi_ai::utils::streaming_json::{
    JsonChange, JsonPathSegment, StreamingJsonParser, ToolCallArgumentsStream,
};
use serde_json::{Map, Value};

// #9265: streamed tool-call arguments were re-parsed in full on every delta.

/// A 31-bit linear congruential generator, so every run checks the same documents.
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> f64 {
        self.0 = (self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345)) & 0x7fff_ffff;
        self.0 as f64 / 0x7fff_ffff as f64
    }

    fn index(&mut self, length: usize) -> usize {
        ((self.next() * length as f64) as usize).min(length.saturating_sub(1))
    }

    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        items[self.index(items.len())]
    }
}

const STRING_PIECES: [&str; 17] = [
    "a", "b", "x y", " ", "\n", "\t", "\\", "\"", "/", "é", "😀", "\u{1}", "\u{1f}", "\u{2028}",
    "{", "]", ",",
];

fn random_string(rand: &mut Random) -> String {
    let mut text = String::new();
    let length = rand.index(6);
    for _ in 0..length {
        text.push_str(rand.pick(&STRING_PIECES));
    }
    text
}

fn random_scalar(rand: &mut Random) -> Value {
    match rand.index(10) {
        0 => Value::String(random_string(rand)),
        1 => Value::from((rand.next() * 2000.0) as i64 - 1000),
        2 => Value::from(rand.next() * 100.0 + 0.5),
        3 => Value::from(1e21),
        4 => Value::from(-5e-8),
        5 => Value::from(0),
        6 => Value::Bool(true),
        7 => Value::Bool(false),
        _ => Value::Null,
    }
}

fn random_value(rand: &mut Random, depth: usize) -> Value {
    let roll = rand.next();
    if depth > 3 || roll < 0.3 {
        return random_scalar(rand);
    }
    if roll < 0.65 {
        let mut members = Map::new();
        let count = rand.index(4);
        for _ in 0..count {
            members.insert(random_string(rand), random_value(rand, depth + 1));
        }
        return Value::Object(members);
    }
    let count = rand.index(4);
    Value::Array((0..count).map(|_| random_value(rand, depth + 1)).collect())
}

/// Insignificant whitespace, as the reference renderer spreads it.
fn space(rand: &mut Random, spaced: bool) -> &'static str {
    if spaced {
        rand.pick(&["", "", " ", "\n  ", "\t", "\r\n"])
    } else {
        ""
    }
}

/// JSON text of `value`, with random insignificant whitespace when `spaced`.
fn render(value: &Value, rand: &mut Random, spaced: bool) -> String {
    match value {
        Value::Array(items) => {
            let head = space(rand, spaced);
            let separator = format!("{},{}", space(rand, spaced), space(rand, spaced));
            let parts: Vec<String> = items
                .iter()
                .map(|item| render(item, rand, spaced))
                .collect();
            let tail = space(rand, spaced);
            format!("[{head}{}{tail}]", parts.join(&separator))
        }
        Value::Object(members) => {
            let head = space(rand, spaced);
            let separator = format!("{},{}", space(rand, spaced), space(rand, spaced));
            let parts: Vec<String> = members
                .iter()
                .map(|(key, item)| {
                    let key = Value::String(key.clone());
                    let before = space(rand, spaced);
                    let after = space(rand, spaced);
                    format!("{key}{before}:{after}{}", render(item, rand, spaced))
                })
                .collect();
            let tail = space(rand, spaced);
            format!("{{{head}{}{tail}}}", parts.join(&separator))
        }
        scalar => serde_json::to_string(scalar).unwrap_or_default(),
    }
}

fn random_documents(seed: u64, count: usize, spaced: bool) -> Vec<String> {
    let mut rand = Random::new(seed);
    (0..count)
        .map(|_| {
            let value = if rand.next() < 0.8 {
                let mut members = Map::new();
                members.insert("path".to_owned(), Value::String(random_string(&mut rand)));
                members.insert("args".to_owned(), random_value(&mut rand, 0));
                Value::Object(members)
            } else {
                random_value(&mut rand, 0)
            };
            render(&value, &mut rand, spaced)
        })
        .collect()
}

struct Reference {
    value: Value,
    complete: bool,
    end: usize,
}

/// Reference for the parser on a prefix of valid JSON: a plain recursive-descent reading of the whole
/// prefix that follows the documented rules. It is written independently of the incremental parser so
/// the two check each other.
fn reference_snapshot(text: &str, pending_scalars: bool) -> Option<Value> {
    let chars: Vec<char> = text.chars().collect();
    let start = skip_space(&chars, 0);
    read_value(&chars, start, pending_scalars).map(|read| read.value)
}

fn skip_space(chars: &[char], start: usize) -> usize {
    let mut index = start;
    while index < chars.len() && matches!(chars[index], ' ' | '\t' | '\n' | '\r') {
        index += 1;
    }
    index
}

fn read_value(chars: &[char], start: usize, pending: bool) -> Option<Reference> {
    match *chars.get(start)? {
        '"' => {
            let (value, complete, end) = read_string(chars, start);
            Some(Reference {
                value: Value::String(value),
                complete,
                end,
            })
        }
        '{' => Some(read_object(chars, start, pending)),
        '[' => Some(read_array(chars, start, pending)),
        _ => read_scalar(chars, start, pending),
    }
}

fn push_units(units: &mut Vec<u16>, character: char) {
    let mut buffer = [0u16; 2];
    units.extend_from_slice(character.encode_utf16(&mut buffer));
}

fn read_string(chars: &[char], start: usize) -> (String, bool, usize) {
    let mut units: Vec<u16> = Vec::new();
    let mut index = start + 1;
    while index < chars.len() {
        let character = chars[index];
        if character == '"' {
            return (String::from_utf16_lossy(&units), true, index + 1);
        }
        if character != '\\' {
            push_units(&mut units, character);
            index += 1;
            continue;
        }
        let Some(&next) = chars.get(index + 1) else {
            return (String::from_utf16_lossy(&units), false, chars.len());
        };
        if let Some(decoded) = simple_escape(next) {
            push_units(&mut units, decoded);
            index += 2;
            continue;
        }
        if next == 'u' {
            let mut end = index + 2;
            while end < index + 6 && end < chars.len() && chars[end].is_ascii_hexdigit() {
                end += 1;
            }
            if end == index + 6 {
                let digits: String = chars[index + 2..end].iter().collect();
                units.push(u32::from_str_radix(&digits, 16).unwrap_or(0) as u16);
                index = end;
                continue;
            }
            if end == chars.len() {
                return (String::from_utf16_lossy(&units), false, end);
            }
            // Not four hex digits: the escape stays literal and the next character is read normally.
            for character in &chars[index..end] {
                push_units(&mut units, *character);
            }
            index = end;
            continue;
        }
        // Invalid escape: the backslash stays literal and the next character is read normally.
        units.push('\\' as u16);
        index += 1;
    }
    (String::from_utf16_lossy(&units), false, chars.len())
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

fn read_object(chars: &[char], start: usize, pending: bool) -> Reference {
    let mut members = Map::new();
    let partial = |members: Map<String, Value>| Reference {
        value: Value::Object(members),
        complete: false,
        end: chars.len(),
    };
    let mut index = skip_space(chars, start + 1);
    if index >= chars.len() {
        return partial(members);
    }
    if chars[index] == '}' {
        return Reference {
            value: Value::Object(members),
            complete: true,
            end: index + 1,
        };
    }
    loop {
        assert!(
            chars.get(index) == Some(&'"'),
            "the reference reads valid JSON only: {chars:?}"
        );
        let (key, key_complete, key_end) = read_string(chars, index);
        if !key_complete {
            return partial(members);
        }
        index = skip_space(chars, key_end);
        if index >= chars.len() {
            return partial(members);
        }
        index = skip_space(chars, index + 1);
        let Some(member) = read_value(chars, index, pending) else {
            return partial(members);
        };
        members.insert(key, member.value);
        if !member.complete {
            return partial(members);
        }
        index = skip_space(chars, member.end);
        if index >= chars.len() {
            return partial(members);
        }
        if chars[index] == '}' {
            return Reference {
                value: Value::Object(members),
                complete: true,
                end: index + 1,
            };
        }
        index = skip_space(chars, index + 1);
        if index >= chars.len() {
            return partial(members);
        }
    }
}

fn read_array(chars: &[char], start: usize, pending: bool) -> Reference {
    let mut elements: Vec<Value> = Vec::new();
    let partial = |elements: Vec<Value>| Reference {
        value: Value::Array(elements),
        complete: false,
        end: chars.len(),
    };
    let mut index = skip_space(chars, start + 1);
    if index >= chars.len() {
        return partial(elements);
    }
    if chars[index] == ']' {
        return Reference {
            value: Value::Array(elements),
            complete: true,
            end: index + 1,
        };
    }
    loop {
        let Some(element) = read_value(chars, index, pending) else {
            return partial(elements);
        };
        elements.push(element.value);
        if !element.complete {
            return partial(elements);
        }
        index = skip_space(chars, element.end);
        if index >= chars.len() {
            return partial(elements);
        }
        if chars[index] == ']' {
            return Reference {
                value: Value::Array(elements),
                complete: true,
                end: index + 1,
            };
        }
        index = skip_space(chars, index + 1);
        if index >= chars.len() {
            return partial(elements);
        }
    }
}

fn read_scalar(chars: &[char], start: usize, pending: bool) -> Option<Reference> {
    let mut end = start;
    while end < chars.len() && !" \t\n\r,:{}[]\"".contains(chars[end]) {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    if end < chars.len() {
        let value = serde_json::from_str(&token).expect("the reference reads valid JSON only");
        return Some(Reference {
            value,
            complete: true,
            end,
        });
    }
    if !pending {
        return None;
    }
    // At the end of the text: a literal prefix, a number, or a number whose exponent has no digits.
    for (literal, value) in [
        ("true", Value::Bool(true)),
        ("false", Value::Bool(false)),
        ("null", Value::Null),
    ] {
        if literal.starts_with(&token) {
            return Some(Reference {
                value,
                complete: false,
                end,
            });
        }
    }
    let value = reference_pending_number(&token)?;
    Some(Reference {
        value,
        complete: false,
        end,
    })
}

fn reference_pending_number(token: &str) -> Option<Value> {
    if token.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(token) {
        return Some(value);
    }
    let mantissa = ["e", "E"].iter().find_map(|marker| {
        let (mantissa, rest) = token.split_once(*marker)?;
        (rest.is_empty() || rest == "+" || rest == "-").then_some(mantissa)
    })?;
    serde_json::from_str::<Value>(mantissa).ok()
}

struct Step {
    read: String,
    snapshot: Option<Value>,
}

/// Feed `text` in pieces; returns the snapshot after each piece and the text read by then.
fn stream(text: &str, chunk_size: usize) -> Vec<Step> {
    let mut parser = StreamingJsonParser::new();
    let mut steps = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let mut end = (index + chunk_size).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        parser.append(&text[index..end]);
        index = end;
        steps.push(Step {
            read: text[..index].to_owned(),
            snapshot: parser.snapshot().cloned(),
        });
    }
    steps
}

fn stream_random(text: &str, seed: u64) -> Vec<Step> {
    let mut rand = Random::new(seed);
    let mut steps = Vec::new();
    let mut parser = StreamingJsonParser::new();
    let mut index = 0;
    while index < text.len() {
        let mut end = (index + 1 + rand.index(12)).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        parser.append(&text[index..end]);
        index = end;
        steps.push(Step {
            read: text[..index].to_owned(),
            snapshot: parser.snapshot().cloned(),
        });
    }
    steps
}

fn final_snapshot(text: &str, chunk_size: usize) -> Option<Value> {
    let chunk = if chunk_size == 0 {
        text.len().max(1)
    } else {
        chunk_size
    };
    stream(text, chunk)
        .last()
        .and_then(|step| step.snapshot.clone())
}

/// A stable seed from the text, so random chunk sizes differ per document.
fn read32(text: &str) -> u64 {
    let mut hash: i64 = 7;
    for unit in text.encode_utf16() {
        hash = (hash.wrapping_mul(31).wrapping_add(unit as i64)) & 0xffff_ffff;
    }
    hash.unsigned_abs() + 1
}

const CHUNK_SIZES: [usize; 5] = [1, 2, 3, 7, 0];

fn steps_for(text: &str, size: usize) -> Vec<Step> {
    if size == 0 {
        stream_random(text, read32(text))
    } else {
        stream(text, size)
    }
}

/// A copy of a value kept current by applying the parser's changes, as a writer such as Durable keeps
/// one.
#[derive(Default)]
struct Mirror {
    value: Option<Value>,
}

impl Mirror {
    fn apply(&mut self, changes: Vec<JsonChange>) {
        for change in changes {
            match change {
                JsonChange::Set { path, value } => {
                    if path.is_empty() {
                        self.value = Some(value);
                    } else {
                        let root = self.value.get_or_insert(Value::Null);
                        let (last, parents) = path.split_last().expect("a non-empty path");
                        let parent = value_at_mut(root, parents);
                        match (parent, last) {
                            (Value::Array(items), JsonPathSegment::Index(index)) => {
                                while items.len() <= *index {
                                    items.push(Value::Null);
                                }
                                items[*index] = value;
                            }
                            (Value::Object(members), JsonPathSegment::Key(key)) => {
                                members.insert(key.clone(), value);
                            }
                            (_, _) => {}
                        }
                    }
                }
                JsonChange::Append { path, text } => {
                    let root = self.value.get_or_insert(Value::Null);
                    let target = value_at_mut(root, &path);
                    let existing = match target {
                        Value::String(existing) => existing.clone(),
                        _ => String::new(),
                    };
                    *target = Value::String(existing + &text);
                }
            }
        }
    }
}

fn value_at_mut<'a>(current: &'a mut Value, path: &[JsonPathSegment]) -> &'a mut Value {
    match path.split_first() {
        None => current,
        Some((JsonPathSegment::Key(key), rest)) => {
            if !current.is_object() {
                *current = Value::Object(Map::new());
            }
            let members = current.as_object_mut().expect("an object");
            let child = members.entry(key.clone()).or_insert(Value::Null);
            value_at_mut(child, rest)
        }
        Some((JsonPathSegment::Index(index), rest)) => {
            if !current.is_array() {
                *current = Value::Array(Vec::new());
            }
            let items = current.as_array_mut().expect("an array");
            while items.len() <= *index {
                items.push(Value::Null);
            }
            value_at_mut(&mut items[*index], rest)
        }
    }
}

fn expect_json(value: &Option<Value>, label: &str) {
    fn walk(value: &Value, label: &str) {
        match value {
            Value::Number(number) => assert!(
                number.as_f64().is_some_and(f64::is_finite),
                "{label}: {number} is not finite"
            ),
            Value::Array(items) => items.iter().for_each(|item| walk(item, label)),
            Value::Object(members) => members.values().for_each(|item| walk(item, label)),
            _ => {}
        }
    }
    if let Some(value) = value {
        walk(value, label);
    }
}

#[test]
fn matches_the_reference_after_every_piece_at_every_chunk_size() {
    for text in random_documents(1, 250, true) {
        for size in CHUNK_SIZES {
            for step in steps_for(&text, size) {
                assert_eq!(
                    step.snapshot,
                    reference_snapshot(&step.read, true),
                    "{:?} (chunk size {size})",
                    step.read
                );
            }
        }
    }
}

#[test]
fn matches_parse_streaming_json_at_every_prefix_of_compact_documents() {
    for text in random_documents(2, 400, false) {
        for step in stream(&text, 1) {
            // Documented differences: parse_streaming_json trims trailing whitespace, and shows an
            // unfinished root null as {}.
            if step.read.ends_with(char::is_whitespace) {
                continue;
            }
            if step.snapshot == Some(Value::Null) && "null".starts_with(&step.read) {
                continue;
            }
            let expected = parse_streaming_json(Some(&step.read));
            let actual = step.snapshot.clone().unwrap_or(Value::Object(Map::new()));
            assert_eq!(actual, expected, "{:?}", step.read);
        }
    }
}

#[test]
fn reads_complete_documents_exactly_like_a_full_parse() {
    for text in random_documents(3, 300, true)
        .into_iter()
        .chain(random_documents(4, 300, false))
    {
        let expected: Value = serde_json::from_str(&text).expect("the generator writes valid JSON");
        for size in CHUNK_SIZES {
            assert_eq!(
                final_snapshot(&text, size).unwrap_or(Value::Null),
                expected,
                "{text}"
            );
        }
    }
}

#[test]
fn never_changes_a_snapshot_once_it_is_returned() {
    for text in random_documents(5, 150, true) {
        let mut parser = StreamingJsonParser::new();
        let mut held: Vec<(Value, String)> = Vec::new();
        for step in steps_for_mut(&mut parser, &text, read32(&text)) {
            if let Some(snapshot) = step.snapshot.clone() {
                held.push((snapshot, step.read.clone()));
            }
        }
        for (snapshot, read) in held {
            assert_eq!(Some(snapshot), reference_snapshot(&read, true), "{read}");
        }
    }
}

fn steps_for_mut(parser: &mut StreamingJsonParser, text: &str, seed: u64) -> Vec<Step> {
    let mut rand = Random::new(seed);
    let mut steps = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let mut end = (index + 1 + rand.index(12)).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        parser.append(&text[index..end]);
        index = end;
        steps.push(Step {
            read: text[..index].to_owned(),
            snapshot: parser.snapshot().cloned(),
        });
    }
    steps
}

#[test]
fn repairs_control_characters_and_invalid_escapes_while_streaming() {
    let mut rand = Random::new(6);
    let malformed = [
        "\n", "\t", "\u{0}", "\\d", "\\x41", "\\u12", "\\uZZZZ", "\\u00e9", "\\\\", "\\q",
    ];
    for round in 0..300 {
        let mut content = String::new();
        for _ in 0..6 {
            content.push_str(rand.pick(&malformed));
        }
        let text = format!("{{\"path\":\"p\",\"content\":\"{content}\",\"n\":{round}}}");
        for step in stream(&text, 1) {
            assert_eq!(
                step.snapshot,
                reference_snapshot(&step.read, true),
                "{:?}",
                step.read
            );
        }
    }
}

#[test]
fn holds_an_escape_back_until_it_is_complete() {
    for (text, value) in [
        (r#"{"a":"x\u00e9y"}"#, "xéy"),
        (r#"{"a":"x\ny"}"#, "x\ny"),
        (r#"{"a":"x\\y"}"#, "x\\y"),
        (r#"{"a":"x\"y"}"#, "x\"y"),
        (r#"{"a":"x\/y"}"#, "x/y"),
    ] {
        for split in 0..=text.len() {
            if !text.is_char_boundary(split) {
                continue;
            }
            let mut parser = StreamingJsonParser::new();
            parser.append(&text[..split]);
            assert_eq!(
                parser.snapshot().cloned(),
                reference_snapshot(&text[..split], true),
                "{:?}",
                &text[..split]
            );
            parser.append(&text[split..]);
            let mut members = Map::new();
            members.insert("a".to_owned(), Value::String(value.to_owned()));
            assert_eq!(parser.snapshot().cloned(), Some(Value::Object(members)));
        }
    }
}

#[test]
fn joins_surrogate_pairs_split_across_pieces() {
    for text in [r#"{"a":"\ud83d\ude00!"}"#, "{\"a\":\"😀!\"}"] {
        for split in 0..=text.len() {
            if !text.is_char_boundary(split) {
                continue;
            }
            let mut parser = StreamingJsonParser::new();
            parser.append(&text[..split]);
            parser.snapshot();
            parser.append(&text[split..]);
            let mut members = Map::new();
            members.insert("a".to_owned(), Value::String("😀!".to_owned()));
            assert_eq!(
                parser.snapshot().cloned(),
                Some(Value::Object(members)),
                "{text:?}"
            );
        }
    }
    // A lone high surrogate cannot live in a Rust String: it is held back like an unterminated
    // escape and shows as U+FFFD once the string ends.
    let mut parser = StreamingJsonParser::new();
    parser.append(r#"{"a":"\ud83d"#);
    assert_eq!(
        parser.snapshot().cloned(),
        Some(serde_json::json!({"a": ""}))
    );
    parser.append(r#"\ude00""#);
    assert_eq!(
        parser.snapshot().cloned(),
        Some(serde_json::json!({"a": "😀"}))
    );
    let mut lone = StreamingJsonParser::new();
    lone.append(r#"{"a":"\ud83d\uZZZZ"}"#);
    assert_eq!(
        lone.snapshot().cloned(),
        Some(serde_json::json!({"a": "\u{fffd}\\uZZZZ"}))
    );
}

#[test]
fn keeps_trailing_whitespace_of_an_unterminated_string() {
    let mut parser = StreamingJsonParser::new();
    parser.append("{\"code\":\"x\t\t  ");
    assert_eq!(
        parser.snapshot().cloned(),
        Some(serde_json::json!({"code": "x\t\t  "}))
    );
}

#[test]
fn keeps_raw_control_characters_and_invalid_escapes_visible() {
    let mut parser = StreamingJsonParser::new();
    parser.append("{\"a\":\"x\ny \\d \\u12\"");
    assert_eq!(
        parser.snapshot().cloned(),
        Some(serde_json::json!({"a": "x\ny \\d \\u12"}))
    );
}

#[test]
fn keeps_invalid_escapes_literal_without_dropping_fields() {
    assert_eq!(
        final_snapshot(
            r#"{"path":"a.tex","content":"\usepackage{x}","mode":"w"}"#,
            1
        ),
        Some(serde_json::json!({"path": "a.tex", "content": "\\usepackage{x}", "mode": "w"}))
    );
    assert_eq!(
        final_snapshot(r#"{"text":"caf\u00e9 \url"}"#, 3),
        Some(serde_json::json!({"text": "café \\url"}))
    );
    let mut parser = StreamingJsonParser::new();
    parser.append(r#"{"text":"ab\u00"#);
    assert_eq!(
        parser.snapshot().cloned(),
        Some(serde_json::json!({"text": "ab"}))
    );
}

#[test]
fn keeps_the_last_value_of_a_duplicate_key_in_the_first_keys_position() {
    let text = r#"{"a":1,"b":2,"a":"three"}"#;
    let parsed: Value = serde_json::from_str(text).expect("valid JSON");
    let snapshot = final_snapshot(text, 1).expect("a value");
    assert_eq!(snapshot, parsed);
    let keys = |value: &Value| {
        value
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&snapshot), keys(&parsed));
}

/// Expected values of the inputs, from other partial-JSON parsers' public test suites.
fn vectors() -> Vec<(&'static str, Option<Value>)> {
    // partial-json (MIT, https://github.com/promplate/partial-json-parser-js), with every partial type
    // allowed, as parse_streaming_json uses it. Differences, documented in the module: an invalid
    // escape stays visible (partial-json cuts the string at the backslash), and Infinity/NaN words are
    // not JSON and are dropped.
    let mut vectors: Vec<(&'static str, Option<Value>)> = vec![
        (r#"""#, Some(Value::from(""))),
        (r#"[""#, Some(serde_json::json!([""]))),
        (r#"{"": ""#, Some(serde_json::json!({"": ""}))),
        ("n", Some(Value::Null)),
        ("t", Some(Value::Bool(true))),
        ("f", Some(Value::Bool(false))),
        ("0", Some(Value::from(0))),
        ("-1.25e+4", Some(Value::from(-1.25e4))),
        ("-1.25e+", Some(Value::from(-1.25))),
        ("-1.25e", Some(Value::from(-1.25))),
        (r#"" \x12"#, Some(Value::from(" \\x12"))),
        ("I", None),
        ("-I", None),
        ("N", None),
    ];
    // Vercel AI SDK fixJson (Apache-2.0): the expected values are the parses of its repaired texts.
    vectors.extend([
        ("", None),
        ("nul", Some(Value::Null)),
        ("t", Some(Value::Bool(true))),
        ("fals", Some(Value::Bool(false))),
        ("12.2", Some(Value::from(12.2))),
        ("-12", Some(Value::from(-12))),
        ("-", None),
        ("2.5e", Some(Value::from(2.5))),
        ("2.5e-", Some(Value::from(2.5))),
        ("2.5e3", Some(Value::from(2.5e3))),
        ("-2.5e3", Some(Value::from(-2.5e3))),
        ("2.5E", Some(Value::from(2.5))),
        ("2.5E-", Some(Value::from(2.5))),
        ("2.5E3", Some(Value::from(2.5e3))),
        ("-2.5E3", Some(Value::from(-2.5e3))),
        ("12.34e", Some(Value::from(12.34))),
        ("5e", Some(Value::from(5))),
        (r#""abc"#, Some(Value::from("abc"))),
        (
            r#""value with \"quoted\" text and \\ escape"#,
            Some(Value::from("value with \"quoted\" text and \\ escape")),
        ),
        (r#""value with \"#, Some(Value::from("value with "))),
        (r#""\u"#, Some(Value::from(""))),
        (r#""\u12"#, Some(Value::from(""))),
        (r#""text \u00"#, Some(Value::from("text "))),
        (r#"{"a":"\u12"#, Some(serde_json::json!({"a": ""}))),
        (r#""value with unicode <""#, Some(Value::from("value with unicode <"))),
        ("[", Some(serde_json::json!([]))),
        ("[[1], [2", Some(serde_json::json!([[1], [2]]))),
        (r#"[["1"], ["2"#, Some(serde_json::json!([["1"], ["2"]]))),
        ("[[false], [nu", Some(serde_json::json!([[false], [null]]))),
        ("[[[]], [[]", Some(serde_json::json!([[[]], [[]]]))),
        ("[[{}], [{", Some(serde_json::json!([[{}], [{}]]))),
        ("[1, ", Some(serde_json::json!([1]))),
        ("[[], 123", Some(serde_json::json!([[], 123]))),
        (r#"{"key":"#, Some(serde_json::json!({}))),
        (
            r#"{"a": {"b": 1}, "c": {"d": 2"#,
            Some(serde_json::json!({"a": {"b": 1}, "c": {"d": 2}})),
        ),
        (
            r#"{"a": {"b": "1"}, "c": {"d": 2"#,
            Some(serde_json::json!({"a": {"b": "1"}, "c": {"d": 2}})),
        ),
        (
            r#"{"a": {"b": false}, "c": {"d": 2"#,
            Some(serde_json::json!({"a": {"b": false}, "c": {"d": 2}})),
        ),
        (
            r#"{"a": {"b": []}, "c": {"d": 2"#,
            Some(serde_json::json!({"a": {"b": []}, "c": {"d": 2}})),
        ),
        (
            r#"{"a": {"b": {}}, "c": {"d": 2"#,
            Some(serde_json::json!({"a": {"b": {}}, "c": {"d": 2}})),
        ),
        (r#"{"ke"#, Some(serde_json::json!({}))),
        (r#"{"k1": 1, "k2"#, Some(serde_json::json!({"k1": 1}))),
        (r#"{"k1": 1, "k2":"#, Some(serde_json::json!({"k1": 1}))),
        (r#"{"key": "value"  "#, Some(serde_json::json!({"key": "value"}))),
        (r#"{"a": {"b": {}"#, Some(serde_json::json!({"a": {"b": {}}}))),
        ("[1, [2, 3, [", Some(serde_json::json!([1, [2, 3, []]]))),
        ("[false, [true, [", Some(serde_json::json!([false, [true, []]]))),
        (r#"{"key": {"subKey":"#, Some(serde_json::json!({"key": {}}))),
        (
            r#"{"key": 123, "key2": {"subKey":"#,
            Some(serde_json::json!({"key": 123, "key2": {}})),
        ),
        (
            r#"{"key": null, "key2": {"subKey":"#,
            Some(serde_json::json!({"key": null, "key2": {}})),
        ),
        (
            r#"{"key": [1, 2, {"#,
            Some(serde_json::json!({"key": [1, 2, {}]})),
        ),
        (
            r#"[1, 2, {"key": "value","#,
            Some(serde_json::json!([1, 2, {"key": "value"}])),
        ),
        (
            r#"{"a": {"b": ["c", {"d": "e","#,
            Some(serde_json::json!({"a": {"b": ["c", {"d": "e"}]}})),
        ),
        (
            r#"{"a": {"b": {"c": {"d":"#,
            Some(serde_json::json!({"a": {"b": {"c": {}}}})),
        ),
        (r#"{"a": 1, "b": ["#, Some(serde_json::json!({"a": 1, "b": []}))),
        (r#"{"a": 1, "b": {"#, Some(serde_json::json!({"a": 1, "b": {}}))),
        (r#"{"a": 1, "b": ""#, Some(serde_json::json!({"a": 1, "b": ""}))),
        (
            "{\n  \"a\": [\n    {\n      \"a1\": \"v1\",\n      \"a2\": \"v2\",\n      \"a3\": \"v3\"\n    }\n  ],\n  \"b\": [\n    {\n      \"b1\": \"n",
            Some(serde_json::json!({"a": [{"a1": "v1", "a2": "v2", "a3": "v3"}], "b": [{"b1": "n"}]})),
        ),
        (
            r#"{"type":"div","children":[{"type":"Card","props":{}"#,
            Some(serde_json::json!({"type": "div", "children": [{"type": "Card", "props": {}}]})),
        ),
        // Difference: fixJson completes 12. to 12; like parse_streaming_json, a number cut after its
        // dot is omitted.
        ("12.", None),
        ("12.e", None),
    ]);
    vectors
}

#[test]
fn reads_the_ported_test_vectors() {
    for (input, expected) in vectors() {
        for size in [0, 1, 2] {
            assert_eq!(
                final_snapshot(input, size),
                expected,
                "{input:?} (size {size})"
            );
        }
    }
}

fn base64_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let Some(index) = ALPHABET.iter().position(|candidate| *candidate == byte) else {
            continue;
        };
        buffer = (buffer << 6) | index as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    bytes
}

#[test]
fn reads_every_json_test_suite_document_a_full_parse_accepts_and_never_fails_on_the_rest() {
    let suite: Value = serde_json::from_str(include_str!("fixtures/json-test-suite.json"))
        .expect("the fixture is JSON");
    let files = suite["files"].as_object().expect("a files map");
    for (name, encoded) in files {
        let encoded = encoded.as_str().expect("a base64 string");
        let text = String::from_utf8_lossy(&base64_decode(encoded)).into_owned();
        let expected: Option<Value> = serde_json::from_str(&text).ok();
        for size in [0, 1, 3] {
            let snapshot = final_snapshot(&text, size);
            match &expected {
                Some(expected) => {
                    assert_eq!(snapshot.as_ref(), Some(expected), "{name} (size {size})")
                }
                None => expect_json(&snapshot, name),
            }
        }
    }
}

const MALFORMED: [&str; 18] = [
    r#"{"a":1 "b":2}"#,
    r#"{"a" 1, "b": 2}"#,
    "{a:1, 'b': 2}",
    r#"{"a":1,,"b":2,}"#,
    "[1 2, , 3,]",
    r#"{"a":[1,2}, "b":3]"#,
    "]]]}}}{",
    r#"{"a":NaN,"b":Infinity,"c":-Infinity,"d":undefined,"e":1}"#,
    r#"{"a":01,"b":1.,"c":.5,"d":+1,"e":0x10,"f":1e}"#,
    r#"{"a":"x"} trailing garbage {"b":2}"#,
    r#"{"a": tru, "b": nulll, "c": falsey}"#,
    r#"{"a":{"b":{"c":"#,
    r#"{"a\": 1}"#,
    r#""unterminated \"#,
    "// comment\n{\"a\":1}",
    "\u{0}{\u{1}\"a\"\u{2}:\u{3}1}",
    r#"{"a":"x"}}}]]]"#,
    "[",
];

#[test]
fn never_fails_keeps_snapshots_plain_and_reads_every_chunk_size_alike() {
    for text in MALFORMED {
        for size in CHUNK_SIZES {
            let steps = steps_for(text, size);
            for step in &steps {
                expect_json(&step.snapshot, text);
            }
            assert_eq!(
                steps.last().and_then(|step| step.snapshot.clone()),
                final_snapshot(text, 1),
                "{text}"
            );
        }
    }
}

#[test]
fn tolerates_missing_commas_and_colons_and_drops_values_in_invalid_positions() {
    assert_eq!(
        final_snapshot(r#"{"a":1 "b":2}"#, 1),
        Some(serde_json::json!({"a": 1, "b": 2}))
    );
    assert_eq!(
        final_snapshot(r#"{"a" 1, "b": 2}"#, 1),
        Some(serde_json::json!({"a": 1, "b": 2}))
    );
    assert_eq!(
        final_snapshot("[1 2, , 3,]", 1),
        Some(serde_json::json!([1, 2, 3]))
    );
    assert_eq!(
        final_snapshot(r#"{"a":NaN,"b":Infinity,"e":1}"#, 1),
        Some(serde_json::json!({"e": 1}))
    );
    assert_eq!(
        final_snapshot(r#"{"a":"x"} trailing garbage {"b":2}"#, 1),
        Some(serde_json::json!({"a": "x"}))
    );
}

#[test]
fn survives_random_corruption_of_valid_documents() {
    let mut rand = Random::new(7);
    let noise = [
        "\"", "\\", "{", "}", "[", "]", ",", ":", "x", "1", "-", "\n", "t",
    ];
    for text in random_documents(8, 300, true) {
        let mut characters: Vec<char> = text.chars().collect();
        for _ in 0..3 {
            let at = rand.index(characters.len() + 1);
            if rand.next() < 0.5 && at < characters.len() {
                characters.remove(at);
            } else {
                let piece = rand.pick(&noise);
                characters.insert(at, piece.chars().next().unwrap_or('x'));
            }
        }
        let corrupted: String = characters.into_iter().collect();
        for step in steps_for(&corrupted, 0) {
            expect_json(&step.snapshot, &corrupted);
        }
        // The changes keep a copy equal to the parser's own value, malformed or not.
        let mut parser = StreamingJsonParser::new();
        let mut mirror = Mirror::default();
        let mut index = 0;
        while index < corrupted.len() {
            let mut end = (index + 4).min(corrupted.len());
            while !corrupted.is_char_boundary(end) {
                end += 1;
            }
            mirror.apply(parser.append(&corrupted[index..end]));
            index = end;
        }
        // A space ends a number or literal at the end, which snapshots show and changes do not yet.
        mirror.apply(parser.append(" "));
        assert_eq!(mirror.value, parser.snapshot().cloned(), "{corrupted}");
    }
}

fn object(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

fn set(path: Vec<JsonPathSegment>, value: Value) -> JsonChange {
    JsonChange::Set { path, value }
}

fn append(path: Vec<JsonPathSegment>, text: &str) -> JsonChange {
    JsonChange::Append {
        path,
        text: text.to_owned(),
    }
}

fn key(name: &str) -> JsonPathSegment {
    JsonPathSegment::Key(name.to_owned())
}

fn index(at: usize) -> JsonPathSegment {
    JsonPathSegment::Index(at)
}

#[test]
fn changes_keep_a_copy_equal_to_the_value_read_so_far_and_to_a_full_parse_at_the_end() {
    for text in random_documents(9, 200, true) {
        for size in CHUNK_SIZES {
            let mut parser = StreamingJsonParser::new();
            let mut mirror = Mirror::default();
            let mut index = 0;
            let steps: Vec<String> = {
                let mut rand = Random::new(read32(&text));
                let mut reads = Vec::new();
                while index < text.len() {
                    let mut end = if size == 0 {
                        (index + 1 + rand.index(12)).min(text.len())
                    } else {
                        (index + size).min(text.len())
                    };
                    while !text.is_char_boundary(end) {
                        end += 1;
                    }
                    mirror.apply(parser.append(&text[index..end]));
                    index = end;
                    reads.push(text[..index].to_owned());
                    // Changes leave out unterminated numbers and literals, which snapshots show.
                    assert_eq!(
                        mirror.value,
                        reference_snapshot(&text[..index], false),
                        "{:?}",
                        &text[..index]
                    );
                }
                reads
            };
            let _ = steps;
            // A space ends a number or literal at the very end, which changes report only then.
            mirror.apply(parser.append(" "));
            assert_eq!(
                mirror.value,
                Some(serde_json::from_str(&text).expect("valid JSON")),
                "{text}"
            );
        }
    }
}

#[test]
fn changes_report_a_number_that_ends_and_a_string_that_starts_in_one_piece_in_order() {
    let mut parser = StreamingJsonParser::new();
    assert_eq!(
        parser.append(r#"{"n":1"#),
        vec![set(Vec::new(), serde_json::json!({}))]
    );
    assert_eq!(
        parser.append(r#"2,"s":"ab"#),
        vec![
            set(vec![key("n")], Value::from(12)),
            set(vec![key("s")], Value::from("ab")),
        ]
    );
    assert_eq!(
        parser.append(r#"c","t":tru"#),
        vec![append(vec![key("s")], "c")]
    );
    assert_eq!(
        parser.append("e}"),
        vec![set(vec![key("t")], Value::Bool(true))]
    );
    assert_eq!(parser.append(r#" {"ignored":1}"#), Vec::new());
}

#[test]
fn changes_report_nested_objects_and_arrays_as_empty_containers_then_their_members_and_items() {
    let mut parser = StreamingJsonParser::new();
    assert_eq!(
        parser.append(r#"{"a":[1,{"b":["x""#),
        vec![
            set(Vec::new(), serde_json::json!({})),
            set(vec![key("a")], serde_json::json!([])),
            set(vec![key("a"), index(0)], Value::from(1)),
            set(vec![key("a"), index(1)], serde_json::json!({})),
            set(vec![key("a"), index(1), key("b")], serde_json::json!([])),
            set(
                vec![key("a"), index(1), key("b"), index(0)],
                Value::from("x")
            ),
        ]
    );
    assert_eq!(
        parser.append(r#","y"#),
        vec![set(
            vec![key("a"), index(1), key("b"), index(1)],
            Value::from("y")
        )]
    );
    assert_eq!(
        parser.append(r#"z"]}],"a":"again"}"#),
        vec![
            append(vec![key("a"), index(1), key("b"), index(1)], "z"),
            set(vec![key("a")], Value::from("again")),
        ]
    );
}

#[test]
fn changes_hold_split_escapes_back_and_report_their_decoded_text_once_complete() {
    let text = r#"{"a":"x\u00e9\n\\\"\ud83d\ude00\q"}"#;
    let expected: Value =
        serde_json::from_str(r#"{"a":"x\u00e9\n\\\"\ud83d\ude00\\q"}"#).expect("valid JSON");
    for first in 1..text.len() {
        for second in first..text.len() {
            if !text.is_char_boundary(first) || !text.is_char_boundary(second) {
                continue;
            }
            let mut parser = StreamingJsonParser::new();
            let mut mirror = Mirror::default();
            for piece in [&text[..first], &text[first..second], &text[second..]] {
                mirror.apply(parser.append(piece));
            }
            assert_eq!(mirror.value.as_ref(), Some(&expected), "{first} {second}");
        }
    }
    let mut parser = StreamingJsonParser::new();
    parser.append(r#"{"a":"x"#);
    assert_eq!(parser.append(r#"\u00"#), Vec::new());
    assert_eq!(parser.append("e9"), vec![append(vec![key("a")], "é")]);
}

#[test]
fn changes_report_a_container_nested_deeper_than_32_levels_once_whole_when_it_closes() {
    let depth = 40;
    let text = format!(
        "{}{}{}",
        "{\"a\":".repeat(depth),
        "\"deep\"",
        "}".repeat(depth)
    );
    let mut parser = StreamingJsonParser::new();
    let mut mirror = Mirror::default();
    let mut index = 0;
    while index < text.len() {
        let end = (index + 3).min(text.len());
        mirror.apply(parser.append(&text[index..end]));
        index = end;
    }
    assert_eq!(
        mirror.value,
        Some(serde_json::from_str(&text).expect("valid JSON"))
    );

    let changes = StreamingJsonParser::new().append(&text);
    assert_eq!(changes.len(), 34);
    let last = changes.last().expect("a last change");
    match last {
        JsonChange::Set { path, value } => {
            assert_eq!(path.len(), 32);
            let inner: Value =
                serde_json::from_str(&text[32 * 5..text.len() - 32]).expect("valid JSON");
            assert_eq!(value, &inner);
        }
        other => panic!("expected a set, got {other:?}"),
    }
}

#[test]
fn changes_report_nothing_for_values_in_invalid_positions_and_text_after_the_root() {
    let mut parser = StreamingJsonParser::new();
    assert_eq!(
        parser.append(r#"{"a":1 2, "b" "x"}"#),
        vec![
            set(Vec::new(), serde_json::json!({})),
            set(vec![key("a")], Value::from(1)),
            set(vec![key("b")], Value::from("x")),
        ]
    );
    assert_eq!(parser.append(r#"{"c":3}"#), Vec::new());
}

#[test]
fn arguments_show_partial_arguments_as_an_object_and_finish_with_one_full_parse() {
    let mut args = ToolCallArgumentsStream::new();
    assert_eq!(args.append("  "), object(serde_json::json!({})));
    assert_eq!(
        args.append(r#"{"path":"a.ts","content":"x"#),
        object(serde_json::json!({"path": "a.ts", "content": "x"}))
    );
    let before = args.append("y");
    assert_eq!(args.append(""), before);
    assert_eq!(args.text(), r#"  {"path":"a.ts","content":"xy"#);
    assert_eq!(
        args.finish(),
        object(serde_json::json!({"path": "a.ts", "content": "xy"}))
    );
    args.append(r#""}"#);
    assert_eq!(
        args.finish(),
        object(serde_json::json!({"path": "a.ts", "content": "xy"}))
    );
}

#[test]
fn arguments_show_non_object_roots_as_no_arguments_yet() {
    let mut args = ToolCallArgumentsStream::new();
    assert_eq!(args.append("[1,2"), object(serde_json::json!({})));
    args.append("]");
    assert_eq!(args.finish(), object(serde_json::json!({})));
}

#[test]
fn arguments_replace_the_text_with_the_providers_complete_text_returning_the_missing_piece() {
    let mut args = ToolCallArgumentsStream::new();
    args.append(r#"{"a":"x"#);
    let replaced = args.replace(r#"{"a":"xyz"}"#);
    assert_eq!(replaced.delta.as_deref(), Some(r#"yz"}"#));
    assert_eq!(replaced.arguments, object(serde_json::json!({"a": "xyz"})));
    let replaced = args.replace(r#"{"b":2}"#);
    assert_eq!(replaced.delta, None);
    assert_eq!(replaced.arguments, object(serde_json::json!({"b": 2})));
    assert_eq!(args.text(), r#"{"b":2}"#);
}
