//! Character classes and segmentation that upstream gets from V8 (`Intl.Segmenter`, `\p{...}`
//! regexes) and from `get-east-asian-width`. The tables come from Node itself (see
//! `parity/gen-tables.mjs`), so lookups agree with the TypeScript oracle.

use crate::unicode_tables as t;
use std::collections::HashSet;
use std::sync::OnceLock;
use unicode_segmentation::UnicodeSegmentation;

fn in_table(table: &[(u32, u32)], c: char) -> bool {
    let cp = c as u32;
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `\p{Default_Ignorable_Code_Point}|\p{Control}|\p{Mark}|\p{Surrogate}`
pub fn is_zero_width(c: char) -> bool {
    in_table(t::ZERO_WIDTH, c)
}
/// Like [`is_zero_width`] plus `\p{Format}`.
pub fn is_non_printing(c: char) -> bool {
    in_table(t::NON_PRINTING, c)
}
pub fn is_mark(c: char) -> bool {
    in_table(t::MARK, c)
}
/// Marks that terminals allocate a cell for (upstream's `terminalSpacingMarkRegex`).
pub fn is_terminal_spacing_mark(c: char) -> bool {
    in_table(t::SPACING_MARK, c)
}
/// Han, Hiragana, Katakana, Hangul or Bopomofo by script extension.
pub fn is_cjk_break(c: char) -> bool {
    in_table(t::CJK_BREAK, c)
}
pub fn is_punctuation(c: char) -> bool {
    in_table(t::PUNCTUATION, c)
}
/// JS `\s`.
pub fn is_js_whitespace(c: char) -> bool {
    in_table(t::WHITE_SPACE, c)
}
pub fn is_letter(c: char) -> bool {
    in_table(t::LETTER, c)
}
pub fn is_number(c: char) -> bool {
    in_table(t::NUMBER, c)
}
pub fn is_symbol(c: char) -> bool {
    in_table(t::SYMBOL, c)
}
pub fn is_uppercase(c: char) -> bool {
    in_table(t::UPPERCASE, c)
}
/// `eastAsianWidth(cp)` from get-east-asian-width: 2 for wide and fullwidth, else 1.
pub fn east_asian_width(c: char) -> usize {
    if in_table(t::EAST_ASIAN_WIDE, c) { 2 } else { 1 }
}

fn rgi_multi() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| t::RGI_EMOJI_MULTI.iter().copied().collect())
}

/// `/^\p{RGI_Emoji}$/v`
pub fn is_rgi_emoji(s: &str) -> bool {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (None, _) => false,
        (Some(c), None) => in_table(t::RGI_EMOJI_SINGLE, c),
        _ => rgi_multi().contains(s),
    }
}

/// Grapheme clusters, as `Intl.Segmenter(undefined, { granularity: "grapheme" })` gives them.
pub fn graphemes(s: &str) -> impl Iterator<Item = &str> {
    s.graphemes(true)
}

/// Grapheme clusters with their byte offsets.
pub fn grapheme_indices(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.grapheme_indices(true)
}

/// One segment of `Intl.Segmenter(undefined, { granularity: "word" })`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordSegment<'a> {
    pub index: usize,
    pub segment: &'a str,
    pub is_word_like: bool,
}

/// Word segments from ICU4X's segmenter, which uses the same dictionaries (CJK, Thai, ...) as
/// V8's ICU.
pub fn word_segments(s: &str) -> Vec<WordSegment<'_>> {
    use icu_segmenter::WordSegmenter;
    use icu_segmenter::options::WordBreakInvariantOptions;
    let seg = WordSegmenter::new_auto(WordBreakInvariantOptions::default());
    let mut it = seg.segment_str(s);
    let mut out = Vec::new();
    let mut prev = it.next().unwrap_or(0);
    while let Some(b) = it.next() {
        out.push(WordSegment { index: prev, segment: &s[prev..b], is_word_like: it.is_word_like() });
        prev = b;
    }
    out
}

/// UTF-16 length, for the places where upstream compares `string.length`.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}
