//! Port of `core/model-resolver.ts` — model resolution and initial
//! selection.
//!
//! WS2.6/WS3.1 subset (recorded): `resolveModelScope` (glob patterns
//! for `--models` cycling) and `restoreModelFromSession` land with
//! their consumers (WS9 cli parity / WS3 sessions). `findInitialModel`
//! covers steps 3–5 (settings default, provider defaults, first
//! available); steps 1–2 (CLI args, scoped models) are the caller's (as
//! in the spec's `buildSessionOptions` / `sdk.ts`).

use pi_rs_ai_types::{Model, ModelThinkingLevel};

use crate::core::defaults::DEFAULT_THINKING_LEVEL;
use crate::core::model_registry::ModelRegistry;

/// Spec: `defaultModelPerProvider` — default model IDs per known
/// provider, in the spec's declaration order (the order is meaningful:
/// `findInitialModel` scans it).
pub const DEFAULT_MODEL_PER_PROVIDER: &[(&str, &str)] = &[
    ("amazon-bedrock", "us.anthropic.claude-opus-4-6-v1"),
    ("ant-ling", "Ring-2.6-1T"),
    ("anthropic", "claude-opus-4-8"),
    ("openai", "gpt-5.4"),
    ("azure-openai-responses", "gpt-5.4"),
    ("openai-codex", "gpt-5.5"),
    ("nvidia", "nvidia/nemotron-3-super-120b-a12b"),
    ("deepseek", "deepseek-v4-pro"),
    ("google", "gemini-3.1-pro-preview"),
    ("google-vertex", "gemini-3.1-pro-preview"),
    ("github-copilot", "gpt-5.4"),
    ("openrouter", "moonshotai/kimi-k2.6"),
    ("vercel-ai-gateway", "zai/glm-5.1"),
    ("xai", "grok-4.20-0309-reasoning"),
    ("groq", "openai/gpt-oss-120b"),
    ("cerebras", "zai-glm-4.7"),
    ("zai", "glm-5.1"),
    ("zai-coding-cn", "glm-5.1"),
    ("mistral", "devstral-medium-latest"),
    ("minimax", "MiniMax-M2.7"),
    ("minimax-cn", "MiniMax-M2.7"),
    ("moonshotai", "kimi-k2.6"),
    ("moonshotai-cn", "kimi-k2.6"),
    ("huggingface", "moonshotai/Kimi-K2.6"),
    ("fireworks", "accounts/fireworks/models/kimi-k2p6"),
    ("together", "moonshotai/Kimi-K2.6"),
    ("opencode", "kimi-k2.6"),
    ("opencode-go", "kimi-k2.6"),
    ("kimi-coding", "kimi-for-coding"),
    ("cloudflare-workers-ai", "@cf/moonshotai/kimi-k2.6"),
    (
        "cloudflare-ai-gateway",
        "workers-ai/@cf/moonshotai/kimi-k2.6",
    ),
    ("xiaomi", "mimo-v2.5-pro"),
    ("xiaomi-token-plan-cn", "mimo-v2.5-pro"),
    ("xiaomi-token-plan-ams", "mimo-v2.5-pro"),
    ("xiaomi-token-plan-sgp", "mimo-v2.5-pro"),
];

// Spec: `isValidThinkingLevel` (`cli/args.ts`) — lives with the settings
// port in pi-rs-host (the `pi.settings` consumer); re-exported to keep the
// spec shape.
pub use pi_rs_host::settings_manager::parse_thinking_level;

/// Spec: `isAlias` — no `-YYYYMMDD` date suffix, or a `-latest` suffix.
fn is_alias(id: &str) -> bool {
    if id.ends_with("-latest") {
        return true;
    }
    let Some(idx) = id.rfind('-') else {
        return true;
    };
    let suffix = &id[idx + 1..];
    !(suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()))
}

