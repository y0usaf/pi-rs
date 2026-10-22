//! `core/model-resolver.ts`: turning `--model`, `--models` and settings into a model.

use std::collections::BTreeSet;
use std::ptr;

use pi_durable::ai::types::Model;

use crate::args::is_valid_thinking_level;
use crate::collate::locale_compare;

pub const DEFAULT_THINKING_LEVEL: &str = "medium";

/// `defaultModelPerProvider`, in declaration order (the fallback walks it in order).
pub const DEFAULT_MODEL_PER_PROVIDER: &[(&str, &str)] = &[
    ("amazon-bedrock", "us.anthropic.claude-opus-4-6-v1"),
    ("ant-ling", "Ring-2.6-1T"),
    ("anthropic", "claude-opus-4-8"),
    ("openai", "gpt-5.5"),
    ("azure", "gpt-5.4"),
    ("openai-codex", "gpt-6.1-sol"),
    ("radius", "balanced"),
    ("nvidia", "nvidia/nemotron-3.5-super-vl-120b-a12b"),
    ("deepseek", "deepseek-v4-pro"),
    ("google", "gemini-3.1-pro-preview"),
    ("google-vertex", "gemini-3.1-pro-preview"),
    ("github-copilot", "gpt-6.1-sol"),
    ("openrouter", "moonshotai/kimi-k2.6"),
    ("vercel-ai-gateway", "zai/glm-5.1"),
    ("xai", "grok-4.7"),
    ("groq", "openai/gpt-oss-120b"),
    ("cerebras", "gpt-oss-120b"),
    ("zai", "glm-5.3"),
    ("zai-coding-cn", "glm-5.3"),
    ("mistral", "devstral-medium-latest"),
    ("minimax", "MiniMax-M2.7"),
    ("minimax-cn", "MiniMax-M2.7"),
    ("moonshotai", "kimi-k2.6"),
    ("moonshotai-cn", "kimi-k2.6"),
    ("huggingface", "moonshotai/Kimi-K2.6"),
    ("fireworks", "accounts/fireworks/models/kimi-k3"),
    ("together", "moonshotai/Kimi-K3"),
    ("baseten", "zai-org/GLM-5.2"),
    ("opencode", "kimi-k2.6"),
    ("opencode-go", "kimi-k3"),
    ("kimi-coding", "kimi-for-coding"),
    ("meta", "muse-spark-1.3"),
    ("cloudflare-workers-ai", "@cf/moonshotai/kimi-k2.6"),
    (
        "cloudflare-ai-gateway",
        "workers-ai/@cf/moonshotai/kimi-k2.6",
    ),
    ("qwen-token-plan", "qwen3.7-max"),
    ("qwen-token-plan-cn", "qwen3.7-max"),
    ("qwen-token-plan-individual", "qwen3.8-max"),
    ("xiaomi", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-cn", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-ams", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-sgp", "mimo-v2.6-pro"),
];

pub fn default_model_for(provider: &str) -> Option<&'static str> {
    DEFAULT_MODEL_PER_PROVIDER
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, id)| *id)
}

#[derive(Debug, Clone)]
pub struct ScopedModel {
    pub model: Model,
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSuffix {
    None,
    Strict,
    Lenient,
}

pub struct ReferenceRequest<'a> {
    pub text: &'a str,
    pub provider: Option<&'a str>,
    pub thinking_suffix: ThinkingSuffix,
    pub fuzzy: bool,
}

#[derive(Debug, Clone)]
pub struct Target<'m> {
    pub provider: String,
    /// Never empty; the first is the anchor model.
    pub provider_models: Vec<&'m Model>,
    pub model_text: String,
    pub model_id: String,
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Reference<'m> {
    Resolved {
        model: &'m Model,
        thinking_level: Option<String>,
        warning: Option<String>,
    },
    Ambiguous {
        candidates: Vec<&'m Model>,
        usable_count: usize,
        thinking_level: Option<String>,
        warning: Option<String>,
    },
    NotFound {
        target: Option<Target<'m>>,
    },
}

