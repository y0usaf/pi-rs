//! `core/tools/truncate.ts`: the line and byte limits on tool output.
//!
//! Lengths that upstream measures with `string.length` are UTF-16 code units here too.

use serde::Serialize;

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;
/// Max characters per grep match line.
pub const GREP_MAX_LINE_LENGTH: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TruncatedBy {
    Lines,
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TruncationResult {
    pub content: String,
    pub truncated: bool,
    pub truncated_by: Option<TruncatedBy>,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub output_lines: usize,
    pub output_bytes: usize,
    pub last_line_partial: bool,
    pub first_line_exceeds_limit: bool,
    pub max_lines: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Limits {
    pub max_lines: Option<usize>,
    pub max_bytes: Option<usize>,
}

fn split_lines_for_counting(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// `(bytes / unit).toFixed(1)`: JavaScript rounds an exact tie up, so round in integers.
fn to_fixed_one(bytes: usize, unit: usize) -> String {
    let tenths = (bytes as u128 * 20 + unit as u128) / (unit as u128 * 2);
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Bytes as `12B`, `1.5KB` or `2.0MB`.
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{}KB", to_fixed_one(bytes, 1024))
    } else {
        format!("{}MB", to_fixed_one(bytes, 1024 * 1024))
    }
}

fn untruncated(
    content: &str,
    lines: usize,
    bytes: usize,
    max_lines: usize,
    max_bytes: usize,
) -> TruncationResult {
    TruncationResult {
        content: content.to_owned(),
        truncated: false,
        truncated_by: None,
        total_lines: lines,
        total_bytes: bytes,
        output_lines: lines,
        output_bytes: bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Keep the first lines that fit. Never returns a partial line; when the first line alone is over
/// the byte limit the content is empty and `first_line_exceeds_limit` is set.
pub fn truncate_head(content: &str, limits: Limits) -> TruncationResult {
    let max_lines = limits.max_lines.unwrap_or(DEFAULT_MAX_LINES);
    let max_bytes = limits.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let total_bytes = content.len();
    let lines = split_lines_for_counting(content);
    let total_lines = lines.len();
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return untruncated(content, total_lines, total_bytes, max_lines, max_bytes);
    }
    if lines[0].len() > max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines,
            max_bytes,
        };
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0;
    let mut by = TruncatedBy::Lines;
    for (i, line) in lines.iter().enumerate().take(max_lines) {
        let line_bytes = line.len() + usize::from(i > 0);
        if bytes + line_bytes > max_bytes {
            by = TruncatedBy::Bytes;
            break;
        }
        kept.push(line);
        bytes += line_bytes;
    }
    if kept.len() >= max_lines && bytes <= max_bytes {
        by = TruncatedBy::Lines;
    }
    let out = kept.join("\n");
    TruncationResult {
        output_lines: kept.len(),
        output_bytes: out.len(),
        content: out,
        truncated: true,
        truncated_by: Some(by),
        total_lines,
        total_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Keep the last lines that fit (bash output). May return the end of a line when the last line
/// alone is over the byte limit.
pub fn truncate_tail(content: &str, limits: Limits) -> TruncationResult {
    let max_lines = limits.max_lines.unwrap_or(DEFAULT_MAX_LINES);
    let max_bytes = limits.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let total_bytes = content.len();
    let lines = split_lines_for_counting(content);
    let total_lines = lines.len();
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return untruncated(content, total_lines, total_bytes, max_lines, max_bytes);
    }
    let mut kept: Vec<String> = Vec::new();
    let mut bytes = 0;
    let mut by = TruncatedBy::Lines;
    let mut last_line_partial = false;
    for line in lines.iter().rev() {
        if kept.len() >= max_lines {
            break;
        }
        let line_bytes = line.len() + usize::from(!kept.is_empty());
        if bytes + line_bytes > max_bytes {
            by = TruncatedBy::Bytes;
            if kept.is_empty() {
                let cut = truncate_string_to_bytes_from_end(line, max_bytes);
                bytes = cut.len();
                kept.push(cut.to_owned());
                last_line_partial = true;
            }
            break;
        }
        kept.push((*line).to_owned());
        bytes += line_bytes;
    }
    kept.reverse();
    if kept.len() >= max_lines && bytes <= max_bytes {
        by = TruncatedBy::Lines;
    }
    let out = kept.join("\n");
    TruncationResult {
        output_lines: kept.len(),
        output_bytes: out.len(),
        content: out,
        truncated: true,
        truncated_by: Some(by),
        total_lines,
        total_bytes,
        last_line_partial,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

fn truncate_string_to_bytes_from_end(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Length in UTF-16 code units, JavaScript's `string.length`.
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.slice(0, n)` in UTF-16 code units. A cut inside a surrogate pair keeps a lone
/// surrogate in JavaScript; here it becomes U+FFFD, which is what Node writes for it as UTF-8.
pub fn js_prefix(text: &str, units: usize) -> String {
    let utf16: Vec<u16> = text.encode_utf16().take(units).collect();
    String::from_utf16_lossy(&utf16)
}

/// One grep match line cut to `max_chars`, with a `... [truncated]` suffix when cut.
pub fn truncate_line(line: &str, max_chars: usize) -> (String, bool) {
    if js_len(line) <= max_chars {
        (line.to_owned(), false)
    } else {
        (
            format!("{}... [truncated]", js_prefix(line, max_chars)),
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(1023), "1023B");
        assert_eq!(format_size(51200), "50.0KB");
        assert_eq!(format_size(1536), "1.5KB");
        assert_eq!(format_size(1280), "1.3KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0MB");
    }

    #[test]
    fn head_by_lines() {
        let r = truncate_head(
            "a\nb\nc\n",
            Limits {
                max_lines: Some(2),
                max_bytes: None,
            },
        );
        assert_eq!(r.content, "a\nb");
        assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(r.total_lines, 3);
    }

    #[test]
    fn tail_partial() {
        let r = truncate_tail(
            "x\nabcdef",
            Limits {
                max_lines: None,
                max_bytes: Some(3),
            },
        );
        assert_eq!(r.content, "def");
        assert!(r.last_line_partial);
    }
}
