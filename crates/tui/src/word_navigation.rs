//! Word-wise cursor movement (upstream `word-navigation.ts`). Positions are byte offsets.

use crate::unicode::{WordSegment, word_segments};
use crate::utils::{PUNCTUATION_CHARS, is_whitespace_char};

/// Custom segmentation and atomic segments (such as paste markers).
#[derive(Default)]
pub struct WordNavigationOptions<'a> {
    pub segment: Option<&'a dyn Fn(&str) -> Vec<WordSegment<'_>>>,
    pub is_atomic_segment: Option<&'a dyn Fn(&str) -> bool>,
}

fn segs<'t>(text: &'t str, o: &WordNavigationOptions) -> Vec<WordSegment<'t>> {
    match o.segment {
        Some(f) => f(text),
        None => word_segments(text),
    }
}

/// Cursor after moving one word back from `cursor`.
pub fn find_word_backward(text: &str, cursor: usize, o: &WordNavigationOptions) -> usize {
    if cursor == 0 {
        return 0;
    }
    let atomic = |s: &str| o.is_atomic_segment.is_some_and(|f| f(s));
    let mut segments = segs(&text[..cursor], o);
    let mut c = cursor;
    while let Some(last) = segments.last() {
        if atomic(last.segment) || !is_whitespace_char(last.segment) {
            break;
        }
        c -= last.segment.len();
        segments.pop();
    }
    let Some(last) = segments.last().copied() else { return c };
    if atomic(last.segment) {
        c -= last.segment.len();
    } else if last.is_word_like {
        let s = last.segment;
        match s.char_indices().filter(|(_, ch)| PUNCTUATION_CHARS.contains(*ch)).last() {
            None => c -= s.len(),
            Some((i, ch)) => c -= s.len() - (i + ch.len_utf8()),
        }
    } else {
        while let Some(last) = segments.last() {
            if atomic(last.segment) || last.is_word_like || is_whitespace_char(last.segment) {
                break;
            }
            c -= last.segment.len();
            segments.pop();
        }
    }
    c
}

/// Cursor after moving one word forward from `cursor`.
pub fn find_word_forward(text: &str, cursor: usize, o: &WordNavigationOptions) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    let atomic = |s: &str| o.is_atomic_segment.is_some_and(|f| f(s));
    let segments = segs(&text[cursor..], o);
    let mut it = segments.iter().peekable();
    let mut c = cursor;
    while let Some(s) = it.peek() {
        if atomic(s.segment) || !is_whitespace_char(s.segment) {
            break;
        }
        c += s.segment.len();
        it.next();
    }
    let Some(s) = it.peek().copied() else { return c };
    if atomic(s.segment) {
        c += s.segment.len();
    } else if s.is_word_like {
        c += s.segment.find(|ch| PUNCTUATION_CHARS.contains(ch)).unwrap_or(s.segment.len());
    } else {
        while let Some(s) = it.peek() {
            if atomic(s.segment) || s.is_word_like || is_whitespace_char(s.segment) {
                break;
            }
            c += s.segment.len();
            it.next();
        }
    }
    c
}
