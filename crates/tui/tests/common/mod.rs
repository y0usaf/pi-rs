//! Shared helpers for parity tests.
#![allow(dead_code)]

use serde_json::Value;

/// UTF-16 offset to byte offset.
pub fn byte_at(t: &str, u16_off: usize) -> usize {
    let mut units = 0;
    for (i, c) in t.char_indices() {
        if units >= u16_off {
            return i;
        }
        units += c.len_utf16();
    }
    t.len()
}

/// Whether our word segmentation of `text` equals V8's (recorded as `[[utf16 index, segment, isWordLike]]`).
/// ICU dictionaries differ slightly between V8 and ICU4X for some CJK runs; scripts that hit such
/// a run stop being comparable from there on.
pub fn segments_agree(text: &str, v8: &Value) -> bool {
    let want: Vec<(usize, &str, bool)> = v8
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                byte_at(text, s[0].as_u64().unwrap() as usize),
                s[1].as_str().unwrap(),
                s[2].as_bool().unwrap(),
            )
        })
        .collect();
    let got: Vec<(usize, &str, bool)> = pi_tui::unicode::word_segments(text)
        .into_iter()
        .map(|s| (s.index, s.segment, s.is_word_like))
        .collect();
    got == want
}
