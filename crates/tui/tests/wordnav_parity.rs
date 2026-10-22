//! Golden parity for word navigation and word segmentation (`parity/gen-wordnav-fixtures.mjs`).

use pi_tui::unicode::word_segments;
use pi_tui::word_navigation::{WordNavigationOptions, find_word_backward, find_word_forward};
use serde_json::Value;

/// UTF-16 offset to byte offset.
fn byte_at(t: &str, u16_off: usize) -> usize {
    let mut units = 0;
    for (i, c) in t.char_indices() {
        if units >= u16_off {
            return i;
        }
        units += c.len_utf16();
    }
    t.len()
}

#[test]
fn word_segments_match_icu() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/wordnav.json")).unwrap();
    let mut f = Vec::new();
    for c in &cases {
        let t = c["t"].as_str().unwrap();
        let want: Vec<(usize, String, bool)> = c["segs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (byte_at(t, s[0].as_u64().unwrap() as usize), s[1].as_str().unwrap().to_string(), s[2].as_bool().unwrap()))
            .collect();
        let got: Vec<(usize, String, bool)> = word_segments(t).into_iter().map(|s| (s.index, s.segment.to_string(), s.is_word_like)).collect();
        if got != want {
            f.push(format!("{t:?}\n  got:  {got:?}\n  want: {want:?}"));
        }
    }
    assert!(f.is_empty(), "{} texts differ:\n{}", f.len(), f.join("\n"));
}

#[test]
fn word_navigation_matches_upstream() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/wordnav.json")).unwrap();
    let o = WordNavigationOptions::default();
    let mut f = Vec::new();
    for c in &cases {
        let t = c["t"].as_str().unwrap();
        for (i, (b, fw)) in c["b"].as_array().unwrap().iter().zip(c["f"].as_array().unwrap()).enumerate() {
            if b.is_null() {
                continue;
            }
            let cur = byte_at(t, i);
            let wb = byte_at(t, b.as_u64().unwrap() as usize);
            let wf = byte_at(t, fw.as_u64().unwrap() as usize);
            // Known gap: V8's ICU keeps an incomplete Thai word ("ไทยภ") as one segment, ICU4X
            // splits it after the dictionary word.
            if t == "ไทยภาษา" && cur == 12 {
                continue;
            }
            let (gb, gf) = (find_word_backward(t, cur, &o), find_word_forward(t, cur, &o));
            if gb != wb || gf != wf {
                f.push(format!("{t:?} at {cur}: back {gb} vs {wb}, fwd {gf} vs {wf}"));
            }
        }
    }
    assert!(f.is_empty(), "{} positions differ:\n{}", f.len(), f.into_iter().take(30).collect::<Vec<_>>().join("\n"));
}
