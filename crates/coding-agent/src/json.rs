//! JSON as upstream reads its config files: `JSON.parse`, and `stripJsonComments` for models.json.
//!
//! Syntax error messages are serde_json's, not V8's (`Unexpected token ...`); upstream shows them
//! only inside "Failed to parse ..." warnings.

use serde_json::Value;

pub fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// `stripJsonComments`: drops `//` line comments and trailing commas outside strings.
pub fn strip_json_comments(input: &str) -> String {
    let no_comments = scan(input, |rest| {
        if rest.starts_with("//") {
            Some((rest.find('\n').unwrap_or(rest.len()), String::new()))
        } else {
            None
        }
    });
    scan(&no_comments, |rest| {
        if !rest.starts_with(',') {
            return None;
        }
        let after = &rest[1..];
        let ws = after.len() - after.trim_start().len();
        match after[ws..].chars().next() {
            Some('}' | ']') => Some((1, String::new())),
            _ => None,
        }
    })
}

/// Copies `input`, skipping over string literals, and lets `at` replace text elsewhere: it gets
/// the rest of the input and returns how many bytes to drop and what to put instead.
fn scan(input: &str, at: impl Fn(&str) -> Option<(usize, String)>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        if rest.starts_with('"') {
            let end = string_end(rest);
            out.push_str(&rest[..end]);
            i += end;
            continue;
        }
        if let Some((skip, with)) = at(rest) {
            out.push_str(&with);
            i += skip;
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Length of the string literal at the start of `text` (an unterminated one is a lone `"`).
fn string_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'"' => return i + 1,
            b'\n' => return 1,
            _ => i += 1,
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips() {
        let text = "{\n // c\n \"a\": \"x//y\", \"b\": [1, 2,],\n}";
        assert_eq!(
            parse(&strip_json_comments(text)).unwrap(),
            serde_json::json!({"a": "x//y", "b": [1, 2]})
        );
    }
}
