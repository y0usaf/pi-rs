//! Width, wrapping, truncation and slicing of ANSI-styled terminal text (upstream `utils.ts`).
//!
//! Positions are byte offsets into UTF-8; widths are terminal columns.

use crate::unicode::{
    east_asian_width, graphemes, is_cjk_break, is_js_whitespace, is_mark,
    is_non_printing, is_punctuation, is_rgi_emoji, is_terminal_spacing_mark, is_zero_width,
    utf16_len,
};
use std::cell::RefCell;
use std::collections::HashMap;

fn could_be_emoji(segment: &str) -> bool {
    let cp = segment.chars().next().map_or(0, |c| c as u32);
    (0x1f000..=0x1fbff).contains(&cp)
        || (0x2300..=0x23ff).contains(&cp)
        || (0x2600..=0x27bf).contains(&cp)
        || (0x2b50..=0x2b55).contains(&cp)
        || segment.contains('\u{FE0F}')
        || utf16_len(segment) > 2
}

/// CJK punctuation that separates prose from completions (upstream `cjkPunctuationRegex`).
pub fn is_cjk_punctuation(c: char) -> bool {
    (is_punctuation(c) && is_cjk_break(c)) || "，．：；！？（）［］｛｝“”‘’…—".contains(c)
}

/// Upstream `autocompleteSeparatorRegex`: whitespace or CJK punctuation.
pub fn is_autocomplete_separator(c: char) -> bool {
    is_js_whitespace(c) || is_cjk_punctuation(c)
}

fn is_printable_ascii(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// Terminal width of one grapheme cluster.
pub fn grapheme_width(segment: &str) -> usize {
    let mut it = segment.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        let code = c as u32;
        if (0x20..=0x7e).contains(&code) {
            return 1;
        }
        if code == 0x09 {
            return 3;
        }
    }
    if segment.is_empty() {
        return 0;
    }
    if segment.chars().all(is_terminal_spacing_mark) {
        return segment.chars().count();
    }
    if segment.chars().all(is_zero_width) {
        return 0;
    }
    if could_be_emoji(segment) && is_rgi_emoji(segment) {
        return 2;
    }
    let base = segment.trim_start_matches(is_non_printing);
    let Some(first) = base.chars().next() else {
        return 0;
    };
    let cp = first as u32;
    if (0x1f1e6..=0x1f1ff).contains(&cp) {
        return 2;
    }
    let mut width = east_asian_width(first);
    let mut follows_mark = false;
    for ch in base.chars().skip(1) {
        if is_terminal_spacing_mark(ch) {
            width += 1;
            follows_mark = false;
        } else if is_mark(ch) {
            follows_mark = true;
        } else if !is_non_printing(ch) {
            let c = ch as u32;
            if follows_mark || (0xff00..=0xffef).contains(&c) {
                width += east_asian_width(ch);
            } else if c == 0x0e33 || c == 0x0eb3 {
                width += 1;
            }
            follows_mark = false;
        }
    }
    width
}

const WIDTH_CACHE_SIZE: usize = 512;

thread_local! {
    static WIDTH_CACHE: RefCell<(HashMap<String, usize>, std::collections::VecDeque<String>)> =
        RefCell::new((HashMap::new(), std::collections::VecDeque::new()));
}

/// Visible width of a string in terminal columns (tabs count 3, escape sequences 0).
pub fn visible_width(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    if let Some(w) = ascii_visible_width(s) {
        return w;
    }
    if let Some(w) = WIDTH_CACHE.with(|c| c.borrow().0.get(s).copied()) {
        return w;
    }
    let mut clean = if s.contains('\t') { s.replace('\t', "   ") } else { s.to_string() };
    if clean.contains('\x1b') {
        clean = strip_ansi_lenient(&clean);
    }
    let width = graphemes(&clean).map(grapheme_width).sum();
    WIDTH_CACHE.with(|c| {
        let (map, order) = &mut *c.borrow_mut();
        if map.len() >= WIDTH_CACHE_SIZE {
            if let Some(k) = order.pop_front() {
                map.remove(&k);
            }
        }
        if map.insert(s.to_string(), width).is_none() {
            order.push_back(s.to_string());
        }
    });
    width
}

