//! Text helpers for the editor (upstream `components/editor.ts`, top of file): paste markers,
//! word wrapping, autocomplete trigger patterns and the scroll border. Offsets are bytes.

use crate::unicode::{WordSegment, grapheme_indices, is_cjk_break, utf16_len};
use crate::utils::{is_autocomplete_separator, is_whitespace_char, slice_by_column, visible_width};

const MARKER_OPEN: &str = "\x5bpaste \x23";
const MARKER_CLOSE: char = '\x5d';
const QUOTE: char = '\u{22}';

/// Length in bytes of a paste marker (`[paste #N]`, `[paste #N +L lines]` or `[paste #N C chars]`)
/// starting at byte `i`, with its id.
pub fn paste_marker_at(text: &str, i: usize) -> Option<(usize, u64)> {
    let rest = text.get(i..)?;
    let after = rest.strip_prefix(MARKER_OPEN)?;
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let n = digits(after);
    if n == 0 {
        return None;
    }
    let id: u64 = after[..n].parse().unwrap_or(u64::MAX);
    let tail = &after[n..];
    let head = MARKER_OPEN.len() + n;
    if tail.starts_with(MARKER_CLOSE) {
        return Some((head + 1, id));
    }
    // Optional " +L lines" or " C chars", then the closing bracket.
    let lines_end = " lines\x5d";
    let chars_end = " chars\x5d";
    if let Some(t) = tail.strip_prefix(' ') {
        if let Some(t2) = t.strip_prefix('+') {
            let m = digits(t2);
            if m > 0 && t2[m..].starts_with(lines_end) {
                return Some((head + 2 + m + lines_end.len(), id));
            }
        }
        let m = digits(t);
        if m > 0 && t[m..].starts_with(chars_end) {
            return Some((head + 1 + m + chars_end.len(), id));
        }
    }
    None
}

/// All paste markers, as the global regex finds them: (start, end, id).
pub fn paste_markers(text: &str) -> Vec<(usize, usize, u64)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find(MARKER_OPEN) {
        let s = i + off;
        if let Some((len, id)) = paste_marker_at(text, s) {
            out.push((s, s + len, id));
            i = s + len;
        } else {
            i = s + 1;
        }
    }
    out
}

/// Upstream `isPasteMarker`: the whole segment is one marker.
pub fn is_paste_marker(segment: &str) -> bool {
    utf16_len(segment) >= 10 && paste_marker_at(segment, 0).is_some_and(|(len, _)| len == segment.len())
}

/// Upstream `segmentWithMarkers`: merges the segments inside markers with valid ids into one.
pub fn segment_with_markers<'a>(
    text: &'a str,
    base: Vec<WordSegment<'a>>,
    valid: &dyn Fn(u64) -> bool,
    any_valid: bool,
) -> Vec<WordSegment<'a>> {
    if !any_valid || !text.contains(MARKER_OPEN) {
        return base;
    }
    let markers: Vec<(usize, usize)> = paste_markers(text).into_iter().filter(|m| valid(m.2)).map(|m| (m.0, m.1)).collect();
    if markers.is_empty() {
        return base;
    }
    let mut out = Vec::new();
    let mut mi = 0;
    for seg in base {
        while mi < markers.len() && markers[mi].1 <= seg.index {
            mi += 1;
        }
        match markers.get(mi) {
            Some(&(s, e)) if seg.index >= s && seg.index < e => {
                if seg.index == s {
                    out.push(WordSegment { index: s, segment: &text[s..e], is_word_like: false });
                }
            }
            _ => out.push(seg),
        }
    }
    out
}

/// Grapheme segments in the `WordSegment` shape.
pub fn grapheme_segments(text: &str) -> Vec<WordSegment<'_>> {
    grapheme_indices(text).map(|(index, segment)| WordSegment { index, segment, is_word_like: false }).collect()
}

/// A piece of a wrapped line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub text: String,
    pub start_index: usize,
    pub end_index: usize,
}

fn chunk(line: &str, s: usize, e: usize) -> TextChunk {
    TextChunk { text: line[s..e].to_string(), start_index: s, end_index: e }
}

fn is_cjk(s: &str) -> bool {
    s.chars().any(is_cjk_break)
}