impl Reference<'_> {
    fn is_not_found(&self) -> bool {
        matches!(self, Reference::NotFound { .. })
    }

    fn with_thinking(
        mut self,
        level: Option<String>,
        warn: Option<String>,
        override_warning: bool,
    ) -> Self {
        match &mut self {
            Reference::Resolved {
                thinking_level,
                warning,
                ..
            }
            | Reference::Ambiguous {
                thinking_level,
                warning,
                ..
            } => {
                *thinking_level = level;
                if override_warning {
                    *warning = warn;
                }
            }
            Reference::NotFound { .. } => {}
        }
        self
    }

    fn warning(&self) -> Option<&String> {
        match self {
            Reference::Resolved { warning, .. } | Reference::Ambiguous { warning, .. } => {
                warning.as_ref()
            }
            Reference::NotFound { .. } => None,
        }
    }
}

fn resolved(model: &Model) -> Reference<'_> {
    Reference::Resolved {
        model,
        thinking_level: None,
        warning: None,
    }
}

fn split_thinking_suffix(text: &str) -> Option<(&str, &str)> {
    let colon = text.rfind(':')?;
    let suffix = &text[colon + 1..];
    is_valid_thinking_level(suffix).then(|| (&text[..colon], suffix))
}

fn split_provider_prefix<'t>(text: &'t str, models: &[&Model]) -> Option<(String, &'t str)> {
    let slash = text.find('/')?;
    let prefix = text[..slash].trim().to_lowercase();
    let model_text = text[slash + 1..].trim();
    if prefix.is_empty() || model_text.is_empty() {
        return None;
    }
    let provider = models
        .iter()
        .find(|m| m.provider.to_lowercase() == prefix)?
        .provider
        .clone();
    Some((provider, model_text))
}

fn distinct_providers(models: &[&Model]) -> usize {
    models
        .iter()
        .map(|m| m.provider.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

pub type Usable<'u> = &'u dyn Fn(&str) -> bool;

/// `resolveModelReference`.
pub fn resolve_model_reference<'m>(
    req: &ReferenceRequest,
    models: &'m [Model],
    usable: Usable,
) -> Reference<'m> {
    let text = req.text.trim();
    if text.is_empty() {
        return Reference::NotFound { target: None };
    }
    let constraint = req.provider.map(str::to_lowercase);
    let pool: Vec<&Model> = match &constraint {
        Some(c) => models
            .iter()
            .filter(|m| m.provider.to_lowercase() == *c)
            .collect(),
        None => models.iter().collect(),
    };
    let result = resolve_text(text, req, &pool, usable);
    if !result.is_not_found() {
        return result;
    }
    let split = split_provider_prefix(text, &pool);
    let anchor = if constraint.is_some() {
        pool.first().copied()
    } else {
        split
            .as_ref()
            .and_then(|(p, _)| pool.iter().find(|m| &m.provider == p).copied())
    };
    let Some(anchor) = anchor else { return result };
    let model_text = split.map(|(_, t)| t).unwrap_or(text).to_string();
    let suffix = if req.thinking_suffix == ThinkingSuffix::None {
        None
    } else {
        split_thinking_suffix(&model_text)
    };
    let mut provider_models = vec![anchor];
    provider_models.extend(
        pool.iter()
            .filter(|m| m.provider == anchor.provider && !ptr::eq(**m, anchor))
            .copied(),
    );
    Reference::NotFound {
        target: Some(Target {
            provider: anchor.provider.clone(),
            provider_models,
            model_id: suffix
                .map(|(b, _)| b.to_string())
                .unwrap_or_else(|| model_text.clone()),
            thinking_level: suffix.map(|(_, l)| l.to_string()),
            model_text,
        }),
    }
}

/// `findExactModelReferenceMatch`.
pub fn find_exact_model_reference_match<'m>(
    reference: &str,
    models: &'m [Model],
) -> Option<&'m Model> {
    let req = ReferenceRequest {
        text: reference,
        provider: None,
        thinking_suffix: ThinkingSuffix::None,
        fuzzy: false,
    };
    match resolve_model_reference(&req, models, &|_| true) {
        Reference::Resolved { model, .. } => Some(model),
        _ => None,
    }
}