/// Drop every recognized escape sequence, keeping lone ESC bytes (as `visibleWidth` does).
fn strip_ansi_lenient(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut copy_from = 0;
    let mut idx = s.find('\x1b');
    while let Some(i) = idx {
        let len = ansi_code_length(s, i);
        let next = if len > 0 {
            out.push_str(&s[copy_from..i]);
            copy_from = i + len;
            i + len
        } else {
            i + 1
        };
        idx = s[next..].find('\x1b').map(|j| j + next);
    }
    out.push_str(&s[copy_from..]);
    out
}

/// Remove ANSI, OSC and APC sequences while keeping visible text.
pub fn strip_terminal_sequences(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    strip_ansi_lenient(s)
}

fn ascii_visible_width(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut width = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if (0x20..=0x7e).contains(&c) {
            width += 1;
            i += 1;
        } else if c == 0x09 {
            width += 3;
            i += 1;
        } else if c == 0x1b {
            let len = ansi_code_length(s, i);
            if len == 0 {
                return None;
            }
            i += len;
        } else {
            return None;
        }
    }
    Some(width)
}

/// Length in bytes of the CSI/OSC/APC sequence starting at `pos`, or 0.
pub fn ansi_code_length(s: &str, pos: usize) -> usize {
    let b = s.as_bytes();
    if pos >= b.len() || b[pos] != 0x1b {
        return 0;
    }
    match b.get(pos + 1) {
        Some(b'[') => {
            for j in pos + 2..b.len() {
                if matches!(b[j], b'm' | b'G' | b'K' | b'H' | b'J') {
                    return j + 1 - pos;
                }
            }
            0
        }
        Some(b']') | Some(b'_') => {
            for j in pos + 2..b.len() {
                if b[j] == 0x07 {
                    return j + 1 - pos;
                }
                if b[j] == 0x1b && b.get(j + 1) == Some(&b'\\') {
                    return j + 2 - pos;
                }
            }
            0
        }
        _ => 0,
    }
}

/// The escape sequence starting at `pos`, if any.
pub fn extract_ansi_code(s: &str, pos: usize) -> Option<&str> {
    let len = ansi_code_length(s, pos);
    (len > 0).then(|| &s[pos..pos + len])
}

/// End (exclusive) of the run of plain text starting at `i`: up to the next escape sequence.
fn text_run_end(s: &str, i: usize) -> usize {
    let mut end = i;
    while end < s.len() && extract_ansi_code(s, end).is_none() {
        end += 1;
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
    }
    end
}

/// The terminal-cell range `(start, end)` of the grapheme at a visible column.
pub fn grapheme_cell_range(line: &str, column: usize) -> Option<(usize, usize)> {
    let mut col = 0;
    let mut i = 0;
    while i < line.len() {
        if let Some(code) = extract_ansi_code(line, i) {
            i += code.len();
            continue;
        }
        let end = text_run_end(line, i);
        for seg in graphemes(&line[i..end]) {
            let w = grapheme_width(seg);
            if w > 0 && column >= col && column < col + w {
                return Some((col, col + w));
            }
            col += w;
        }
        i = end;
    }
    None
}

