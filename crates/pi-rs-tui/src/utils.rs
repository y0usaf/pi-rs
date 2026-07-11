//! ANSI- and Unicode-aware terminal text utilities.
use unicode_segmentation::UnicodeSegmentation;

fn ansi_len_at(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&0x1b) {
        return None;
    }
    match bytes.get(at + 1).copied() {
        Some(b'[') => bytes[at + 2..]
            .iter()
            .position(|b| matches!(b, b'm' | b'G' | b'K' | b'H' | b'J' | b'u' | b'~'))
            .map(|n| n + 3),
        Some(b']' | b'_') => {
            let mut i = at + 2;
            while i < bytes.len() {
                if bytes[i] == 7 {
                    return Some(i + 1 - at);
                }
                if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                    return Some(i + 2 - at);
                }
                i += 1;
            }
            None
        }
        _ => None,
    }
}

fn grapheme_width(grapheme: &str) -> usize {
    let Some(first) = grapheme.chars().next() else {
        return 0;
    };
    if first.is_control() {
        return 0;
    }
    // Unicode terminal width ranges used by pi's string-width dependency.
    let cp = first as u32;
    let wide = cp >= 0x1100
        && matches!(cp,
        0x1100..=0x115f | 0x231a..=0x231b | 0x2329..=0x232a |
        0x23e9..=0x23ec | 0x23f0 | 0x23f3 | 0x25fd..=0x25fe |
        0x2614..=0x2615 | 0x2648..=0x2653 | 0x267f | 0x2693 |
        0x26a1 | 0x26aa..=0x26ab | 0x26bd..=0x26be | 0x26c4..=0x26c5 |
        0x26ce | 0x26d4 | 0x26ea | 0x26f2..=0x26f3 | 0x26f5 |
        0x26fa | 0x26fd | 0x2705 | 0x270a..=0x270b | 0x2728 |
        0x274c | 0x274e | 0x2753..=0x2755 | 0x2757 | 0x2795..=0x2797 |
        0x27b0 | 0x27bf | 0x2b1b..=0x2b1c | 0x2b50 | 0x2b55 |
        0x2e80..=0xa4cf | 0xac00..=0xd7a3 | 0xf900..=0xfaff |
        0xfe10..=0xfe19 | 0xfe30..=0xfe6f | 0xff00..=0xff60 |
        0xffe0..=0xffe6 | 0x1f000..=0x1faff | 0x20000..=0x3fffd);
    if wide { 2 } else { 1 }
}
pub fn visible_width(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut clean = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(n) = ansi_len_at(bytes, i) {
            i += n;
            continue;
        }
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if ch == '\t' {
            clean.push_str("   ");
        } else {
            clean.push(ch);
        }
        i += ch.len_utf8();
    }
    clean.graphemes(true).map(grapheme_width).sum()
}

fn units(text: &str) -> Vec<(&str, usize, bool)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(n) = ansi_len_at(bytes, i) {
            out.push((&text[i..i + n], 0, true));
            i += n;
            continue;
        }
        let end = text[i..].find('\x1b').map_or(text.len(), |n| i + n);
        for g in text[i..end].graphemes(true) {
            out.push((g, if g == "\t" { 3 } else { grapheme_width(g) }, false));
        }
        i = end;
    }
    out
}

/// Spec: pi-tui `finalizeTruncatedResult` — truncated output always ends
/// in a full reset so trailing styles cannot bleed into what follows.
fn finalize_truncated(
    prefix: &str,
    prefix_width: usize,
    ellipsis: &str,
    ellipsis_width: usize,
    max: usize,
    pad: bool,
) -> String {
    let reset = "\x1b[0m";
    let mut result = if ellipsis.is_empty() {
        format!("{prefix}{reset}")
    } else {
        format!("{prefix}{reset}{ellipsis}{reset}")
    };
    if pad {
        result.push_str(&" ".repeat(max.saturating_sub(prefix_width + ellipsis_width)));
    }
    result
}

