//! Port of `packages/ai/src/models.ts` — the pure model helpers.
//!
//! The registry half (`getModel` / `getProviders` / `getModels` over the
//! generated catalog) lands with WS2.4 (catalog as data); only the
//! catalog-independent functions live here.

use crate::types::{Model, ModelThinkingLevel, ThinkingLevelMap, Usage, UsageCost};

/// Spec: `EXTENDED_THINKING_LEVELS`.
pub const EXTENDED_THINKING_LEVELS: [ModelThinkingLevel; 7] = [
    ModelThinkingLevel::Off,
    ModelThinkingLevel::Minimal,
    ModelThinkingLevel::Low,
    ModelThinkingLevel::Medium,
    ModelThinkingLevel::High,
    ModelThinkingLevel::XHigh,
    ModelThinkingLevel::Max,
];

/// Spec: `calculateCost` — selects the highest matching request-wide pricing
/// tier, writes `usage.cost`, and returns it.
pub fn calculate_cost(model: &Model, usage: &mut Usage) -> UsageCost {
    let input_tokens = usage.input + usage.cache_read + usage.cache_write;
    let mut rates = (
        model.cost.input,
        model.cost.output,
        model.cost.cache_read,
        model.cost.cache_write,
    );
    let mut matched_threshold = None;
    for tier in &model.cost.tiers {
        if input_tokens > tier.input_tokens_above
            && matched_threshold.is_none_or(|threshold| tier.input_tokens_above > threshold)
        {
            rates = (tier.input, tier.output, tier.cache_read, tier.cache_write);
            matched_threshold = Some(tier.input_tokens_above);
        }
    }
    usage.cost.input = (rates.0 / 1_000_000.0) * usage.input as f64;
    usage.cost.output = (rates.1 / 1_000_000.0) * usage.output as f64;
    usage.cost.cache_read = (rates.2 / 1_000_000.0) * usage.cache_read as f64;
    usage.cost.cache_write = (rates.3 / 1_000_000.0) * usage.cache_write as f64;
    usage.cost.total =
        usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;
    usage.cost
}

/// Spec: `getSupportedThinkingLevels`.
///
/// A non-reasoning model supports only `off`. For a reasoning model, an
/// explicit `null` in `thinkingLevelMap` marks a level unsupported, a missing
/// key falls back to provider defaults — except `xhigh` and `max`, which are
/// supported only when explicitly mapped.
pub fn get_supported_thinking_levels(model: &Model) -> Vec<ModelThinkingLevel> {
    supported_thinking_levels_for(model.reasoning, model.thinking_level_map.as_ref())
}

/// [`get_supported_thinking_levels`] over the two fields the spec reads —
/// the duck-typed seam the Lua boundary uses (JS reads properties, not a
/// closed model type).
pub fn supported_thinking_levels_for(
    reasoning: bool,
    map: Option<&ThinkingLevelMap>,
) -> Vec<ModelThinkingLevel> {
    if !reasoning {
        return vec![ModelThinkingLevel::Off];
    }

    EXTENDED_THINKING_LEVELS
        .into_iter()
        .filter(|level| {
            let mapped = map.and_then(|map| map.get(level));
            match mapped {
                Some(None) => false,
                Some(Some(_)) => true,
                None => !matches!(level, ModelThinkingLevel::XHigh | ModelThinkingLevel::Max),
            }
        })
        .collect()
}

/// Spec: `clampThinkingLevel` — nearest supported level, searching upward
/// from the requested level first, then downward.
pub fn clamp_thinking_level(model: &Model, level: ModelThinkingLevel) -> ModelThinkingLevel {
    clamp_thinking_level_for(model.reasoning, model.thinking_level_map.as_ref(), level)
}

/// [`clamp_thinking_level`] over the two fields the spec reads (see
/// [`supported_thinking_levels_for`]).
pub fn clamp_thinking_level_for(
    reasoning: bool,
    map: Option<&ThinkingLevelMap>,
    level: ModelThinkingLevel,
) -> ModelThinkingLevel {
    let available = supported_thinking_levels_for(reasoning, map);
    if available.contains(&level) {
        return level;
    }

    let requested_index = EXTENDED_THINKING_LEVELS
        .iter()
        .position(|candidate| *candidate == level)
        .unwrap_or(0);

    for candidate in &EXTENDED_THINKING_LEVELS[requested_index..] {
        if available.contains(candidate) {
            return *candidate;
        }
    }
    for candidate in EXTENDED_THINKING_LEVELS[..requested_index].iter().rev() {
        if available.contains(candidate) {
            return *candidate;
        }
    }
    available
        .first()
        .copied()
        .unwrap_or(ModelThinkingLevel::Off)
}

/// Spec: `modelsAreEqual` — id + provider equality; `None` never matches.
pub fn models_are_equal(a: Option<&Model>, b: Option<&Model>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.id == b.id && a.provider == b.provider,
        _ => false,
    }
}