/// The OSC 8 hyperlink URL covering a visible column.
pub fn osc8_link_at_column(line: &str, column: usize) -> Option<String> {
    let mut active: Option<String> = None;
    let mut col = 0;
    let mut i = 0;
    while i < line.len() {
        if let Some(code) = extract_ansi_code(line, i) {
            if let Some(rest) = code.strip_prefix("\x1b]8;") {
                let body = rest.strip_suffix('\x07').or_else(|| rest.strip_suffix("\x1b\\"));
                if let Some(body) = body {
                    if let Some(semi) = body.find(';') {
                        let url = &body[semi + 1..];
                        if !url.contains('\x07') && !url.contains('\x1b') {
                            active = (!url.is_empty()).then(|| url.to_string());
                        }
                    }
                }
            }
            i += code.len();
            continue;
        }
        let end = text_run_end(line, i);
        for seg in graphemes(&line[i..end]) {
            let w = if seg == "\t" { 3 } else { grapheme_width(seg) };
            if column >= col && column < col + w {
                return active;
            }
            col += w;
        }
        i = end;
    }
    None
}

/// Decompose Thai/Lao AM vowels and expand visible tabs to three spaces.
pub fn normalize_terminal_output(s: &str) -> String {
    let normalized = if s.contains(['\u{0e33}', '\u{0eb3}']) {
        s.replace('\u{0e33}', "\u{0e4d}\u{0e32}").replace('\u{0eb3}', "\u{0ecd}\u{0eb2}")
    } else {
        s.to_string()
    };
    if !normalized.contains('\t') {
        return normalized;
    }
    let mut out = String::with_capacity(normalized.len() + 8);
    let mut i = 0;
    while i < normalized.len() {
        if let Some(code) = extract_ansi_code(&normalized, i) {
            out.push_str(code);
            i += code.len();
            continue;
        }
        let c = normalized[i..].chars().next().unwrap();
        if c == '\t' {
            out.push_str("   ");
        } else {
            out.push(c);
        }
        i += c.len_utf8();
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hyperlink {
    params: String,
    url: String,
    /// BEL or ST.
    terminator: &'static str,
}

/// `None`: not OSC 8. `Some(None)`: a close. `Some(Some(link))`: an open.
fn parse_osc8(code: &str) -> Option<Option<Hyperlink>> {
    if !code.starts_with("\x1b]8;") {
        return None;
    }
    let terminator = if code.ends_with('\x07') { "\x07" } else { "\x1b\\" };
    let end = code.len().saturating_sub(terminator.len()).max(4);
    let body = &code[4..end];
    let semi = body.find(';')?;
    let url = &body[semi + 1..];
    if url.is_empty() {
        return Some(None);
    }
    Some(Some(Hyperlink { params: body[..semi].to_string(), url: url.to_string(), terminator }))
}

fn format_osc8(h: &Hyperlink) -> String {
    format!("\x1b]8;{};{}{}", h.params, h.url, h.terminator)
}

fn format_osc8_close(terminator: &str) -> String {
    format!("\x1b]8;;{terminator}")
}

fn active_osc8_close(prefix: &str) -> String {
    if !prefix.contains("\x1b]8;") {
        return String::new();
    }
    let mut active: Option<Hyperlink> = None;
    let mut i = 0;
    while i < prefix.len() {
        if let Some(code) = extract_ansi_code(prefix, i) {
            if let Some(h) = parse_osc8(code) {
                active = h;
            }
            i += code.len();
        } else {
            i += 1;
        }
    }
    active.map(|h| format_osc8_close(h.terminator)).unwrap_or_default()
}

/// First match of `/\x1b\[([\d;]*)m/` in the code.
fn sgr_params(code: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(p) = code[from..].find("\x1b\x5b") {
        let start = from + p + 2;
        let rest = &code[start..];
        let n = rest.bytes().take_while(|b| b.is_ascii_digit() || *b == b';').count();
        if rest.as_bytes().get(n) == Some(&b'm') {
            return Some(&rest[..n]);
        }
        from = from + p + 1;
    }
    None
}

/// JS `Number.parseInt(s, 10)`, NaN as `None`.
fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let n: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if n.is_empty() {
        return None;
    }
    let v: i64 = n.parse().unwrap_or(i64::MAX);
    Some(if neg { -v } else { v })
}

/// Tracks active SGR attributes and the open OSC 8 link, to carry styling across line breaks.
#[derive(Debug, Default, Clone)]
pub(crate) struct AnsiCodeTracker {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg: Option<String>,
    bg: Option<String>,
    link: Option<Hyperlink>,
}

impl AnsiCodeTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn process(&mut self, code: &str) {
        if let Some(h) = parse_osc8(code) {
            self.link = h;
            return;
        }
        if !code.ends_with('m') {
            return;
        }
        let Some(params) = sgr_params(code) else { return };
        if params.is_empty() || params == "0" {
            self.reset();
            return;
        }
        let parts: Vec<&str> = params.split(';').collect();
        let mut i = 0;
        while i < parts.len() {
            let code = parse_int(parts[i]);
            if code == Some(38) || code == Some(48) {
                if parts.get(i + 1) == Some(&"5") && i + 2 < parts.len() {
                    let color = format!("{};{};{}", parts[i], parts[i + 1], parts[i + 2]);
                    if code == Some(38) { self.fg = Some(color) } else { self.bg = Some(color) }
                    i += 3;
                    continue;
                } else if parts.get(i + 1) == Some(&"2") && i + 4 < parts.len() {
                    let color = parts[i..i + 5].join(";");
                    if code == Some(38) { self.fg = Some(color) } else { self.bg = Some(color) }
                    i += 5;
                    continue;
                }
            }
            match code {
                Some(0) => self.reset(),
                Some(1) => self.bold = true,
                Some(2) => self.dim = true,
                Some(3) => self.italic = true,
                Some(4) => self.underline = true,
                Some(5) => self.blink = true,
                Some(7) => self.inverse = true,
                Some(8) => self.hidden = true,
                Some(9) => self.strikethrough = true,
                Some(21) => self.bold = false,
                Some(22) => {
                    self.bold = false;
                    self.dim = false;
                }
                Some(23) => self.italic = false,
                Some(24) => self.underline = false,
                Some(25) => self.blink = false,
                Some(27) => self.inverse = false,
                Some(28) => self.hidden = false,
                Some(29) => self.strikethrough = false,
                Some(39) => self.fg = None,
                Some(49) => self.bg = None,
                Some(c) if (30..=37).contains(&c) || (90..=97).contains(&c) => {
                    self.fg = Some(c.to_string())
                }
                Some(c) if (40..=47).contains(&c) || (100..=107).contains(&c) => {
                    self.bg = Some(c.to_string())
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn reset(&mut self) {
        let link = self.link.take();
        *self = Self { link, ..Self::default() };
    }

    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn active_codes(&self) -> String {
        let mut codes: Vec<&str> = Vec::new();
        for (on, c) in [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.blink, "5"),
            (self.inverse, "7"),
            (self.hidden, "8"),
            (self.strikethrough, "9"),
        ] {
            if on {
                codes.push(c);
            }
        }
        if let Some(fg) = self.fg.as_deref().filter(|s| !s.is_empty()) {
            codes.push(fg);
        }
        if let Some(bg) = self.bg.as_deref().filter(|s| !s.is_empty()) {
            codes.push(bg);
        }
        let mut out = if codes.is_empty() { String::new() } else { format!("\x1b\x5b{}m", codes.join(";")) };
        if let Some(h) = &self.link {
            out.push_str(&format_osc8(h));
        }
        out
    }

    pub(crate) fn active_background_code(&self) -> String {
        match self.bg.as_deref() {
            Some(bg) if !bg.is_empty() => format!("\x1b\x5b{bg}m"),
            _ => String::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn has_active_codes(&self) -> bool {
        self.bold
            || self.dim
            || self.italic
            || self.underline
            || self.blink
            || self.inverse
            || self.hidden
            || self.strikethrough
            || self.fg.is_some()
            || self.bg.is_some()
            || self.link.is_some()
    }

    /// Codes to close at line end: underline (so it does not bleed into padding) and open links.
    pub(crate) fn line_end_reset(&self) -> String {
        let mut out = String::new();
        if self.underline {
            out.push_str("\x1b\x5b24m");
        }
        if let Some(h) = &self.link {
            out.push_str(&format_osc8_close(h.terminator));
        }
        out
    }
}

pub(crate) fn update_tracker_from_text(text: &str, tracker: &mut AnsiCodeTracker) {
    let mut idx = text.find('\x1b');
    while let Some(i) = idx {
        let len = ansi_code_length(text, i);
        let next = if len > 0 {
            tracker.process(&text[i..i + len]);
            i + len
        } else {
            i + 1
        };
        idx = text[next..].find('\x1b').map(|j| j + next);
    }
}

/// The background color active at the end of an ANSI-styled string.
pub fn active_background_ansi(text: &str) -> String {
    let mut t = AnsiCodeTracker::new();
    update_tracker_from_text(text, &mut t);
    t.active_background_code()
}

fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Space,
        Word,
    }
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut pending = String::new();
    let mut kind: Option<Kind> = None;
    let mut i = 0;
    while i < text.len() {
        if let Some(code) = extract_ansi_code(text, i) {
            pending.push_str(code);
            i += code.len();
            continue;
        }
        let after = i + text[i..].chars().next().map_or(1, char::len_utf8);
        let mut end = text[after..].find('\x1b').map(|j| j + after);
        while let Some(e) = end {
            if ansi_code_length(text, e) != 0 {
                break;
            }
            end = text[e + 1..].find('\x1b').map(|j| j + e + 1);
        }
        let end = end.unwrap_or(text.len());
        let chunk = &text[i..end];
        let ascii = is_printable_ascii(chunk);
        for segment in graphemes(chunk) {
            let is_space = segment == " ";
            if !ascii && !is_space && segment.chars().any(is_cjk_break) {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    kind = None;
                }
                tokens.push(std::mem::take(&mut pending) + segment);
                continue;
            }
            let seg_kind = if is_space { Kind::Space } else { Kind::Word };
            if !current.is_empty() && kind != Some(seg_kind) {
                tokens.push(std::mem::take(&mut current));
            }
            current.push_str(&pending);
            pending.clear();
            kind = Some(seg_kind);
            current.push_str(segment);
        }
        i = end;
    }
    if !pending.is_empty() {
        if !current.is_empty() {
            current.push_str(&pending);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(&pending);
        } else {
            current = pending;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// JS `String.prototype.trimEnd`.
pub(crate) fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| is_js_whitespace(c) || matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{FEFF}'))
}

/// JS `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    let f = |c: char| is_js_whitespace(c) || matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{FEFF}');
    s.trim_matches(f)
}

/// Split on CRLF, CR or LF.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' {
            out.push(&text[start..i]);
            i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
            start = i;
        } else if b[i] == b'\n' {
            out.push(&text[start..i]);
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    out.push(&text[start..]);
    out
}

/// Word-wrap ANSI-styled text to `width` columns, carrying active styles across breaks.
/// No padding; lines may be shorter than `width`.
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut result: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();
    for input in split_lines(text) {
        let prefix = if result.is_empty() { String::new() } else { tracker.active_codes() };
        result.extend(wrap_single_line(&(prefix + input), width));
        update_tracker_from_text(input, &mut tracker);
    }
    if result.is_empty() { vec![String::new()] } else { result }
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }
    let mut wrapped: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for token in split_into_tokens_with_ansi(line) {
        let token_len = visible_width(&token);
        let is_ws = js_trim(&token).is_empty();
        if token_len > width && !is_ws {
            if !current.is_empty() {
                current.push_str(&tracker.line_end_reset());
                wrapped.push(std::mem::take(&mut current));
            }
            let mut broken = break_long_word(&token, width, &mut tracker);
            current = broken.pop().unwrap_or_default();
            wrapped.extend(broken);
            current_len = visible_width(&current);
            continue;
        }
        if current_len + token_len > width && current_len > 0 {
            let mut line_to_wrap = js_trim_end(&current).to_string();
            line_to_wrap.push_str(&tracker.line_end_reset());
            wrapped.push(line_to_wrap);
            if is_ws {
                current = tracker.active_codes();
                current_len = 0;
            } else {
                current = tracker.active_codes() + &token;
                current_len = token_len;
            }
        } else {
            current.push_str(&token);
            current_len += token_len;
        }
        update_tracker_from_text(&token, &mut tracker);
    }
    if !current.is_empty() {
        wrapped.push(current);
    }
    if wrapped.is_empty() {
        return vec![String::new()];
    }
    wrapped.iter().map(|l| js_trim_end(l).to_string()).collect()
}

