//! Port of `packages/tui/src/fuzzy.ts` — fuzzy matching utilities.
//!
//! Matches if all query characters appear in order (not necessarily
//! consecutive). Lower score = better match. The spec indexes UTF-16
//! units; this port walks Unicode scalar values — identical for the
//! ASCII provider/model/command names the matcher is fed.

/// Spec: `FuzzyMatch`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

const NO_MATCH: FuzzyMatch = FuzzyMatch {
    matches: false,
    score: 0.0,
};

/// Spec: the word-boundary class `[\s\-_./:]`.
fn is_word_boundary_char(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '.' | '/' | ':')
}

/// Spec: the inner `matchQuery(normalizedQuery)` closure.
fn match_query(query: &[char], text: &[char]) -> FuzzyMatch {
    if query.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }

    if query.len() > text.len() {
        return NO_MATCH;
    }

    let mut query_index = 0usize;
    let mut score = 0.0f64;
    let mut last_match_index: isize = -1;
    let mut consecutive_matches = 0u32;

    let mut i = 0usize;
    while i < text.len() && query_index < query.len() {
        if text[i] == query[query_index] {
            let is_word_boundary = i == 0 || is_word_boundary_char(text[i - 1]);

            // Reward consecutive matches
            if last_match_index == i as isize - 1 {
                consecutive_matches += 1;
                score -= f64::from(consecutive_matches) * 5.0;
            } else {
                consecutive_matches = 0;
                // Penalize gaps
                if last_match_index >= 0 {
                    score += (i as f64 - last_match_index as f64 - 1.0) * 2.0;
                }
            }

            // Reward word boundary matches
            if is_word_boundary {
                score -= 10.0;
            }

            // Slight penalty for later matches
            score += i as f64 * 0.1;

            last_match_index = i as isize;
            query_index += 1;
        }
        i += 1;
    }

    if query_index < query.len() {
        return NO_MATCH;
    }

    if query == text {
        score -= 100.0;
    }

    FuzzyMatch {
        matches: true,
        score,
    }
}

/// Spec: the `^([a-z]+)([0-9]+)$` / `^([0-9]+)([a-z]+)$` swap — returns
/// the swapped query when the (lowercased) query is exactly one letter
/// run followed by one digit run, or vice versa.
fn swapped_query(query: &[char]) -> Option<Vec<char>> {
    if query.is_empty() {
        return None;
    }
    // letters+digits → digits+letters, or digits+letters → letters+digits:
    // both swaps are tail-then-head around the run boundary.
    let index = query
        .iter()
        .position(|c| !c.is_ascii_lowercase())
        .filter(|&i| i > 0)
        .filter(|&i| query[i..].iter().all(char::is_ascii_digit))
        .or_else(|| {
            query
                .iter()
                .position(|c| !c.is_ascii_digit())
                .filter(|&i| i > 0)
                .filter(|&i| query[i..].iter().all(|c| c.is_ascii_lowercase()))
        })?;

    let (head, tail) = query.split_at(index);
    let mut swapped: Vec<char> = Vec::with_capacity(query.len());
    swapped.extend_from_slice(tail);
    swapped.extend_from_slice(head);
    Some(swapped)
}

/// Spec: `fuzzyMatch(query, text)`.
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();

    let primary_match = match_query(&query_lower, &text_lower);
    if primary_match.matches {
        return primary_match;
    }

    let Some(swapped) = swapped_query(&query_lower) else {
        return primary_match;
    };

    let swapped_match = match_query(&swapped, &text_lower);
    if !swapped_match.matches {
        return primary_match;
    }

    FuzzyMatch {
        matches: true,
        score: swapped_match.score + 5.0,
    }
}

/// Spec: `fuzzyFilter(items, query, getText)` — filter and sort items by
/// fuzzy match quality (best matches first; stable for ties). Supports
/// space-separated tokens: all tokens must match.
pub fn fuzzy_filter<T>(items: Vec<T>, query: &str, get_text: impl Fn(&T) -> String) -> Vec<T> {
    if query.trim().is_empty() {
        return items;
    }

    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return items;
    }

    let mut results: Vec<(T, f64)> = Vec::new();

    for item in items {
        let text = get_text(&item);
        let mut total_score = 0.0f64;
        let mut all_match = true;

        for token in &tokens {
            let m = fuzzy_match(token, &text);
            if m.matches {
                total_score += m.score;
            } else {
                all_match = false;
                break;
            }
        }

        if all_match {
            results.push((item, total_score));
        }
    }

    // JS `Array.prototype.sort` is stable; `sort_by` matches.
    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        let m = fuzzy_match("", "anything");
        assert!(m.matches);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn query_longer_than_text_fails() {
        assert!(!fuzzy_match("longquery", "abc").matches);
    }

    #[test]
    fn in_order_subsequence_matches() {
        assert!(fuzzy_match("cld", "claude").matches);
        assert!(!fuzzy_match("dlc", "claude").matches);
    }

    #[test]
    fn exact_match_beats_prefix_match() {
        let exact = fuzzy_match("sonnet", "sonnet");
        let partial = fuzzy_match("sonnet", "sonnet-4");
        assert!(exact.score < partial.score);
    }

    #[test]
    fn word_boundary_is_rewarded() {
        // "s" at a boundary ("claude sonnet") vs mid-word ("claudes").
        let boundary = fuzzy_match("s", "claude sonnet");
        let mid = fuzzy_match("s", "claudes");
        assert!(boundary.score < mid.score);
    }

    #[test]
    fn alphanumeric_swap_matches() {
        // Spec: "4o" ↔ "o4" style swap with +5 penalty.
        let direct = fuzzy_match("o4", "o4-mini");
        let swapped = fuzzy_match("4o", "o4-mini");
        assert!(direct.matches);
        assert!(swapped.matches);
        assert_eq!(swapped.score, direct.score + 5.0);
    }

    #[test]
    fn filter_requires_all_tokens_and_sorts_best_first() {
        let items = vec![
            "anthropic claude-opus-4-8",
            "openai gpt-5.4",
            "anthropic claude-haiku-4-5",
        ];
        let filtered = fuzzy_filter(items, "anthropic opus", |s| (*s).to_owned());
        assert_eq!(filtered, vec!["anthropic claude-opus-4-8"]);
    }

    #[test]
    fn filter_blank_query_returns_input_order() {
        let items = vec!["b", "a"];
        assert_eq!(
            fuzzy_filter(items, "  ", |s| (*s).to_owned()),
            vec!["b", "a"]
        );
    }
}