fn resolve_text<'m>(
    text: &str,
    req: &ReferenceRequest,
    pool: &[&'m Model],
    usable: Usable,
) -> Reference<'m> {
    if let Some(r) = select_reference(text, pool, req.fuzzy, usable) {
        return r;
    }
    if req.fuzzy
        && let Some(r) = select_fuzzy(fuzzy_matches(text, pool), usable) {
            return r;
        }
    if req.thinking_suffix == ThinkingSuffix::None {
        return Reference::NotFound { target: None };
    }
    if let Some((base, level)) = split_thinking_suffix(text) {
        let inner = resolve_text(base, req, pool, usable);
        if inner.is_not_found() {
            return inner;
        }
        let level = if inner.warning().is_some() {
            None
        } else {
            Some(level.to_string())
        };
        return inner.with_thinking(level, None, false);
    }
    if req.thinking_suffix == ThinkingSuffix::Strict {
        return Reference::NotFound { target: None };
    }
    let Some(colon) = text.rfind(':') else {
        return Reference::NotFound { target: None };
    };
    let suffix = &text[colon + 1..];
    let inner = resolve_text(&text[..colon], req, pool, usable);
    if inner.is_not_found() {
        return inner;
    }
    let warning = format!(
        "Invalid thinking level \"{suffix}\" in pattern \"{text}\". Using default instead."
    );
    inner.with_thinking(None, Some(warning), true)
}

fn select_reference<'m>(
    text: &str,
    pool: &[&'m Model],
    fuzzy: bool,
    usable: Usable,
) -> Option<Reference<'m>> {
    let lower = text.to_lowercase();
    let raw: Vec<&Model> = pool
        .iter()
        .filter(|m| m.id.to_lowercase() == lower)
        .copied()
        .collect();
    let mut named: Option<&Model> = None;
    if let Some((provider, model_text)) = split_provider_prefix(text, pool) {
        let provider_models: Vec<&Model> = pool
            .iter()
            .filter(|m| m.provider == provider)
            .copied()
            .collect();
        let lower_model = model_text.to_lowercase();
        named = provider_models
            .iter()
            .find(|m| m.id.to_lowercase() == lower_model)
            .copied()
            .or_else(|| raw.iter().find(|m| m.provider == provider).copied())
            .or_else(|| {
                if fuzzy {
                    best_fuzzy_match(&fuzzy_matches(model_text, &provider_models))
                } else {
                    None
                }
            });
    }
    let others: Vec<&Model> = raw
        .iter()
        .filter(|m| !named.is_some_and(|n| ptr::eq(**m, n)))
        .copied()
        .collect();
    let mut candidates: Vec<&Model> = named.into_iter().collect();
    candidates.extend(others.iter().copied());
    if candidates.is_empty() {
        return None;
    }
    if distinct_providers(&candidates) == 1 {
        return Some(resolved(candidates[0]));
    }
    if let Some(n) = named.filter(|n| usable(&n.provider)) {
        return Some(resolved(n));
    }
    let usable_others: Vec<&Model> = others
        .iter()
        .filter(|m| usable(&m.provider))
        .copied()
        .collect();
    if usable_others.len() == 1 {
        return Some(resolved(usable_others[0]));
    }
    if let (true, Some(n)) = (usable_others.is_empty(), named) {
        return Some(resolved(n));
    }
    Some(Reference::Ambiguous {
        candidates,
        usable_count: usable_others.len(),
        thinking_level: None,
        warning: None,
    })
}

fn fuzzy_matches<'m>(text: &str, models: &[&'m Model]) -> Vec<&'m Model> {
    let lower = text.to_lowercase();
    models
        .iter()
        .filter(|m| m.id.to_lowercase().contains(&lower) || m.name.to_lowercase().contains(&lower))
        .copied()
        .collect()
}

fn select_fuzzy<'m>(matches: Vec<&'m Model>, usable: Usable) -> Option<Reference<'m>> {
    if matches.is_empty() {
        return None;
    }
    let multiple = distinct_providers(&matches) > 1;
    let usable_matches: Vec<&Model> = if multiple {
        matches
            .iter()
            .filter(|m| usable(&m.provider))
            .copied()
            .collect()
    } else {
        vec![]
    };
    let pool = if usable_matches.is_empty() {
        &matches
    } else {
        &usable_matches
    };
    let best = best_fuzzy_match(pool)?;
    let tied: Vec<&Model> = pool.iter().filter(|m| m.id == best.id).copied().collect();
    if distinct_providers(&tied) == 1 {
        return Some(resolved(best));
    }
    let usable_count = if usable_matches.is_empty() {
        0
    } else {
        tied.len()
    };
    Some(Reference::Ambiguous {
        candidates: tied,
        usable_count,
        thinking_level: None,
        warning: None,
    })
}