/// Upstream `PUNCTUATION_REGEX`.
pub const PUNCTUATION_CHARS: &str = "(){}\x5b\x5d<>.,;:'\x22!?+-=*/\x5c|&%^$\x23@~\x60";
/// JS `/\s/` on one character.
pub fn is_whitespace_char(s: &str) -> bool {
    s.chars().any(is_js_whitespace)
}

/// Upstream `isPunctuationChar`.
pub fn is_punctuation_char(s: &str) -> bool {
    s.chars().any(|c| PUNCTUATION_CHARS.contains(c))
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = tracker.active_codes();
    let mut current_width = 0;
    let mut i = 0;
    while i < word.len() {
        if let Some(code) = extract_ansi_code(word, i) {
            current.push_str(code);
            tracker.process(code);
            i += code.len();
            continue;
        }
        let end = text_run_end(word, i);
        for g in graphemes(&word[i..end]) {
            let w = visible_width(g);
            if current_width + w > width {
                current.push_str(&tracker.line_end_reset());
                lines.push(std::mem::replace(&mut current, tracker.active_codes()));
                current_width = 0;
            }
            current.push_str(g);
            current_width += w;
        }
        i = end;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() { vec![String::new()] } else { lines }
}

/// Pad `line` to `width` and pass it through `bg`.
pub fn apply_background_to_line(line: &str, width: usize, bg: &dyn Fn(&str) -> String) -> String {
    let pad = width.saturating_sub(visible_width(line));
    bg(&format!("{line}{}", " ".repeat(pad)))
}

/// End of the plain-text run at `i`: stops at an escape sequence or a tab.
fn text_run_end_tab(s: &str, i: usize) -> usize {
    let b = s.as_bytes();
    let mut end = i;
    while end < s.len() && b[end] != b'\t' && extract_ansi_code(s, end).is_none() {
        end += 1;
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
    }
    end
}

fn truncate_fragment_to_width(text: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }
    if is_printable_ascii(text) {
        let clipped = &text[..text.len().min(max_width)];
        return (clipped.to_string(), clipped.len());
    }
    let has_ansi = text.contains('\x1b');
    let has_tabs = text.contains('\t');
    let mut result = String::new();
    let mut width = 0;
    if !has_ansi && !has_tabs {
        for seg in graphemes(text) {
            let w = grapheme_width(seg);
            if width + w > max_width {
                break;
            }
            result.push_str(seg);
            width += w;
        }
        return (result, width);
    }
    let mut pending = String::new();
    let mut i = 0;
    while i < text.len() {
        if let Some(code) = extract_ansi_code(text, i) {
            pending.push_str(code);
            i += code.len();
            continue;
        }
        if text.as_bytes()[i] == b'\t' {
            if width + 3 > max_width {
                break;
            }
            result.push_str(&std::mem::take(&mut pending));
            result.push('\t');
            width += 3;
            i += 1;
            continue;
        }
        let end = text_run_end_tab(text, i);
        for seg in graphemes(&text[i..end]) {
            let w = grapheme_width(seg);
            if width + w > max_width {
                return (result, width);
            }
            result.push_str(&std::mem::take(&mut pending));
            result.push_str(seg);
            width += w;
        }
        i = end;
    }
    (result, width)
}

