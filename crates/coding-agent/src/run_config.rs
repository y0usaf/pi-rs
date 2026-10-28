//! What `main.ts`, `buildSessionOptions` and `createAgentSession` (sdk.ts) decide before a print
//! run: the active tools, the model and the thinking level, with upstream's diagnostics.

use pi_durable::ai::thinking::clamp_thinking_level;
use pi_durable::ai::types::{Model, ModelThinkingLevel};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::harness::Conversation;
use pi_durable::harness::types::ModelRef;

use crate::args::{Args, tool_list_error};
use crate::model_resolver::{
    DEFAULT_THINKING_LEVEL, InitialModelSources, ScopedModel, find_initial_model, glob_match,
    resolve_cli_model, resolve_model_scope,
};
use crate::models::ModelRegistry;
use crate::settings::{DEFAULT_TOOL_NAMES, SettingsManager, apply_tool_modifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub error: bool,
    pub message: String,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            error: true,
            message: message.into(),
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            error: false,
            message: message.into(),
        }
    }

    /// `reportDiagnostics`: `Error: ...` / `Warning: ...` on stderr.
    pub fn render(&self) -> String {
        format!(
            "{}{}",
            if self.error { "Error: " } else { "Warning: " },
            self.message
        )
    }
}

fn is_modifier(entry: &str) -> bool {
    entry.starts_with('+') || entry.starts_with('-')
}

/// `createToolNameMatcher`: exact names, and `*` patterns.
fn tool_matches(entries: &[String], name: &str) -> bool {
    entries.iter().any(|e| {
        if e.contains('*') {
            glob_match(name, e)
        } else {
            e == name
        }
    })
}

/// The tools a session starts with (sdk.ts): `--tools` (a list, or `+name`/`-name` changes to
/// the default), else `defaultTools`, else read, bash, edit, write; minus `--exclude-tools`.
pub fn select_tools(args: &Args, settings: &SettingsManager) -> Result<Vec<String>, String> {
    if let Some(tools) = &args.tools
        && let Some(error) = tool_list_error(tools)
    {
        return Err(format!("Invalid tools option: {error}"));
    }
    let defaults: Vec<String> = if args.no_tools || args.no_builtin_tools {
        Vec::new()
    } else {
        settings
            .default_tools()
            .unwrap_or_else(|| DEFAULT_TOOL_NAMES.iter().map(|s| s.to_string()).collect())
    };
    let selected = match &args.tools {
        Some(tools) if tools.iter().any(|t| is_modifier(t)) => {
            apply_tool_modifiers(defaults, tools)
        }
        Some(tools) => tools.clone(),
        None => defaults,
    };
    let excluded = args.exclude_tools.clone().unwrap_or_default();
    Ok(selected
        .into_iter()
        .filter(|name| !tool_matches(&excluded, name))
        .collect())
}

pub fn parse_level(level: &str) -> Option<ModelThinkingLevel> {
    serde_json::from_value(serde_json::Value::String(level.to_string())).ok()
}