fn best_fuzzy_match<'m>(matches: &[&'m Model]) -> Option<&'m Model> {
    let aliases: Vec<&Model> = matches
        .iter()
        .filter(|m| is_alias(&m.id))
        .copied()
        .collect();
    let mut pool = if aliases.is_empty() {
        matches.to_vec()
    } else {
        aliases
    };
    pool.sort_by(|a, b| locale_compare(&b.id, &a.id));
    pool.first().copied()
}

/// `id.endsWith("-latest") || !/-\d{8}$/.test(id)`.
fn is_alias(id: &str) -> bool {
    if id.ends_with("-latest") {
        return true;
    }
    let b = id.as_bytes();
    let dated =
        b.len() >= 9 && b[b.len() - 9] == b'-' && b[b.len() - 8..].iter().all(u8::is_ascii_digit);
    !dated
}

fn build_fallback_model(provider_models: &[&Model], model_id: &str) -> Model {
    let first = provider_models[0];
    let base = default_model_for(&first.provider)
        .and_then(|d| provider_models.iter().find(|m| m.id == d).copied())
        .unwrap_or(first);
    let mut model = base.clone();
    model.id = model_id.to_string();
    model.name = model_id.to_string();
    model
}

/// A small `minimatch(text, pattern, { nocase: true })`: `*` and `?` stay within one
/// `/`-separated segment, `**` spans segments, `[...]` classes take ranges and `!`/`^`.
pub fn glob_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    glob_at(&t, &p)
}

fn glob_at(t: &[char], p: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') => {
            let globstar = p.get(1) == Some(&'*');
            let rest = if globstar { &p[2..] } else { &p[1..] };
            (0..=t.len())
                .take_while(|&i| globstar || i == 0 || t[i - 1] != '/')
                .any(|i| glob_at(&t[i..], rest))
        }
        Some('?') => t.first().is_some_and(|&c| c != '/') && glob_at(&t[1..], &p[1..]),
        Some('[') => match class_end(p) {
            Some(end) => {
                let Some(&c) = t.first() else { return false };
                c != '/' && class_matches(&p[1..end], c) && glob_at(&t[1..], &p[end + 1..])
            }
            None => t.first() == Some(&'[') && glob_at(&t[1..], &p[1..]),
        },
        Some(&c) => t.first() == Some(&c) && glob_at(&t[1..], &p[1..]),
    }
}

fn class_end(p: &[char]) -> Option<usize> {
    let mut i = 1;
    if matches!(p.get(i), Some('!' | '^')) {
        i += 1;
    }
    if p.get(i) == Some(&']') {
        i += 1;
    }
    p[i..].iter().position(|&c| c == ']').map(|j| i + j)
}

fn class_matches(class: &[char], c: char) -> bool {
    let (negate, class) = match class.first() {
        Some('!' | '^') => (true, &class[1..]),
        _ => (false, class),
    };
    let mut hit = false;
    let mut i = 0;
    while i < class.len() {
        if i + 2 < class.len() && class[i + 1] == '-' {
            hit |= class[i] <= c && c <= class[i + 2];
            i += 3;
        } else {
            hit |= class[i] == c;
            i += 1;
        }
    }
    hit != negate
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub pattern: String,
}