/// Spec: `findExactModelReferenceMatch` — bare model id or canonical
/// `provider/modelId`; ambiguous bare-id matches are rejected.
pub fn find_exact_model_reference_match<'a>(
    model_reference: &str,
    available_models: &[&'a Model],
) -> Option<&'a Model> {
    let trimmed = model_reference.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.to_lowercase();

    let canonical: Vec<&&Model> = available_models
        .iter()
        .filter(|m| format!("{}/{}", m.provider, m.id).to_lowercase() == normalized)
        .collect();
    if canonical.len() == 1 {
        return Some(canonical[0]);
    }
    if canonical.len() > 1 {
        return None;
    }

    if let Some(slash_index) = trimmed.find('/') {
        let provider = trimmed[..slash_index].trim();
        let model_id = trimmed[slash_index + 1..].trim();
        if !provider.is_empty() && !model_id.is_empty() {
            let provider_matches: Vec<&&Model> = available_models
                .iter()
                .filter(|m| {
                    m.provider.to_lowercase() == provider.to_lowercase()
                        && m.id.to_lowercase() == model_id.to_lowercase()
                })
                .collect();
            if provider_matches.len() == 1 {
                return Some(provider_matches[0]);
            }
            if provider_matches.len() > 1 {
                return None;
            }
        }
    }

    let id_matches: Vec<&&Model> = available_models
        .iter()
        .filter(|m| m.id.to_lowercase() == normalized)
        .collect();
    if id_matches.len() == 1 {
        Some(id_matches[0])
    } else {
        None
    }
}

/// Spec: `tryMatchModel` — exact reference first, then partial id/name
/// matching with alias preference.
fn try_match_model<'a>(model_pattern: &str, available_models: &[&'a Model]) -> Option<&'a Model> {
    if let Some(exact) = find_exact_model_reference_match(model_pattern, available_models) {
        return Some(exact);
    }

    let pattern_lower = model_pattern.to_lowercase();
    let matches: Vec<&&Model> = available_models
        .iter()
        .filter(|m| {
            m.id.to_lowercase().contains(&pattern_lower)
                || m.name.to_lowercase().contains(&pattern_lower)
        })
        .collect();

    if matches.is_empty() {
        return None;
    }

    let mut aliases: Vec<&&Model> = matches
        .iter()
        .filter(|m| is_alias(&m.id))
        .copied()
        .collect();
    let mut dated: Vec<&&Model> = matches
        .iter()
        .filter(|m| !is_alias(&m.id))
        .copied()
        .collect();

    if !aliases.is_empty() {
        // Prefer alias — pick the one that sorts highest.
        aliases.sort_by(|a, b| b.id.cmp(&a.id));
        Some(aliases[0])
    } else {
        // No alias found — pick latest dated version.
        dated.sort_by(|a, b| b.id.cmp(&a.id));
        Some(dated[0])
    }
}

/// Spec: `ParsedModelResult`.
#[derive(Clone, Debug, Default)]
pub struct ParsedModelResult<'a> {
    pub model: Option<&'a Model>,
    pub thinking_level: Option<ModelThinkingLevel>,
    pub warning: Option<String>,
}

/// Spec: `parseModelPattern` — try the full pattern, then split on the
/// last colon for a `:<thinking>` suffix.
pub fn parse_model_pattern<'a>(
    pattern: &str,
    available_models: &[&'a Model],
    allow_invalid_thinking_level_fallback: bool,
) -> ParsedModelResult<'a> {
    if let Some(exact) = try_match_model(pattern, available_models) {
        return ParsedModelResult {
            model: Some(exact),
            thinking_level: None,
            warning: None,
        };
    }

    let Some(last_colon) = pattern.rfind(':') else {
        return ParsedModelResult::default();
    };

    let prefix = &pattern[..last_colon];
    let suffix = &pattern[last_colon + 1..];

    if let Some(level) = parse_thinking_level(suffix) {
        let result = parse_model_pattern(
            prefix,
            available_models,
            allow_invalid_thinking_level_fallback,
        );
        if result.model.is_some() {
            let has_warning = result.warning.is_some();
            return ParsedModelResult {
                model: result.model,
                thinking_level: if has_warning { None } else { Some(level) },
                warning: result.warning,
            };
        }
        result
    } else {
        if !allow_invalid_thinking_level_fallback {
            // Strict mode (CLI --model): treat the suffix as part of the
            // model id and fail rather than resolve a different model.
            return ParsedModelResult::default();
        }
        let result = parse_model_pattern(
            prefix,
            available_models,
            allow_invalid_thinking_level_fallback,
        );
        if result.model.is_some() {
            return ParsedModelResult {
                model: result.model,
                thinking_level: None,
                warning: Some(format!(
                    "Invalid thinking level \"{suffix}\" in pattern \"{pattern}\". Using default instead."
                )),
            };
        }
        result
    }
}