fn finalize_truncated(prefix: &str, prefix_width: usize, ellipsis: &str, ellipsis_width: usize, max_width: usize, pad: bool) -> String {
    let reset = "\x1b\x5b0m";
    let close = active_osc8_close(prefix);
    let mut out = if ellipsis.is_empty() {
        format!("{prefix}{close}{reset}")
    } else {
        format!("{prefix}{close}{reset}{ellipsis}{reset}")
    };
    if pad {
        out.push_str(&" ".repeat(max_width.saturating_sub(prefix_width + ellipsis_width)));
    }
    out
}

/// Truncate to `max_width` columns, ending with `ellipsis` when cut; with `pad`, fill to exactly
/// `max_width` with spaces. Upstream's defaults are `"..."` and `false`.
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str, pad: bool) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.is_empty() {
        return if pad { " ".repeat(max_width) } else { String::new() };
    }
    let ellipsis_width = visible_width(ellipsis);
    if ellipsis_width >= max_width {
        let tw = visible_width(text);
        if tw <= max_width {
            return if pad { format!("{text}{}", " ".repeat(max_width - tw)) } else { text.to_string() };
        }
        let (clipped, cw) = truncate_fragment_to_width(ellipsis, max_width);
        if cw == 0 {
            return if pad { " ".repeat(max_width) } else { String::new() };
        }
        return finalize_truncated("", 0, &clipped, cw, max_width, pad);
    }
    if is_printable_ascii(text) {
        if text.len() <= max_width {
            return if pad { format!("{text}{}", " ".repeat(max_width - text.len())) } else { text.to_string() };
        }
        let target = max_width - ellipsis_width;
        return finalize_truncated(&text[..target], target, ellipsis, ellipsis_width, max_width, pad);
    }
    let target = max_width - ellipsis_width;
    let mut result = String::new();
    let mut pending = String::new();
    let mut so_far = 0;
    let mut kept = 0;
    let mut keep_prefix = true;
    let mut overflowed = false;
    let exhausted;
    if !text.contains('\x1b') && !text.contains('\t') {
        for seg in graphemes(text) {
            let w = grapheme_width(seg);
            if keep_prefix && kept + w <= target {
                result.push_str(seg);
                kept += w;
            } else {
                keep_prefix = false;
            }
            so_far += w;
            if so_far > max_width {
                overflowed = true;
                break;
            }
        }
        exhausted = !overflowed;
    } else {
        let mut i = 0;
        'outer: while i < text.len() {
            if let Some(code) = extract_ansi_code(text, i) {
                pending.push_str(code);
                i += code.len();
                continue;
            }
            if text.as_bytes()[i] == b'\t' {
                if keep_prefix && kept + 3 <= target {
                    result.push_str(&std::mem::take(&mut pending));
                    result.push('\t');
                    kept += 3;
                } else {
                    keep_prefix = false;
                    pending.clear();
                }
                so_far += 3;
                if so_far > max_width {
                    overflowed = true;
                    break;
                }
                i += 1;
                continue;
            }
            let end = text_run_end_tab(text, i);
            for seg in graphemes(&text[i..end]) {
                let w = grapheme_width(seg);
                if keep_prefix && kept + w <= target {
                    result.push_str(&std::mem::take(&mut pending));
                    result.push_str(seg);
                    kept += w;
                } else {
                    keep_prefix = false;
                    pending.clear();
                }
                so_far += w;
                if so_far > max_width {
                    overflowed = true;
                    break 'outer;
                }
            }
            i = end;
        }
        exhausted = i >= text.len();
    }
    if !overflowed && exhausted {
        return if pad { format!("{text}{}", " ".repeat(max_width.saturating_sub(so_far))) } else { text.to_string() };
    }
    finalize_truncated(&result, kept, ellipsis, ellipsis_width, max_width, pad)
}

