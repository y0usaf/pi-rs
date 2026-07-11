//! Behavioral parity tests for `models.rs` against the spec's `models.ts`
//! (`calculateCost`, `getSupportedThinkingLevels`, `clampThinkingLevel`,
//! `modelsAreEqual`), driven by the transcribed model fixtures.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ModelThinkingLevel::{High, Low, Max, Medium, Minimal, Off, XHigh};
use pi_rs_ai_types::{
    Model, ModelCostTier, ModelThinkingLevel, Usage, calculate_cost, clamp_thinking_level,
    get_supported_thinking_levels, models_are_equal,
};

fn model(name: &str) -> Model {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn calculate_cost_matches_spec_math() {
    // Opus 4.7 rates: 5 / 25 / 0.5 / 6.25 $ per million tokens.
    let opus = model("model_anthropic_opus47.json");
    let mut usage = Usage {
        input: 1_000_000,
        output: 2_000_000,
        cache_read: 500_000,
        cache_write: 100_000,
        ..Usage::default()
    };
    let cost = calculate_cost(&opus, &mut usage);
    assert_eq!(cost.input, 5.0);
    assert_eq!(cost.output, 50.0);
    assert_eq!(cost.cache_read, 0.25);
    assert_eq!(cost.cache_write, 0.625);
    assert_eq!(cost.total, 5.0 + 50.0 + 0.25 + 0.625);
    // Spec mutates usage.cost in place.
    assert_eq!(usage.cost, cost);
}

#[test]
fn calculate_cost_selects_highest_matching_request_tier() {
    let mut opus = model("model_anthropic_opus47.json");
    opus.cost.tiers = vec![
        ModelCostTier {
            input_tokens_above: 100,
            input: 10.0,
            ..ModelCostTier::default()
        },
        ModelCostTier {
            input_tokens_above: 200,
            input: 20.0,
            ..ModelCostTier::default()
        },
    ];
    let mut usage = Usage {
        input: 201,
        ..Usage::default()
    };
    assert_eq!(calculate_cost(&opus, &mut usage).input, 0.00402);
}

#[test]
fn supported_levels_non_reasoning_is_off_only() {
    let mut nova = model("model_bedrock_nova2lite.json");
    nova.reasoning = false;
    assert_eq!(get_supported_thinking_levels(&nova), vec![Off]);
}

#[test]
fn supported_levels_defaults_exclude_unmapped_extended_levels() {
    // A reasoning model without a thinkingLevelMap supports defaults through
    // high; xhigh and max require explicit mappings.
    let copilot = model("model_copilot_haiku45.json");
    assert_eq!(
        get_supported_thinking_levels(&copilot),
        vec![Off, Minimal, Low, Medium, High]
    );
}

#[test]
fn supported_levels_explicit_nulls_are_unsupported() {
    // Ring 2.6: off/minimal/low/medium are explicitly null.
    let ring = model("model_antling_ring.json");
    assert_eq!(get_supported_thinking_levels(&ring), vec![High, XHigh]);

    // Opus 4.7 explicitly maps both extended levels.
    let opus = model("model_anthropic_opus47.json");
    assert_eq!(
        get_supported_thinking_levels(&opus),
        vec![Off, Minimal, Low, Medium, High, XHigh, Max]
    );
}

#[test]
fn clamp_searches_up_then_down() {
    let ring = model("model_antling_ring.json");
    // Supported: [high, xhigh]. Requests below clamp upward…
    assert_eq!(clamp_thinking_level(&ring, Off), High);
    assert_eq!(clamp_thinking_level(&ring, Low), High);
    assert_eq!(clamp_thinking_level(&ring, High), High);
    assert_eq!(clamp_thinking_level(&ring, XHigh), XHigh);

    // Copilot haiku: extended levels are unsupported → fall back to high.
    let copilot = model("model_copilot_haiku45.json");
    assert_eq!(clamp_thinking_level(&copilot, XHigh), High);
    assert_eq!(clamp_thinking_level(&copilot, Max), High);
    assert_eq!(clamp_thinking_level(&copilot, Medium), Medium);
}

#[test]
fn models_are_equal_compares_id_and_provider() {
    let a = model("model_anthropic_opus47.json");
    let mut b = a.clone();
    assert!(models_are_equal(Some(&a), Some(&b)));
    b.provider = "github-copilot".to_owned();
    assert!(!models_are_equal(Some(&a), Some(&b)));
    assert!(!models_are_equal(Some(&a), None));
    assert!(!models_are_equal(None, None));
}