/// Spec: `buildFallbackModel` — a provider's default (or first) model
/// with the requested custom id.
fn build_fallback_model(
    provider: &str,
    model_id: &str,
    available_models: &[&Model],
) -> Option<Model> {
    let provider_models: Vec<&&Model> = available_models
        .iter()
        .filter(|m| m.provider == provider)
        .collect();
    let first = provider_models.first()?;

    let default_id = DEFAULT_MODEL_PER_PROVIDER
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, id)| *id);
    let base_model = default_id
        .and_then(|id| provider_models.iter().find(|m| m.id == id))
        .unwrap_or(first);

    let mut model = (**base_model).clone();
    model.id = model_id.to_owned();
    model.name = model_id.to_owned();
    Some(model)
}

/// Spec: `ResolveCliModelResult`.
#[derive(Clone, Debug, Default)]
pub struct ResolveCliModelResult {
    pub model: Option<Model>,
    pub thinking_level: Option<ModelThinkingLevel>,
    pub warning: Option<String>,
    /// When set, `model` is `None`.
    pub error: Option<String>,
}

/// Spec: `resolveCliModel` — resolve a single model from CLI flags
/// (`--provider <name> --model <pattern>` / `--model <provider>/<pattern>`).
pub fn resolve_cli_model(
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    model_registry: &ModelRegistry,
) -> ResolveCliModelResult {
    let Some(cli_model) = cli_model else {
        return ResolveCliModelResult::default();
    };

    // Important: all models, not just configured-auth ones — this lets
    // `--api-key` be used for first-time setup.
    let available_models: Vec<&Model> = model_registry.get_all().iter().collect();
    if available_models.is_empty() {
        return ResolveCliModelResult {
            error: Some(
                "No models available. Check your installation or add models to models.json."
                    .to_owned(),
            ),
            ..Default::default()
        };
    }

    // Canonical provider lookup (case-insensitive).
    let canonical_provider = |name: &str| -> Option<String> {
        let lower = name.to_lowercase();
        available_models
            .iter()
            .find(|m| m.provider.to_lowercase() == lower)
            .map(|m| m.provider.clone())
    };

    let mut provider = cli_provider.and_then(canonical_provider);
    if let Some(cli_provider) = cli_provider
        && provider.is_none()
    {
        return ResolveCliModelResult {
            error: Some(format!(
                "Unknown provider \"{cli_provider}\". Use --list-models to see available providers/models."
            )),
            ..Default::default()
        };
    }

    // Without --provider, try the "provider/model" interpretation first.
    let mut pattern = cli_model.to_owned();
    let mut inferred_provider = false;

    if provider.is_none()
        && let Some(slash_index) = cli_model.find('/')
        && let Some(canonical) = canonical_provider(&cli_model[..slash_index])
    {
        provider = Some(canonical);
        pattern = cli_model[slash_index + 1..].to_owned();
        inferred_provider = true;
    }

    let exact_across_all = |reference: &str| -> Option<Model> {
        let lower = reference.to_lowercase();
        available_models
            .iter()
            .find(|m| {
                m.id.to_lowercase() == lower
                    || format!("{}/{}", m.provider, m.id).to_lowercase() == lower
            })
            .map(|m| (*m).clone())
    };

    // No provider inferred: exact matches handle ids with natural slashes.
    if provider.is_none()
        && let Some(exact) = exact_across_all(cli_model)
    {
        return ResolveCliModelResult {
            model: Some(exact),
            ..Default::default()
        };
    }

    if cli_provider.is_some()
        && let Some(provider) = provider.as_deref()
    {
        // Tolerate `--model <provider>/<pattern>` alongside --provider.
        let prefix = format!("{provider}/").to_lowercase();
        if cli_model.to_lowercase().starts_with(&prefix) {
            pattern = cli_model[prefix.len()..].to_owned();
        }
    }

    let candidates: Vec<&Model> = match provider.as_deref() {
        Some(provider) => available_models
            .iter()
            .filter(|m| m.provider == provider)
            .copied()
            .collect(),
        None => available_models.clone(),
    };
    let parsed = parse_model_pattern(&pattern, &candidates, false);

    if let Some(model) = parsed.model {
        return ResolveCliModelResult {
            model: Some(model.clone()),
            thinking_level: parsed.thinking_level,
            warning: parsed.warning,
            error: None,
        };
    }

    // Inferred provider with no match: fall back to matching the full
    // input as a raw model id across all models.
    if inferred_provider {
        if let Some(exact) = exact_across_all(cli_model) {
            return ResolveCliModelResult {
                model: Some(exact),
                ..Default::default()
            };
        }
        let fallback = parse_model_pattern(cli_model, &available_models, false);
        if let Some(model) = fallback.model {
            return ResolveCliModelResult {
                model: Some(model.clone()),
                thinking_level: fallback.thinking_level,
                warning: fallback.warning,
                error: None,
            };
        }
    }

    if let Some(provider) = provider.as_deref()
        && let Some(fallback_model) = build_fallback_model(provider, &pattern, &available_models)
    {
        let message = format!(
            "Model \"{pattern}\" not found for provider \"{provider}\". Using custom model id."
        );
        let fallback_warning = match parsed.warning {
            Some(warning) => format!("{warning} {message}"),
            None => message,
        };
        return ResolveCliModelResult {
            model: Some(fallback_model),
            thinking_level: None,
            warning: Some(fallback_warning),
            error: None,
        };
    }

    let display = match provider.as_deref() {
        Some(provider) => format!("{provider}/{pattern}"),
        None => cli_model.to_owned(),
    };
    ResolveCliModelResult {
        model: None,
        thinking_level: None,
        warning: parsed.warning,
        error: Some(format!(
            "Model \"{display}\" not found. Use --list-models to see available models."
        )),
    }
}

