//! Formatting helpers and the 2-D layout of upstream latex.ts (stacked fractions, limits, scripts,
//! matrices). Layout nodes are referenced from rendered text by private-use markers.

use super::tables;
use crate::unicode::{is_js_whitespace, is_letter, is_number};
use crate::utils::visible_width;

pub(super) const NEGATIVE_SPACE: &str = "\u{0000}";
pub(super) const NAMED_OPERATOR_START: char = '\u{f0004}';
pub(super) const NAMED_OPERATOR_END: char = '\u{f0005}';
pub(super) const LAYOUT_MARKER_START: char = '\u{f0000}';
pub(super) const LAYOUT_MARKER_END: char = '\u{f0001}';
pub(super) const PROTECTED_SPACE: char = '\u{f0002}';

/// JS `\s`.
pub(super) fn is_space(c: char) -> bool {
    is_js_whitespace(c) || matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{FEFF}')
}
pub(super) fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}
pub(super) fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_space)
}
pub(super) fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ScriptKind {
    Sub,
    Sup,
}

pub(super) enum Node {
    Fraction { numerator: String, denominator: String },
    Operator { operator: String, lower: Option<String>, upper: Option<String> },
    Script { lower: Option<String>, upper: Option<String> },
    Matrix { lines: Vec<String>, baseline: usize },
}

pub(super) fn marker(index: usize) -> String {
    format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}")
}

fn replace_characters(value: &str, table: fn(&str) -> Option<&'static str>) -> Option<String> {
    let mut out = String::new();
    let mut buf = [0u8; 4];
    for c in value.chars() {
        out.push_str(table(c.encode_utf8(&mut buf))?);
    }
    Some(out)
}

/// `value.trim().replace(/\s*([=+-])\s*/g, "$1")`.
fn normalize_script_value(value: &str) -> String {
    let mut out = String::new();
    let mut chars = trim(value).chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '=' | '+' | '-') {
            let keep = trim_end(&out).len();
            out.truncate(keep);
            out.push(c);
            while chars.peek().is_some_and(|&n| is_space(n)) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(super) fn format_unicode_script(value: &str, kind: ScriptKind) -> Option<String> {
    let table = if kind == ScriptKind::Sub { tables::subscript } else { tables::superscript };
    replace_characters(&normalize_script_value(value), table)
}

pub(super) fn format_script(value: &str, kind: ScriptKind) -> String {
    let value = normalize_script_value(value);
    if let Some(unicode) = format_unicode_script(&value, kind) {
        return unicode;
    }
    let prefix = if kind == ScriptKind::Sub { "_" } else { "^" };
    let alpha = !value.is_empty() && value.chars().all(|c| c.is_ascii_alphabetic());
    if value.chars().count() == 1 || (kind == ScriptKind::Sub && alpha) {
        return format!("{prefix}{value}");
    }
    format!("{prefix}({value})")
}

fn letters_numbers_dots(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| is_letter(c) || is_number(c) || c == '.')
}

pub(super) fn format_fraction(numerator: &str, denominator: &str) -> String {
    let (n, d) = (trim(numerator), trim(denominator));
    let simple_n = letters_numbers_dots(n);
    let simple_d = (!d.is_empty() && d.chars().all(|c| is_number(c) || c == '.')) || d.chars().count() == 1;
    let n = if simple_n { n.to_string() } else { format!("({n})") };
    let d = if simple_d { d.to_string() } else { format!("({d})") };
    format!("{n}/{d}")
}

pub(super) fn format_root(value: &str, symbol: &str) -> String {
    let value = trim(value);
    if letters_numbers_dots(value) { format!("{symbol}{value}") } else { format!("{symbol}({value})") }
}

