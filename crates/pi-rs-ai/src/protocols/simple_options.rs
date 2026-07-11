//! Port of `providers/simple-options.ts`.

use pi_rs_ai_types::{Model, ThinkingBudgets, ThinkingLevel};

use super::options::{SimpleStreamOptions, StreamOptions};

/// Spec: `buildBaseOptions(model, options, apiKey)` — copies the shared
/// option surface; `api_key` wins over `options.api_key`.
pub fn build_base_options(
    _model: &Model,
    options: Option<&SimpleStreamOptions>,
    api_key: Option<&str>,
) -> StreamOptions {
    let base = options.map(|o| &o.base);
    StreamOptions {
        temperature: base.and_then(|o| o.temperature),
        max_tokens: base.and_then(|o| o.max_tokens),
        signal: base.and_then(|o| o.signal.clone()),
        api_key: api_key
            .filter(|key| !key.is_empty())
            .map(str::to_string)
            .or_else(|| base.and_then(|o| o.api_key.clone())),
        transport: base.and_then(|o| o.transport),
        cache_retention: base.and_then(|o| o.cache_retention),
        session_id: base.and_then(|o| o.session_id.clone()),
        on_payload: base.and_then(|o| o.on_payload.clone()),
        on_response: base.and_then(|o| o.on_response.clone()),
        headers: base.and_then(|o| o.headers.clone()),
        timeout_ms: base.and_then(|o| o.timeout_ms),
        websocket_connect_timeout_ms: base.and_then(|o| o.websocket_connect_timeout_ms),
        max_retries: base.and_then(|o| o.max_retries),
        max_retry_delay_ms: base.and_then(|o| o.max_retry_delay_ms),
        metadata: base.and_then(|o| o.metadata.clone()),
    }
}

/// Spec: `clampReasoning` — `xhigh` clamps to `high`.
pub fn clamp_reasoning(effort: ThinkingLevel) -> ThinkingLevel {
    if effort == ThinkingLevel::XHigh {
        ThinkingLevel::High
    } else {
        effort
    }
}

/// Result of [`adjust_max_tokens_for_thinking`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdjustedTokens {
    pub max_tokens: u64,
    pub thinking_budget: u64,
}

/// Spec: `adjustMaxTokensForThinking` — fit a thinking budget inside the
/// output cap. `base_max_tokens: None` means no explicit caller cap: use
/// the model cap and fit thinking inside it.
pub fn adjust_max_tokens_for_thinking(
    base_max_tokens: Option<u64>,
    model_max_tokens: u64,
    reasoning_level: ThinkingLevel,
    custom_budgets: Option<&ThinkingBudgets>,
) -> AdjustedTokens {
    const MIN_OUTPUT_TOKENS: u64 = 1024;
    let defaults = ThinkingBudgets {
        minimal: Some(1024),
        low: Some(2048),
        medium: Some(8192),
        high: Some(16384),
    };
    let pick = |default: Option<u64>, custom: Option<u64>| custom.or(default).unwrap_or(0);
    let custom = custom_budgets.copied().unwrap_or_default();

    let mut thinking_budget = match clamp_reasoning(reasoning_level) {
        ThinkingLevel::Minimal => pick(defaults.minimal, custom.minimal),
        ThinkingLevel::Low => pick(defaults.low, custom.low),
        ThinkingLevel::Medium => pick(defaults.medium, custom.medium),
        ThinkingLevel::High | ThinkingLevel::XHigh => pick(defaults.high, custom.high),
    };
    let max_tokens = match base_max_tokens {
        None => model_max_tokens,
        Some(base) => (base + thinking_budget).min(model_max_tokens),
    };

    if max_tokens <= thinking_budget {
        thinking_budget = max_tokens.saturating_sub(MIN_OUTPUT_TOKENS);
    }

    AdjustedTokens {
        max_tokens,
        thinking_budget,
    }
}