pub fn truncate_to_width(text: &str, max: usize, ellipsis: &str, pad: bool) -> String {
    if max == 0 {
        return String::new();
    }
    if text.is_empty() {
        return if pad { " ".repeat(max) } else { String::new() };
    }
    let width = visible_width(text);
    if width <= max {
        let mut s = text.to_owned();
        if pad {
            s.push_str(&" ".repeat(max - width));
        }
        return s;
    }
    let ew = visible_width(ellipsis);
    if ew >= max {
        // Spec: a too-wide ellipsis is clipped and stands alone.
        let clipped = slice_by_column(ellipsis, 0, max, true);
        let clipped_width = visible_width(&clipped);
        if clipped_width == 0 {
            return if pad { " ".repeat(max) } else { String::new() };
        }
        return finalize_truncated("", 0, &clipped, clipped_width, max, pad);
    }
    let target = max - ew;
    let mut out = String::new();
    let mut pending = String::new();
    let mut kept = 0;
    let mut keep_contiguous_prefix = true;
    for (s, w, ansi) in units(text) {
        if ansi {
            // Spec: ANSI codes flush only when a following glyph is kept;
            // codes at or past the cut are dropped.
            pending.push_str(s);
            continue;
        }
        if keep_contiguous_prefix && kept + w <= target {
            out.push_str(&pending);
            pending.clear();
            out.push_str(s);
            kept += w;
        } else {
            keep_contiguous_prefix = false;
            pending.clear();
        }
    }
    finalize_truncated(&out, kept, ellipsis, ew, max, pad)
}

pub fn slice_by_column(text: &str, start: usize, width: usize, _strict: bool) -> String {
    let end = start.saturating_add(width);
    let mut col = 0;
    let mut out = String::new();
    let mut pending = String::new();
    for (s, w, ansi) in units(text) {
        if ansi {
            pending.push_str(s);
            continue;
        }
        let include = col >= start && col + w <= end;
        if include {
            out.push_str(&pending);
            pending.clear();
            out.push_str(s);
        } else if col + w > start {
            pending.clear();
        }
        col += w;
    }
    out
}

/// Normalize text for terminal output without changing logical content
/// (pi `normalizeTerminalOutput`): decompose Thai/Lao AM vowels.
pub fn normalize_terminal_output(text: &str) -> String {
    if !text.contains('\u{0e33}') && !text.contains('\u{0eb3}') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\u{0e33}' => out.push_str("\u{0e4d}\u{0e32}"),
            '\u{0eb3}' => out.push_str("\u{0ecd}\u{0eb2}"),
            _ => out.push(ch),
        }
    }
    out
}

/// Track active ANSI SGR codes to preserve styling across line breaks
/// (pi `AnsiCodeTracker`).
#[derive(Default, Clone)]
struct AnsiCodeTracker {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg_color: Option<String>,
    bg_color: Option<String>,
    hyperlink: Option<(String, String, &'static str)>, // params, url, terminator
}

impl AnsiCodeTracker {
    fn reset(&mut self) {
        *self = AnsiCodeTracker {
            hyperlink: self.hyperlink.take(),
            ..AnsiCodeTracker::default()
        };
    }