/// Space named operators from neighbouring words, drop the markers, collapse spaces per line.
pub(super) fn normalize_output(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut step = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let left = i > 0 && {
            let p = chars[i - 1];
            is_letter(p) || is_number(p) || matches!(p, ')' | ']' | '}' | LAYOUT_MARKER_END)
        };
        if c == NAMED_OPERATOR_START && left {
            step.push(' ');
        } else {
            step.push(c);
        }
    }
    let chars: Vec<char> = step.chars().filter(|&c| c != NAMED_OPERATOR_START).collect();
    let mut step = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let right = chars.get(i + 1).is_some_and(|&n| is_letter(n) || is_number(n) || n == '√' || n == LAYOUT_MARKER_START);
        if c == NAMED_OPERATOR_END && right {
            step.push(' ');
        } else if c != NAMED_OPERATOR_END {
            step.push(c);
        }
    }
    let lines: Vec<String> = step
        .split('\n')
        .map(|line| {
            let mut out = String::new();
            let mut in_run = false;
            for c in line.chars() {
                if c == ' ' || c == '\t' {
                    if !in_run {
                        out.push(' ');
                    }
                    in_run = true;
                } else {
                    out.push(c);
                    in_run = false;
                }
            }
            trim(&out).to_string()
        })
        .collect();
    let n = lines.len();
    let kept: Vec<&str> =
        lines.iter().enumerate().filter(|(i, l)| !l.is_empty() || (*i > 0 && *i + 1 < n)).map(|(_, l)| l.as_str()).collect();
    trim(&kept.join("\n")).to_string()
}

pub(super) struct Layout {
    pub lines: Vec<String>,
    pub width: usize,
    pub baseline: usize,
}

fn pad_layout_line(line: &str, width: usize, centered: bool) -> String {
    let padding = width.saturating_sub(visible_width(line));
    let left = if centered { padding / 2 } else { 0 };
    format!("{}{line}{}", " ".repeat(left), " ".repeat(padding - left))
}

fn text_layout(text: String) -> Layout {
    Layout { width: visible_width(&text), lines: vec![text], baseline: 0 }
}

fn join_layouts(layouts: &[Layout]) -> Layout {
    if layouts.is_empty() {
        return Layout { lines: vec![String::new()], width: 0, baseline: 0 };
    }
    let baseline = layouts.iter().map(|l| l.baseline).max().unwrap_or(0);
    // `lines.length - baseline - 1` may be negative upstream (a script with neither part never
    // happens, so lines are never empty); keep it signed.
    let below = layouts.iter().map(|l| l.lines.len() as isize - l.baseline as isize - 1).max().unwrap_or(0);
    let mut lines = Vec::new();
    let mut row: isize = 0;
    while row <= baseline as isize + below {
        let mut line = String::new();
        for layout in layouts {
            let source_row = row - baseline as isize + layout.baseline as isize;
            if source_row >= 0 && (source_row as usize) < layout.lines.len() {
                line.push_str(&pad_layout_line(&layout.lines[source_row as usize], layout.width, false));
            } else {
                line.push_str(&" ".repeat(layout.width));
            }
        }
        lines.push(trim_end(&line).to_string());
        row += 1;
    }
    Layout { lines, width: layouts.iter().map(|l| l.width).sum(), baseline }
}

/// Matches of `/\u{f0000}(\d+)\u{f0001}/g`: (start, end, index).
fn markers(line: &str) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = line[from..].find(LAYOUT_MARKER_START) {
        let start = from + off;
        let digits_at = start + LAYOUT_MARKER_START.len_utf8();
        let digits = line[digits_at..].bytes().take_while(u8::is_ascii_digit).count();
        let close = digits_at + digits;
        if digits > 0 && line[close..].starts_with(LAYOUT_MARKER_END) {
            let end = close + LAYOUT_MARKER_END.len_utf8();
            out.push((start, end, line[digits_at..close].parse().unwrap_or(usize::MAX)));
            from = end;
        } else {
            from = digits_at;
        }
    }
    out
}

/// Index of a marker ending `text` (`TRAILING_LAYOUT_MARKER_PATTERN`).
pub(super) fn trailing_marker(text: &str) -> Option<usize> {
    let body = text.strip_suffix(LAYOUT_MARKER_END)?;
    let digits = body.bytes().rev().take_while(u8::is_ascii_digit).count();
    let (head, num) = body.split_at(body.len() - digits);
    if digits == 0 || !head.ends_with(LAYOUT_MARKER_START) {
        return None;
    }
    num.parse().ok()
}

fn starts_space(s: &str) -> bool {
    s.chars().next().is_some_and(is_space)
}