/// Upstream `wordWrapLine`.
pub fn word_wrap_line(line: &str, max_width: usize, pre: Option<&[WordSegment]>) -> Vec<TextChunk> {
    if line.is_empty() || max_width == 0 {
        return vec![TextChunk { text: String::new(), start_index: 0, end_index: 0 }];
    }
    if visible_width(line) <= max_width {
        return vec![chunk(line, 0, line.len())];
    }
    let owned;
    let segments: &[WordSegment] = match pre {
        Some(p) => p,
        None => {
            owned = grapheme_segments(line);
            &owned
        }
    };
    let mut chunks = Vec::new();
    let mut current_width = 0usize;
    let mut chunk_start = 0usize;
    let mut wrap_opp: Option<usize> = None;
    let mut wrap_opp_width = 0usize;
    for (i, seg) in segments.iter().enumerate() {
        let g = seg.segment;
        let gw = visible_width(g);
        let ci = seg.index;
        let is_ws = !is_paste_marker(g) && is_whitespace_char(g);
        if current_width + gw > max_width {
            match wrap_opp {
                Some(w) if current_width - wrap_opp_width + gw <= max_width => {
                    chunks.push(chunk(line, chunk_start, w));
                    chunk_start = w;
                    current_width -= wrap_opp_width;
                }
                _ if chunk_start < ci => {
                    chunks.push(chunk(line, chunk_start, ci));
                    chunk_start = ci;
                    current_width = 0;
                }
                _ => {}
            }
            wrap_opp = None;
        }
        if gw > max_width {
            let sub = word_wrap_line(g, max_width, None);
            for sc in &sub[..sub.len() - 1] {
                chunks.push(TextChunk { text: sc.text.clone(), start_index: ci + sc.start_index, end_index: ci + sc.end_index });
            }
            let last = &sub[sub.len() - 1];
            chunk_start = ci + last.start_index;
            current_width = visible_width(&last.text);
            wrap_opp = None;
            continue;
        }
        current_width += gw;
        if let Some(next) = segments.get(i + 1) {
            let n = next.segment;
            if is_ws && (is_paste_marker(n) || !is_whitespace_char(n)) {
                wrap_opp = Some(next.index);
                wrap_opp_width = current_width;
            } else if !is_ws && !is_whitespace_char(n) {
                let a = !is_paste_marker(g) && is_cjk(g);
                let b = !is_paste_marker(n) && is_cjk(n);
                if a || b {
                    wrap_opp = Some(next.index);
                    wrap_opp_width = current_width;
                }
            }
        }
    }
    chunks.push(chunk(line, chunk_start, line.len()));
    chunks
}

pub const DEFAULT_TRIGGER_CHARACTERS: [&str; 2] = ["@", "\x23"];
/// Characters that may wrap a trigger token in prose.
const TOKEN_OPENERS: &str = "\x28\x5b\x7b<\x60";

/// Upstream `buildTriggerPattern(...).test(text)` (`debounce == false`) and
/// `buildDebouncePattern(...).test(text)` (`debounce == true`), on the text before the cursor.
pub fn matches_trigger(text: &str, triggers: &[String], debounce: bool) -> bool {
    let is_trigger = |c: char| triggers.iter().any(|t| t.chars().count() == 1 && t.starts_with(c));
    let unquoted = |s: &str| !s.chars().any(is_autocomplete_separator);
    let quoted = |s: &str| !s.contains(QUOTE);
    // Token starts: the beginning, or just after a separator.
    let mut starts = vec![0];
    for (i, c) in text.char_indices() {
        if is_autocomplete_separator(c) {
            starts.push(i + c.len_utf8());
        }
    }
    for s in starts {
        let mut p = s;
        loop {
            let rest = &text[p..];
            let Some(c) = rest.chars().next() else { break };
            let after = &rest[c.len_utf8()..];
            let ok = if debounce {
                (c == '@' && (after.strip_prefix(QUOTE).is_some_and(quoted) || unquoted(after)))
                    || (c != '@' && is_trigger(c) && unquoted(after))
            } else {
                (c == '@' && after.strip_prefix(QUOTE).is_some_and(quoted)) || (is_trigger(c) && unquoted(after))
            };
            if ok {
                return true;
            }
            if TOKEN_OPENERS.contains(c) {
                p += c.len_utf8();
            } else {
                break;
            }
        }
    }
    false
}

/// Upstream `createScrollBorder`.
pub fn create_scroll_border(direction: &str, hidden: usize, width: usize) -> String {
    let label = format!(" {direction} {hidden} more ");
    let lw = visible_width(&label);
    if lw + 2 <= width {
        let left = (width - lw) / 2;
        return "─".repeat(left) + &label + &"─".repeat(width - left - lw);
    }
    let indicator = format!("─── {direction} {hidden} more ");
    let iw = visible_width(&indicator);
    if width >= iw {
        return indicator + &"─".repeat(width - iw);
    }
    let ellipsis = &"..."[..width.min(3)];
    slice_by_column(&indicator, 0, width - ellipsis.len(), true) + ellipsis
}