/// Spec: `InitialModelResult` — `fallback_message` arrives with its
/// consumers (scoped models / session restore, WS3+).
#[derive(Clone, Debug)]
pub struct InitialModelResult {
    pub model: Option<Model>,
    pub thinking_level: ModelThinkingLevel,
}

/// Spec: `findInitialModel` steps 3–5 — settings default (with its
/// thinking level), then the first available model preferring
/// per-provider defaults in declaration order. (Steps 1–2 — CLI args,
/// scoped models — are the caller's; see module doc.)
pub fn find_initial_model(
    model_registry: &ModelRegistry,
    auth_storage: &crate::core::auth_storage::AuthStorage,
    default_provider: Option<&str>,
    default_model_id: Option<&str>,
    default_thinking_level: Option<ModelThinkingLevel>,
) -> InitialModelResult {
    // 3. Try saved default from settings
    if let (Some(provider), Some(model_id)) = (default_provider, default_model_id)
        && let Some(found) = model_registry.find(provider, model_id)
    {
        return InitialModelResult {
            model: Some(found.clone()),
            thinking_level: default_thinking_level.unwrap_or(DEFAULT_THINKING_LEVEL),
        };
    }

    // 4. Try first available model with valid API key
    let available = model_registry.get_available(auth_storage);
    let model = DEFAULT_MODEL_PER_PROVIDER
        .iter()
        .find_map(|(provider, default_id)| {
            available
                .iter()
                .find(|m| m.provider == *provider && m.id == *default_id)
        })
        .or_else(|| available.first())
        .map(|m| (*m).clone());

    // 5. No model found
    InitialModelResult {
        model,
        thinking_level: DEFAULT_THINKING_LEVEL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::auth_storage::{AuthCredential, AuthStorage, AuthStorageData};

    fn registry() -> (AuthStorage, ModelRegistry) {
        let mut data = AuthStorageData::new();
        data.insert(
            "anthropic".to_owned(),
            AuthCredential::ApiKey { key: "sk".into() },
        );
        let storage = AuthStorage::in_memory(data);
        let registry = ModelRegistry::new(&storage);
        (storage, registry)
    }

    #[test]
    fn alias_detection() {
        assert!(is_alias("claude-sonnet-4-5"));
        assert!(is_alias("claude-3-5-sonnet-latest"));
        assert!(!is_alias("claude-sonnet-4-5-20250929"));
    }

    #[test]
    fn exact_reference_matching() {
        let (_storage, registry) = registry();
        let all: Vec<&Model> = registry.get_all().iter().collect();
        let m = find_exact_model_reference_match("anthropic/claude-opus-4-8", &all);
        assert_eq!(m.map(|m| m.provider.as_str()), Some("anthropic"));
        // Bare ids that exist under several providers are ambiguous
        // and rejected (anthropic / cloudflare-ai-gateway / opencode).
        let m = find_exact_model_reference_match("claude-opus-4-8", &all);
        assert!(m.is_none());
    }

    #[test]
    fn pattern_with_thinking_suffix() {
        let (_storage, registry) = registry();
        let all: Vec<&Model> = registry.get_all().iter().collect();
        let parsed = parse_model_pattern("claude-opus-4-8:high", &all, true);
        assert!(parsed.model.is_some());
        assert_eq!(parsed.thinking_level, Some(ModelThinkingLevel::High));
        assert!(parsed.warning.is_none());
    }

    #[test]
    fn invalid_suffix_warns_in_scope_mode_and_fails_in_strict_mode() {
        let (_storage, registry) = registry();
        let all: Vec<&Model> = registry.get_all().iter().collect();
        let scope = parse_model_pattern("claude-opus-4-8:hgih", &all, true);
        assert!(scope.model.is_some());
        assert_eq!(
            scope.warning.as_deref(),
            Some(
                "Invalid thinking level \"hgih\" in pattern \"claude-opus-4-8:hgih\". Using default instead."
            )
        );
        let strict = parse_model_pattern("claude-opus-4-8:hgih", &all, false);
        assert!(strict.model.is_none());
    }

    #[test]
    fn cli_model_provider_slash_pattern() {
        let (_storage, registry) = registry();
        let resolved = resolve_cli_model(None, Some("anthropic/claude-opus-4-8"), &registry);
        assert_eq!(
            resolved.model.map(|m| m.id),
            Some("claude-opus-4-8".to_owned())
        );
        assert!(resolved.error.is_none());
    }

    #[test]
    fn cli_unknown_provider_errors() {
        let (_storage, registry) = registry();
        let resolved = resolve_cli_model(Some("nope"), Some("x"), &registry);
        assert_eq!(
            resolved.error.as_deref(),
            Some("Unknown provider \"nope\". Use --list-models to see available providers/models.")
        );
    }

    #[test]
    fn cli_unknown_model_with_provider_falls_back_to_custom_id() {
        let (_storage, registry) = registry();
        let resolved = resolve_cli_model(Some("anthropic"), Some("my-fine-tune"), &registry);
        let model = resolved.model.unwrap_or_else(|| unreachable!());
        assert_eq!(model.id, "my-fine-tune");
        assert_eq!(model.provider, "anthropic");
        assert_eq!(
            resolved.warning.as_deref(),
            Some(
                "Model \"my-fine-tune\" not found for provider \"anthropic\". Using custom model id."
            )
        );
    }

    #[test]
    fn initial_model_prefers_provider_default() {
        let (storage, registry) = registry();
        let result = find_initial_model(&registry, &storage, None, None, None);
        // anthropic is the only configured provider unless env keys add
        // more; bedrock (earlier in the table) is ambient-authenticated
        // only when AWS_* env vars exist.
        assert!(result.model.is_some());
        assert_eq!(result.thinking_level, ModelThinkingLevel::Medium);
    }

    #[test]
    fn initial_model_settings_default_wins_with_its_thinking_level() {
        let (storage, registry) = registry();
        let result = find_initial_model(
            &registry,
            &storage,
            Some("anthropic"),
            Some("claude-sonnet-4-5"),
            Some(ModelThinkingLevel::High),
        );
        let model = result.model.unwrap_or_else(|| unreachable!());
        assert_eq!(model.id, "claude-sonnet-4-5");
        assert_eq!(result.thinking_level, ModelThinkingLevel::High);
    }

    #[test]
    fn initial_model_unresolvable_settings_default_falls_through() {
        let (storage, registry) = registry();
        let result = find_initial_model(&registry, &storage, Some("anthropic"), Some("gone"), None);
        assert!(result.model.is_some());
        assert_eq!(result.thinking_level, ModelThinkingLevel::Medium);
    }
}