    fn process(&mut self, code: &str) {
        // OSC 8 hyperlink open/close, preserving the original terminator.
        if let Some(body_start) = code.strip_prefix("\x1b]8;") {
            let (body, terminator) = if let Some(b) = body_start.strip_suffix('\x07') {
                (b, "\x07")
            } else if let Some(b) = body_start.strip_suffix("\x1b\\") {
                (b, "\x1b\\")
            } else {
                return;
            };
            let Some(separator) = body.find(';') else {
                return;
            };
            let params = &body[..separator];
            let url = &body[separator + 1..];
            if url.is_empty() {
                self.hyperlink = None;
            } else {
                self.hyperlink = Some((params.to_owned(), url.to_owned(), terminator));
            }
            return;
        }
        let Some(params) = code.strip_prefix("\x1b[").and_then(|c| c.strip_suffix('m')) else {
            return;
        };
        if !params.chars().all(|c| c.is_ascii_digit() || c == ';') {
            return;
        }
        if params.is_empty() || params == "0" {
            self.reset();
            return;
        }
        let parts: Vec<&str> = params.split(';').collect();
        let mut i = 0;
        while i < parts.len() {
            let code: i64 = parts[i].parse().unwrap_or(-1);
            if (code == 38 || code == 48) && parts.get(i + 1) == Some(&"5") && parts.len() > i + 2 {
                let color = format!("{};{};{}", parts[i], parts[i + 1], parts[i + 2]);
                if code == 38 {
                    self.fg_color = Some(color);
                } else {
                    self.bg_color = Some(color);
                }
                i += 3;
                continue;
            }
            if (code == 38 || code == 48) && parts.get(i + 1) == Some(&"2") && parts.len() > i + 4 {
                let color = format!(
                    "{};{};{};{};{}",
                    parts[i],
                    parts[i + 1],
                    parts[i + 2],
                    parts[i + 3],
                    parts[i + 4]
                );
                if code == 38 {
                    self.fg_color = Some(color);
                } else {
                    self.bg_color = Some(color);
                }
                i += 5;
                continue;
            }
            match code {
                0 => self.reset(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                5 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strikethrough = true,
                21 => self.bold = false,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strikethrough = false,
                39 => self.fg_color = None,
                49 => self.bg_color = None,
                30..=37 | 90..=97 => self.fg_color = Some(code.to_string()),
                40..=47 | 100..=107 => self.bg_color = Some(code.to_string()),
                _ => {}
            }
            i += 1;
        }
    }

    fn process_text(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if let Some(n) = ansi_len_at(bytes, i) {
                self.process(&text[i..i + n]);
                i += n;
            } else {
                i += text[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }

    fn active_codes(&self) -> String {
        let mut codes: Vec<String> = Vec::new();
        if self.bold {
            codes.push("1".into());
        }
        if self.dim {
            codes.push("2".into());
        }
        if self.italic {
            codes.push("3".into());
        }
        if self.underline {
            codes.push("4".into());
        }
        if self.blink {
            codes.push("5".into());
        }
        if self.inverse {
            codes.push("7".into());
        }
        if self.hidden {
            codes.push("8".into());
        }
        if self.strikethrough {
            codes.push("9".into());
        }
        if let Some(fg) = &self.fg_color {
            codes.push(fg.clone());
        }
        if let Some(bg) = &self.bg_color {
            codes.push(bg.clone());
        }
        let mut result = if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        };
        if let Some((params, url, terminator)) = &self.hyperlink {
            result.push_str(&format!("\x1b]8;{params};{url}{terminator}"));
        }
        result
    }

    fn line_end_reset(&self) -> String {
        let mut result = String::new();
        if self.underline {
            result.push_str("\x1b[24m");
        }
        if let Some((_, _, terminator)) = &self.hyperlink {
            result.push_str(&format!("\x1b]8;;{terminator}"));
        }
        result
    }
}

fn is_cjk_grapheme(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(|c| {
        let cp = c as u32;
        matches!(cp,
            0x1100..=0x11ff | 0x2e80..=0x2eff | 0x3000..=0x303f | 0x3040..=0x30ff |
            0x3100..=0x312f | 0x3130..=0x318f | 0x31a0..=0x31bf | 0x31f0..=0x31ff |
            0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xa960..=0xa97f | 0xac00..=0xd7ff |
            0xf900..=0xfaff | 0x20000..=0x3fffd)
    })
}

/// Split text into word/space/CJK tokens while keeping ANSI codes attached
/// (pi `splitIntoTokensWithAnsi`).
fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Space,
        Word,
    }
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut pending_ansi = String::new();
    let mut current_kind: Option<Kind> = None;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(n) = ansi_len_at(bytes, i) {
            pending_ansi.push_str(&text[i..i + n]);
            i += n;
            continue;
        }
        let end = text[i..].find('\x1b').map_or(text.len(), |n| i + n);
        for segment in text[i..end].graphemes(true) {
            let is_space = segment == " ";
            if !is_space && is_cjk_grapheme(segment) {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    current_kind = None;
                }
                let mut token = std::mem::take(&mut pending_ansi);
                token.push_str(segment);
                tokens.push(token);
                continue;
            }
            let kind = if is_space { Kind::Space } else { Kind::Word };
            if !current.is_empty() && current_kind != Some(kind) {
                tokens.push(std::mem::take(&mut current));
            }
            if !pending_ansi.is_empty() {
                current.push_str(&std::mem::take(&mut pending_ansi));
            }
            current_kind = Some(kind);
            current.push_str(segment);
        }
        i = end;
    }
    if !pending_ansi.is_empty() {
        if !current.is_empty() {
            current.push_str(&pending_ansi);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(&pending_ansi);
        } else {
            current = pending_ansi;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = tracker.active_codes();
    let mut current_width = 0;
    let bytes = word.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(n) = ansi_len_at(bytes, i) {
            let code = &word[i..i + n];
            current_line.push_str(code);
            tracker.process(code);
            i += n;
            continue;
        }
        let end = word[i..].find('\x1b').map_or(word.len(), |n| i + n);
        for grapheme in word[i..end].graphemes(true) {
            let w = visible_width(grapheme);
            if current_width + w > width {
                let reset = tracker.line_end_reset();
                if !reset.is_empty() {
                    current_line.push_str(&reset);
                }
                lines.push(std::mem::take(&mut current_line));
                current_line = tracker.active_codes();
                current_width = 0;
            }
            current_line.push_str(grapheme);
            current_width += w;
        }
        i = end;
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn trim_end_whitespace(line: &str) -> String {
    line.trim_end().to_owned()
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_owned()];
    }
    let mut wrapped: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::default();
    let tokens = split_into_tokens_with_ansi(line);
    let mut current_line = String::new();
    let mut current_visible = 0;
    for token in tokens {
        let token_visible = visible_width(&token);
        let is_whitespace = token.trim().is_empty();
        if token_visible > width && !is_whitespace {
            if !current_line.is_empty() {
                let reset = tracker.line_end_reset();
                if !reset.is_empty() {
                    current_line.push_str(&reset);
                }
                wrapped.push(std::mem::take(&mut current_line));
            }
            let broken = break_long_word(&token, width, &mut tracker);
            let last = broken.len() - 1;
            for line in &broken[..last] {
                wrapped.push(line.clone());
            }
            current_line = broken[last].clone();
            current_visible = visible_width(&current_line);
            continue;
        }
        let total_needed = current_visible + token_visible;
        if total_needed > width && current_visible > 0 {
            let mut line_to_wrap = trim_end_whitespace(&current_line);
            let reset = tracker.line_end_reset();
            if !reset.is_empty() {
                line_to_wrap.push_str(&reset);
            }
            wrapped.push(line_to_wrap);
            if is_whitespace {
                current_line = tracker.active_codes();
                current_visible = 0;
            } else {
                current_line = tracker.active_codes();
                current_line.push_str(&token);
                current_visible = token_visible;
            }
        } else {
            current_line.push_str(&token);
            current_visible += token_visible;
        }
        tracker.process_text(&token);
    }
    if !current_line.is_empty() {
        wrapped.push(current_line);
    }
    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped.iter().map(|l| trim_end_whitespace(l)).collect()
    }
}

