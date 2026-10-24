//! Fuzzy matching (upstream `fuzzy.ts`). All query characters must appear in order; lower
//! scores are better. Positions are UTF-16 code units, as upstream scores them.

use crate::unicode::is_js_whitespace;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

fn is_boundary(u: u16) -> bool {
    match char::from_u32(u as u32) {
        Some(c) => is_js_whitespace(c) || matches!(c, '-' | '_' | '.' | '/' | ':'),
        None => false,
    }
}

fn score_positions(text: &[u16], positions: &[i64]) -> f64 {
    let mut score = 0.0f64;
    let mut last: i64 = -1;
    let mut consecutive = 0i64;
    for &i in positions {
        if last == i - 1 {
            consecutive += 1;
            score -= (consecutive * 5) as f64;
        } else {
            consecutive = 0;
            if last >= 0 {
                score += ((i - last - 1) * 2) as f64;
            }
        }
        if i == 0 || is_boundary(text[i as usize - 1]) {
            score -= 10.0;
        }
        score += i as f64 * 0.1;
        last = i;
    }
    score
}

fn find_sub(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    let rest = hay.get(from..)?;
    if needle.len() > rest.len() {
        return None;
    }
    (0..=rest.len() - needle.len())
        .find(|&i| &rest[i..i + needle.len()] == needle)
        .map(|i| from + i)
}

fn match_query(q: &[u16], text: &[u16]) -> FuzzyMatch {
    if q.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }
    if q.len() > text.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }
    let mut positions: Vec<i64> = Vec::with_capacity(q.len());
    let mut last: i64 = -1;
    for &c in q {
        let from = (last + 1) as usize;
        let Some(off) = text
            .get(from..)
            .and_then(|t| t.iter().position(|&u| u == c))
        else {
            return FuzzyMatch {
                matches: false,
                score: 0.0,
            };
        };
        last = (from + off) as i64;
        positions.push(last);
    }
    let mut score = score_positions(text, &positions);
    let mut start = find_sub(text, q, 0);
    while let Some(s) = start {
        for (k, p) in positions.iter_mut().enumerate() {
            *p = (s + k) as i64;
        }
        score = score.min(score_positions(text, &positions));
        start = find_sub(text, q, s + 1);
    }
    if q == text {
        score -= 100.0;
    }
    FuzzyMatch {
        matches: true,
        score,
    }
}

/// Swap a trailing number to the front or a leading one to the back (`abc12` <-> `12abc`).
fn swapped(q: &str) -> Option<String> {
    let letters = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase());
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let split = q.find(|c: char| c.is_ascii_digit()).unwrap_or(q.len());
    let (a, b) = q.split_at(split);
    if letters(a) && digits(b) {
        return Some(format!("{b}{a}"));
    }
    let split = q.find(|c: char| c.is_ascii_lowercase()).unwrap_or(q.len());
    let (a, b) = q.split_at(split);
    if digits(a) && letters(b) {
        return Some(format!("{b}{a}"));
    }
    None
}

/// Fuzzy-match `query` against `text`.
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let ql = query.to_lowercase();
    let tl: Vec<u16> = text.to_lowercase().encode_utf16().collect();
    let primary = match_query(&ql.encode_utf16().collect::<Vec<_>>(), &tl);
    if primary.matches {
        return primary;
    }
    let Some(sw) = swapped(&ql) else {
        return primary;
    };
    let m = match_query(&sw.encode_utf16().collect::<Vec<_>>(), &tl);
    if !m.matches {
        return primary;
    }
    FuzzyMatch {
        matches: true,
        score: m.score + 5.0,
    }
}

/// Keep items matching every whitespace- or slash-separated token, best first (stable).
pub fn fuzzy_filter<T: Clone>(items: &[T], query: &str, text: impl Fn(&T) -> String) -> Vec<T> {
    let q = crate::utils::js_trim(query);
    if q.is_empty() {
        return items.to_vec();
    }
    let tokens: Vec<&str> = q
        .split(|c: char| is_js_whitespace(c) || c == '/')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return items.to_vec();
    }
    let mut results: Vec<(T, f64)> = Vec::new();
    for item in items {
        let t = text(item);
        let mut total = 0.0;
        let mut all = true;
        for tok in &tokens {
            let m = fuzzy_match(tok, &t);
            if m.matches {
                total += m.score;
            } else {
                all = false;
                break;
            }
        }
        if all {
            results.push((item.clone(), total));
        }
    }
    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().map(|r| r.0).collect()
}