/// Visible columns `start_col..start_col + length` of a line, keeping ANSI codes. With `strict`,
/// a wide character straddling the end is dropped.
pub fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    slice_with_width(line, start_col, length, strict).0
}

/// Like [`slice_by_column`], also returning the visible width of the result.
pub fn slice_with_width(line: &str, start_col: usize, length: usize, strict: bool) -> (String, usize) {
    if length == 0 {
        return (String::new(), 0);
    }
    let end_col = start_col + length;
    let mut result = String::new();
    let mut result_width = 0;
    let mut col = 0;
    let mut i = 0;
    let mut pending = String::new();
    while i < line.len() {
        if let Some(code) = extract_ansi_code(line, i) {
            if col >= start_col && col < end_col {
                result.push_str(&std::mem::take(&mut pending));
                result.push_str(code);
            } else if col < start_col {
                pending.push_str(code);
            }
            i += code.len();
            continue;
        }
        let end = text_run_end(line, i);
        for seg in graphemes(&line[i..end]) {
            let w = grapheme_width(seg);
            let in_range = col >= start_col && col < end_col;
            let fits = !strict || col + w <= end_col;
            if in_range && fits {
                result.push_str(&std::mem::take(&mut pending));
                result.push_str(seg);
                result_width += w;
            }
            col += w;
            if col >= end_col {
                break;
            }
        }
        i = end;
        if col >= end_col {
            break;
        }
    }
    (result, result_width)
}