/// `resolveModelScopeFromModels`: `--models` / `enabledModels` patterns over the available models.
pub fn resolve_model_scope(
    patterns: &[String],
    models: &[Model],
) -> (Vec<ScopedModel>, Vec<ScopeDiagnostic>) {
    let mut scoped: Vec<ScopedModel> = Vec::new();
    let mut diags: Vec<ScopeDiagnostic> = Vec::new();
    let add = |scoped: &mut Vec<ScopedModel>, model: &Model, level: Option<String>| {
        if !scoped
            .iter()
            .any(|s| s.model.provider == model.provider && s.model.id == model.id)
        {
            scoped.push(ScopedModel {
                model: model.clone(),
                thinking_level: level,
            });
        }
    };
    let no_match = |diags: &mut Vec<ScopeDiagnostic>, pattern: &str| {
        diags.push(ScopeDiagnostic {
            code: "no-match",
            message: format!("No models match pattern \"{pattern}\""),
            pattern: pattern.to_string(),
        })
    };
    let ambiguous = |diags: &mut Vec<ScopeDiagnostic>, pattern: &str, candidates: &[&Model]| {
        let mut providers: Vec<&str> = candidates
            .iter()
            .map(|m| m.provider.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        providers.sort_by(|a, b| locale_compare(a, b));
        diags.push(ScopeDiagnostic {
            code: "ambiguous",
            message: format!(
                "Pattern \"{pattern}\" matches several providers: {}. Use provider/id to choose one.",
                providers.join(", ")
            ),
            pattern: pattern.to_string(),
        })
    };

    for pattern in patterns {
        if pattern.contains(['*', '?', '[']) {
            let thinking = split_thinking_suffix(pattern);
            let glob = thinking.map(|(b, _)| b).unwrap_or(pattern);
            let level = thinking.map(|(_, l)| l.to_string());
            let req = ReferenceRequest {
                text: glob,
                provider: None,
                thinking_suffix: ThinkingSuffix::None,
                fuzzy: false,
            };
            let matches: Vec<&Model> = match resolve_model_reference(&req, models, &|_| true) {
                Reference::Resolved { model, .. } => vec![model],
                Reference::Ambiguous { candidates, .. } => {
                    ambiguous(&mut diags, pattern, &candidates);
                    candidates
                }
                Reference::NotFound { .. } => models
                    .iter()
                    .filter(|m| {
                        glob_match(&format!("{}/{}", m.provider, m.id), glob)
                            || glob_match(&m.id, glob)
                    })
                    .collect(),
            };
            if matches.is_empty() {
                no_match(&mut diags, pattern);
            }
            for m in matches {
                add(&mut scoped, m, level.clone());
            }
            continue;
        }
        let req = ReferenceRequest {
            text: pattern,
            provider: None,
            thinking_suffix: ThinkingSuffix::Lenient,
            fuzzy: true,
        };
        let result = resolve_model_reference(&req, models, &|_| true);
        if let Some(w) = result.warning() {
            diags.push(ScopeDiagnostic {
                code: "invalid-thinking-level",
                message: w.clone(),
                pattern: pattern.clone(),
            });
        }
        match result {
            Reference::NotFound { .. } => no_match(&mut diags, pattern),
            Reference::Resolved {
                model,
                thinking_level,
                ..
            } => add(&mut scoped, model, thinking_level),
            Reference::Ambiguous {
                candidates,
                thinking_level,
                ..
            } => {
                ambiguous(&mut diags, pattern, &candidates);
                for m in candidates {
                    add(&mut scoped, m, thinking_level.clone());
                }
            }
        }
    }
    (scoped, diags)
}

#[derive(Debug, Clone, Default)]
pub struct CliModel {
    pub model: Option<Model>,
    pub thinking_level: Option<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

fn cli_error(error: String) -> CliModel {
    CliModel {
        error: Some(error),
        ..CliModel::default()
    }
}

/// `resolveCliModel`: `--provider` / `--model`.
pub fn resolve_cli_model(
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    cli_thinking: Option<&str>,
    inline_api_key: bool,
    models: &[Model],
    has_auth: Usable,
) -> CliModel {
    let Some(cli_model) = cli_model else {
        return CliModel::default();
    };
    if models.is_empty() {
        return cli_error(
            "No models available. Check your installation or add models to models.json.".into(),
        );
    }
    let provider = cli_provider
        .and_then(|cp| {
            models
                .iter()
                .find(|m| m.provider.to_lowercase() == cp.to_lowercase())
        })
        .map(|m| m.provider.clone());
    if let (Some(cp), None) = (cli_provider, &provider) {
        return cli_error(format!(
            "Unknown provider \"{cp}\". Use --list-models to see available providers/models."
        ));
    }
    let req = ReferenceRequest {
        text: cli_model,
        provider: provider.as_deref(),
        thinking_suffix: ThinkingSuffix::Strict,
        fuzzy: true,
    };
    let never = |_: &str| false;
    let usable: Usable = if inline_api_key { &never } else { has_auth };
    match resolve_model_reference(&req, models, usable) {
        Reference::Resolved {
            model,
            thinking_level,
            warning,
        } => CliModel {
            model: Some(model.clone()),
            thinking_level,
            warning,
            error: None,
        },
        Reference::Ambiguous {
            candidates,
            usable_count,
            ..
        } => {
            let mut names: Vec<String> = candidates
                .iter()
                .map(|m| format!("{}/{}", m.provider, m.id))
                .collect();
            names.sort_by(|a, b| locale_compare(a, b));
            let hint = if inline_api_key {
                "--api-key does not choose a provider."
            } else if usable_count == 0 {
                "No matching provider is authenticated."
            } else {
                "More than one matching provider is authenticated."
            };
            cli_error(format!(
                "Model \"{cli_model}\" is ambiguous across providers: {}. {hint} Use --provider or provider/model.",
                names.join(", ")
            ))
        }
        Reference::NotFound { target: None } => cli_error(format!(
            "Model \"{cli_model}\" not found. Use --list-models to see available models."
        )),
        Reference::NotFound {
            target: Some(target),
        } => {
            let model_id = if cli_thinking.is_some() {
                &target.model_text
            } else {
                &target.model_id
            };
            let fallback_thinking = if cli_thinking.is_some() {
                None
            } else {
                target.thinking_level.clone()
            };
            let mut model = build_fallback_model(&target.provider_models, model_id);
            if cli_thinking
                .or(fallback_thinking.as_deref())
                .is_some_and(|l| l != "off")
            {
                model.reasoning = true;
            }
            CliModel {
                model: Some(model),
                thinking_level: fallback_thinking,
                warning: Some(format!(
                    "Model \"{model_id}\" not found for provider \"{}\". Using custom model id.",
                    target.provider
                )),
                error: None,
            }
        }
    }
}

/// `findInitialModel`'s choice when neither scope nor settings settle it:
/// a provider default among the available models, else the first available.
pub fn default_available<'m>(available: &[&'m Model]) -> Option<&'m Model> {
    DEFAULT_MODEL_PER_PROVIDER
        .iter()
        .find_map(|(p, id)| {
            available
                .iter()
                .find(|m| m.provider == *p && m.id == *id)
                .copied()
        })
        .or_else(|| available.first().copied())
}