pub(super) fn render_layout(source: &str, nodes: &[Node]) -> Layout {
    let mut rendered_lines: Vec<String> = Vec::new();
    let mut first_baseline = 0;
    for source_line in source.split('\n') {
        let mut layouts = Vec::new();
        let mut position = 0;
        let mut previous: Option<&Node> = None;
        for (index, end, n) in markers(source_line) {
            let Some(node) = nodes.get(n) else { continue };
            if index > position {
                let sliced = &source_line[position..index];
                let trimmed = trim_end(if previous.is_some() { trim_start(sliced) } else { sliced });
                let lead = matches!(previous, Some(Node::Matrix { .. })) && starts_space(sliced);
                let trail = matches!(node, Node::Matrix { .. }) && sliced.chars().last().is_some_and(is_space);
                let text = if !trimmed.is_empty() {
                    format!("{}{trimmed}{}", if lead { " " } else { "" }, if trail { " " } else { "" })
                } else if lead || trail {
                    " ".to_string()
                } else {
                    String::new()
                };
                layouts.push(text_layout(text));
            }
            match node {
                Node::Fraction { numerator, denominator } => {
                    let num = render_layout(numerator, nodes);
                    let den = render_layout(denominator, nodes);
                    let content = num.width.max(den.width).max(1);
                    let width = content + 2;
                    let mut lines: Vec<String> = num.lines.iter().map(|l| pad_layout_line(l, width, true)).collect();
                    lines.push(format!(" {} ", "─".repeat(content)));
                    lines.extend(den.lines.iter().map(|l| pad_layout_line(l, width, true)));
                    layouts.push(Layout { lines, width, baseline: num.lines.len() });
                }
                Node::Operator { operator, lower, upper } => {
                    let content = visible_width(operator)
                        .max(lower.as_deref().map_or(0, visible_width))
                        .max(upper.as_deref().map_or(0, visible_width));
                    let mut lines = Vec::new();
                    if let Some(u) = upper {
                        lines.push(format!("{} ", pad_layout_line(u, content, true)));
                    }
                    lines.push(format!("{} ", pad_layout_line(operator, content, true)));
                    if let Some(l) = lower {
                        lines.push(format!("{} ", pad_layout_line(l, content, true)));
                    }
                    layouts.push(Layout { lines, width: content + 1, baseline: usize::from(upper.is_some()) });
                }
                Node::Script { lower, upper } => {
                    let up = upper.as_deref().map(|u| render_layout(u, nodes));
                    let low = lower.as_deref().map(|l| render_layout(l, nodes));
                    let width = up.as_ref().map_or(0, |l| l.width).max(low.as_ref().map_or(0, |l| l.width));
                    let mut lines: Vec<String> = Vec::new();
                    if let Some(u) = &up {
                        lines.extend(u.lines.iter().map(|l| pad_layout_line(l, width, false)));
                    }
                    lines.push(" ".repeat(width));
                    if let Some(l) = &low {
                        lines.extend(l.lines.iter().map(|l| pad_layout_line(l, width, false)));
                    }
                    layouts.push(Layout { lines, width, baseline: up.as_ref().map_or(0, |u| u.lines.len()) });
                }
                Node::Matrix { lines, baseline } => {
                    let width = lines.iter().map(|l| visible_width(l)).max().unwrap_or(0);
                    let lines = lines.iter().map(|l| pad_layout_line(l, width, false)).collect();
                    layouts.push(Layout { lines, width, baseline: *baseline });
                }
            }
            position = end;
            previous = Some(node);
        }
        if position < source_line.len() {
            let sliced = &source_line[position..];
            let trimmed = if previous.is_some() { trim_start(sliced) } else { sliced };
            let text = if matches!(previous, Some(Node::Matrix { .. })) && starts_space(sliced) {
                format!(" {trimmed}")
            } else {
                trimmed.to_string()
            };
            layouts.push(text_layout(text));
        }
        let line_layout = join_layouts(&layouts);
        if rendered_lines.is_empty() {
            first_baseline = line_layout.baseline;
        }
        rendered_lines.extend(line_layout.lines);
    }
    let width = rendered_lines.iter().map(|l| visible_width(l)).max().unwrap_or(0);
    Layout { lines: rendered_lines, width, baseline: first_baseline }
}