pub fn level_name(level: ModelThinkingLevel) -> String {
    serde_json::to_value(level)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub struct ModelChoice {
    pub model: Option<Model>,
    pub thinking: ModelThinkingLevel,
    pub diagnostics: Vec<Diagnostic>,
}

/// The model and thinking level: `--model` (with `--provider` and a `:level` suffix), else the
/// `--models`/`enabledModels` scope (preferring the saved default), else the session's model
/// when continuing, else `findInitialModel`; the level is `--thinking`, a pattern's level, the
/// per-model setting, `defaultThinkingLevel`, or medium, clamped to what the model supports.
pub fn choose_model(
    args: &Args,
    settings: &SettingsManager,
    registry: &ModelRegistry,
    session_model: Option<Model>,
    session_thinking: Option<ModelThinkingLevel>,
) -> ModelChoice {
    let mut diagnostics = Vec::new();
    let mut model: Option<Model> = None;
    let mut thinking: Option<String> = None;
    let has_auth = |p: &str| registry.has_configured_auth(p);

    let patterns = args
        .models
        .clone()
        .or_else(|| settings.enabled_models())
        .unwrap_or_default();
    let scoped: Vec<ScopedModel> = if patterns.is_empty() {
        Vec::new()
    } else {
        let available: Vec<Model> = registry.available().into_iter().cloned().collect();
        let (scoped, diags) = resolve_model_scope(&patterns, &available);
        for d in diags {
            eprintln!("Warning: {}", d.message);
        }
        scoped
    };

    if let (Some(provider), None) = (&args.provider, &args.model) {
        diagnostics.push(Diagnostic::error(format!(
            "--provider requires --model (for example: --provider {provider} --model <pattern>)"
        )));
    }
    if args.model.is_some() {
        let resolved = resolve_cli_model(
            args.provider.as_deref(),
            args.model.as_deref(),
            args.thinking.as_deref(),
            args.api_key.is_some(),
            registry.models(),
            &has_auth,
        );
        if let Some(w) = resolved.warning {
            diagnostics.push(Diagnostic::warning(w));
        }
        if let Some(e) = resolved.error {
            diagnostics.push(Diagnostic::error(e));
        }
        if resolved.model.is_some() {
            model = resolved.model;
            if args.thinking.is_none() {
                thinking = resolved.thinking_level;
            }
        }
    }
    let continuing = session_model.is_some();
    if model.is_none() && !scoped.is_empty() && !continuing {
        let saved = match (settings.default_provider(), settings.default_model()) {
            (Some(p), Some(id)) => registry.get_model(p, id),
            _ => None,
        };
        let pick = saved
            .and_then(|s| {
                scoped
                    .iter()
                    .find(|sm| sm.model.provider == s.provider && sm.model.id == s.id)
            })
            .unwrap_or(&scoped[0]);
        model = Some(pick.model.clone());
        if args.thinking.is_none() && pick.thinking_level.is_some() {
            thinking = pick.thinking_level.clone();
        }
    }
    if let Some(level) = &args.thinking {
        thinking = Some(level.clone());
    }
    if args.api_key.is_some() {
        match &model {
            None => diagnostics.push(Diagnostic::error(
                "--api-key requires a model to be specified via --model, --provider/--model, or --models",
            )),
            Some(m) => registry.set_runtime_api_key(&m.provider, args.api_key.as_deref().unwrap_or_default()),
        }
    }

    if model.is_none() {
        model = session_model;
    }
    if model.is_none() {
        let available = registry.available();
        let initial = find_initial_model(
            &[],
            &InitialModelSources {
                default_provider: settings.default_provider(),
                default_model_id: settings.default_model(),
                default_thinking: settings.default_thinking_level(),
                model_thinking: &|p, id| settings.model_thinking_level(p, id).map(str::to_string),
                get_model: &|p, id| registry.get_model(p, id).cloned(),
                has_auth: &has_auth,
                available: &available,
            },
        );
        model = initial.model;
    }

    let mut level = thinking.as_deref().and_then(parse_level);
    if level.is_none() && continuing {
        level = session_thinking;
    }
    if level.is_none()
        && let Some(m) = &model
    {
        level = settings
            .model_thinking_level(&m.provider, &m.id)
            .and_then(parse_level);
    }
    let level = level
        .or_else(|| settings.default_thinking_level().and_then(parse_level))
        .or_else(|| parse_level(DEFAULT_THINKING_LEVEL))
        .unwrap_or(ModelThinkingLevel::Medium);
    let thinking = match &model {
        None => ModelThinkingLevel::Off,
        Some(m) => clamp_thinking_level(m, level),
    };
    ModelChoice {
        model,
        thinking,
        diagnostics,
    }
}

pub fn unknown_model() -> Model {
    Model::new("unknown", "unknown", "unknown", 0, 0, false)
}

pub fn is_unknown_model(model: &Model) -> bool {
    model.provider == "unknown" && model.id == "unknown" && model.api == "unknown"
}

/// The models a session may switch to: every model the catalog has.
pub fn available(registry: &ModelRegistry) -> Vec<Model> {
    registry.available().into_iter().cloned().collect()
}

/// The session's scoped models: `enabledModels` resolved over the available models, or none when
/// every model is enabled.
pub fn scope(settings: &SettingsManager, available: &[Model]) -> Vec<Model> {
    match settings.enabled_models() {
        Some(patterns) if !patterns.is_empty() => resolve_model_scope(&patterns, available)
            .0
            .into_iter()
            .map(|scoped| scoped.model)
            .collect(),
        _ => Vec::new(),
    }
}

/// Point the session at `model`, clamping the thinking level to what it accepts: the per-model
/// setting, else `thinking`. Hands back the level in effect.
pub fn set_model(
    root: &Conversation,
    cancel: &CancelToken,
    settings: &SettingsManager,
    model: &Model,
    thinking: ModelThinkingLevel,
) -> Result<ModelThinkingLevel, String> {
    let override_level = settings
        .model_thinking_level(&model.provider, &model.id)
        .and_then(parse_level);
    let level = clamp_thinking_level(model, override_level.unwrap_or(thinking));
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: model.provider.clone(),
                model_id: model.id.clone(),
            }),
            thinking_level: Change::Set(level),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;
    Ok(level)
}

/// The model and thinking level a session recorded for itself, so switching to it does not
/// rewrite what it stored. None when it recorded none, or recorded a model the catalog lacks.
pub fn recorded_agent(
    root: &Conversation,
    cancel: &CancelToken,
    registry: &ModelRegistry,
) -> Result<Option<(Model, ModelThinkingLevel)>, String> {
    let agent = root.agent(cancel).map_err(|e| e.to_string())?;
    let Some(reference) = agent.model else {
        return Ok(None);
    };
    let Some(found) = registry
        .get_model(&reference.provider, &reference.model_id)
        .cloned()
    else {
        return Ok(None);
    };
    let level = clamp_thinking_level(&found, agent.thinking_level);
    Ok(Some((found, level)))
}

/// Point the session's thinking level at `level`.
pub fn set_thinking(
    root: &Conversation,
    cancel: &CancelToken,
    level: ModelThinkingLevel,
) -> Result<(), String> {
    root.configure(
        &AgentChange {
            thinking_level: Change::Set(level),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())
}

/// Point a session's agent at the loop's model, thinking level and cwd, as upstream does for a
/// session it creates or switches to.
pub fn apply_agent(
    root: &Conversation,
    cancel: &CancelToken,
    model: &Model,
    thinking: ModelThinkingLevel,
    cwd: &str,
) -> Result<(), String> {
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: model.provider.clone(),
                model_id: model.id.clone(),
            }),
            thinking_level: Change::Set(clamp_thinking_level(model, thinking)),
            cwd: Change::Set(cwd.to_owned()),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())
}