pub struct InitialModel {
    pub model: Option<Model>,
    pub thinking_level: String,
}

/// `findInitialModel` (not continuing a session).
pub fn find_initial_model(
    scoped: &[ScopedModel],
    default_provider: Option<&str>,
    default_model_id: Option<&str>,
    default_thinking: Option<&str>,
    model_thinking: &dyn Fn(&str, &str) -> Option<String>,
    get_model: &dyn Fn(&str, &str) -> Option<Model>,
    has_auth: Usable,
    available: &[&Model],
) -> InitialModel {
    let fallback = || DEFAULT_THINKING_LEVEL.to_string();
    if let Some(first) = scoped.first() {
        let level = first
            .thinking_level
            .clone()
            .or_else(|| model_thinking(&first.model.provider, &first.model.id))
            .or_else(|| default_thinking.map(str::to_string))
            .unwrap_or_else(fallback);
        return InitialModel {
            model: Some(first.model.clone()),
            thinking_level: level,
        };
    }
    if let (Some(p), Some(id)) = (default_provider, default_model_id)
        && let Some(found) = get_model(p, id).filter(|m| has_auth(&m.provider)) {
            let level = model_thinking(p, id)
                .or_else(|| default_thinking.map(str::to_string))
                .unwrap_or_else(fallback);
            return InitialModel {
                model: Some(found),
                thinking_level: level,
            };
        }
    InitialModel {
        model: default_available(available).cloned(),
        thinking_level: fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("openai/gpt-5.5", "OPENAI/*"));
        assert!(!glob_match("openrouter/openai/gpt-5.5", "openrouter/*"));
        assert!(glob_match("openrouter/openai/gpt-5.5", "openrouter/**"));
        assert!(glob_match("gpt-5.5-mini", "gpt-5.[0-5]?mini"));
        assert!(glob_match("mock-1", "[!a-c]*"));
        assert!(!glob_match("claude", "[!a-c]*"));
    }

    #[test]
    fn aliases() {
        assert!(is_alias("claude-sonnet-4-5"));
        assert!(is_alias("x-latest"));
        assert!(!is_alias("claude-sonnet-4-5-20250929"));
    }

    #[test]
    fn initial_model_prefers_provider_defaults() {
        let a = Model::new("zzz", "first", "openai-completions", 1, 1, false);
        let b = Model::new("openai", "gpt-5.5", "openai-completions", 1, 1, false);
        let available = [&a, &b];
        assert_eq!(default_available(&available).unwrap().id, "gpt-5.5");
        let r = find_initial_model(
            &[],
            None,
            None,
            None,
            &|_, _| None,
            &|_, _| None,
            &|_| true,
            &available[..1],
        );
        assert_eq!(r.model.unwrap().id, "first");
        assert_eq!(r.thinking_level, "medium");
    }
}
