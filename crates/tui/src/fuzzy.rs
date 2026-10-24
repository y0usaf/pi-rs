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
    let mut qi = 0;
    let mut score = 0.0f64;
    let mut last: i64 = -1;
    let mut consecutive = 0i64;
    while qi < q.len() {
        let from = (last + 1) as usize;
        let Some(off) = text
            .get(from..)
            .and_then(|t| t.iter().position(|&u| u == q[qi]))
        else {
            break;
        };
        let i = (from + off) as i64;
        let boundary = i == 0 || is_boundary(text[i as usize - 1]);
        if last == i - 1 {
            consecutive += 1;
            score -= (consecutive * 5) as f64;
        } else {
            consecutive = 0;
            if last >= 0 {
                score += ((i - last - 1) * 2) as f64;
            }
        }
        if boundary {
            score -= 10.0;
        }
        score += i as f64 * 0.1;
        last = i;
        qi += 1;
    }
    if qi < q.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
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