/// The parts of a line before and after an overlay region, in one pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Segments {
    pub before: String,
    pub before_width: usize,
    pub after: String,
    pub after_width: usize,
}

/// Extract the text before column `before_end` and the `after_len` columns from `after_start`,
/// the latter inheriting the styling active where it starts.
pub fn extract_segments(line: &str, before_end: usize, after_start: usize, after_len: usize, strict_after: bool) -> Segments {
    let mut s = Segments::default();
    let mut col = 0;
    let mut i = 0;
    let mut pending_before = String::new();
    let mut after_started = false;
    let after_end = after_start + after_len;
    let mut tracker = AnsiCodeTracker::new();
    let done = |col: usize| if after_len == 0 { col >= before_end } else { col >= after_end };
    while i < line.len() {
        if let Some(code) = extract_ansi_code(line, i) {
            tracker.process(code);
            if col < before_end {
                pending_before.push_str(code);
            } else if col >= after_start && col < after_end && (after_started || code.starts_with("\x1b_")) {
                s.after.push_str(code);
            }
            i += code.len();
            continue;
        }
        let end = text_run_end(line, i);
        for seg in graphemes(&line[i..end]) {
            let w = grapheme_width(seg);
            if col < before_end && col + w <= before_end {
                s.before.push_str(&std::mem::take(&mut pending_before));
                s.before.push_str(seg);
                s.before_width += w;
            } else if col >= after_start && col < after_end {
                if !strict_after || col + w <= after_end {
                    if !after_started {
                        s.after.push_str(&tracker.active_codes());
                        after_started = true;
                    }
                    s.after.push_str(seg);
                    s.after_width += w;
                }
            }
            col += w;
            if done(col) {
                break;
            }
        }
        i = end;
        if done(col) {
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width("\x1b\x5b31mred\x1b\x5b0m"), 3);
        assert_eq!(visible_width("日本"), 4);
        assert_eq!(visible_width("a\tb"), 5);
        assert_eq!(visible_width("👍🏽"), 2);
    }

    #[test]
    fn truncates() {
        assert_eq!(truncate_to_width("hello world", 8, "...", false), "hello\x1b\x5b0m...\x1b\x5b0m");
        assert_eq!(truncate_to_width("hi", 5, "...", true), "hi   ");
    }

    #[test]
    fn wraps() {
        assert_eq!(wrap_text_with_ansi("hello world foo", 11), vec!["hello world", "foo"]);
    }
}
