//! Ranking cases from upstream fuzzy.test.ts (#11220).

use pi_tui::fuzzy::{fuzzy_filter, fuzzy_match};

#[test]
fn scores_a_contiguous_occurrence_instead_of_an_earlier_scattered_alignment() {
    let opus = fuzzy_match("opus", "openrouter openrouter/anthropic/claude-opus-4.1");
    let sonnet = fuzzy_match("opus", "openrouter openrouter/anthropic/claude-sonnet-4.5");
    let gpt_oss = fuzzy_match("opus", "openrouter openrouter/openai/gpt-oss-120b");

    assert!(opus.score < sonnet.score);
    assert!(opus.score < gpt_oss.score);
}

#[test]
fn uses_the_best_contiguous_occurrence_not_the_first() {
    let embedded_first = fuzzy_match("opus", "xxopus opus");
    let boundary_only = fuzzy_match("opus", "xxxxxx opus");

    assert_eq!(embedded_first.score, boundary_only.score);
}

#[test]
fn ranks_a_query_match_above_a_shared_provider_prefix() {
    let items = [
        "radius radius/gpt-5.4 radius gpt-5.4 GPT 5.4",
        "radius radius/agentic radius agentic Agentic",
        "radius radius/gpt-6-sol radius gpt-6-sol GPT 6 Sol",
        "radius radius/gpt-6-astra radius gpt-6-astra GPT 6 Astra",
    ];
    let result = fuzzy_filter(&items, "astra", |x| x.to_string());

    assert_eq!(result[0], items[3]);
}