/// Wrap text with ANSI codes preserved (pi `wrapTextWithAnsi`).
///
/// ONLY does word wrapping - no padding, no background colors. Active ANSI
/// codes carry across line breaks and literal newlines.
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let width = width.max(1);
    let mut result: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::default();
    for input_line in text.split('\n') {
        let prefix = if result.is_empty() {
            String::new()
        } else {
            tracker.active_codes()
        };
        let combined = format!("{prefix}{input_line}");
        for wrapped in wrap_single_line(&combined, width) {
            result.push(wrapped);
        }
        tracker.process_text(input_line);
    }
    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

/// Apply background color to a line, padding to full width
/// (pi `applyBackgroundToLine`).
pub fn apply_background_to_line(line: &str, width: usize, bg: &dyn Fn(&str) -> String) -> String {
    let visible = visible_width(line);
    let mut padded = line.to_owned();
    padded.push_str(&" ".repeat(width.saturating_sub(visible)));
    bg(&padded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ansi_wide() {
        assert_eq!(visible_width("\x1b[31m界a\x1b[0m"), 3);
        // finalizeTruncatedResult: truncated output ends in full resets.
        assert_eq!(
            truncate_to_width("abcdef", 5, "...", false),
            "ab\x1b[0m...\x1b[0m"
        );
    }
    #[test]
    fn wraps_words() {
        assert_eq!(wrap_text_with_ansi("one two", 5), ["one", "two"]);
    }
    #[test]
    fn wrap_preserves_active_styles_across_breaks() {
        let lines = wrap_text_with_ansi("\x1b[31mred red red\x1b[0m", 4);
        assert_eq!(lines, ["\x1b[31mred", "\x1b[31mred", "\x1b[31mred\x1b[0m"]);
        let multi = wrap_text_with_ansi("\x1b[1mbold\nnext\x1b[0m", 10);
        assert_eq!(multi, ["\x1b[1mbold", "\x1b[1mnext\x1b[0m"]);
    }
    #[test]
    fn normalize_decomposes_thai_am() {
        assert_eq!(normalize_terminal_output("a"), "a");
        assert_eq!(normalize_terminal_output("\u{0e33}"), "\u{0e4d}\u{0e32}");
    }
}
