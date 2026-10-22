//! `core/settings-manager.ts` (reading only) and the project trust decision of
//! `core/project-trust.ts` and `core/trust-manager.ts`.
//!
//! Settings come from `<agentDir>/settings.json` and, in a trusted project,
//! `<cwd>/.pi/settings.json`, deep-merged with the project winning. Print mode never writes them.

use crate::config::CONFIG_DIR_NAME;
use crate::paths::{PathInput, home_dir, join, normalize_path, resolve};
use serde_json::{Map, Value};

/// Tools on when `defaultTools` does not change them.
pub const DEFAULT_TOOL_NAMES: [&str; 4] = ["read", "bash", "edit", "write"];

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub merged: Map<String, Value>,
    pub project_trusted: bool,
    /// `Invalid settings file <path>: <message>` warnings, as upstream prints them.
    pub warnings: Vec<String>,
}

fn is_modifier(entry: &str) -> bool {
    entry.starts_with('+') || entry.starts_with('-')
}

fn deep_merge(base: &Map<String, Value>, overrides: &Map<String, Value>) -> Map<String, Value> {
    let mut out = base.clone();
    for (key, value) in overrides {
        let merged = match (base.get(key), value) {
            (Some(Value::Object(a)), Value::Object(b)) => Value::Object(deep_merge(a, b)),
            _ => value.clone(),
        };
        out.insert(key.clone(), merged);
    }
    out
}

/// `deepMergeSettings`: a `defaultTools` list of only `+name`/`-name` entries extends the inherited one.
fn merge_settings(base: &Map<String, Value>, overrides: &Map<String, Value>) -> Map<String, Value> {
    let mut merged = deep_merge(base, overrides);
    if let (Some(Value::Array(a)), Some(Value::Array(b))) =
        (base.get("defaultTools"), overrides.get("defaultTools"))
        && b.iter().all(|e| e.as_str().is_some_and(is_modifier)) {
            let mut both = a.clone();
            both.extend(b.iter().cloned());
            merged.insert("defaultTools".to_owned(), Value::Array(both));
        }
    merged
}

/// `migrateSettings`, for the keys print mode reads.
fn migrate(mut settings: Map<String, Value>) -> Map<String, Value> {
    if settings.contains_key("queueMode") && !settings.contains_key("steeringMode")
        && let Some(v) = settings.remove("queueMode") {
            settings.insert("steeringMode".to_owned(), v);
        }
    if !settings.contains_key("transport")
        && let Some(Value::Bool(ws)) = settings.get("websockets").cloned() {
            settings.insert(
                "transport".to_owned(),
                Value::String(if ws { "websocket" } else { "sse" }.to_owned()),
            );
            settings.remove("websockets");
        }
    settings
}

/// Reads one settings file: `Ok(None)` when it does not exist, `Err(message)` when it is not a
/// JSON object.
fn load(path: &str) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(crate::node_fs::message(&e, "open", path)),
    };
    if text.is_empty() {
        return Ok(Map::new());
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match crate::json::parse(text)? {
        Value::Object(map) => Ok(migrate(map)),
        // `JSON.parse` takes any JSON value; a non-object reads as no settings.
        _ => Ok(Map::new()),
    }
}

impl Settings {
    /// `SettingsManager.create(cwd, agentDir, { projectTrusted })`.
    pub fn load(cwd: &str, agent_dir: &str, project_trusted: bool) -> Self {
        let global_path = join(agent_dir, "settings.json");
        let project_path = join(cwd, &format!("{CONFIG_DIR_NAME}/settings.json"));
        let mut warnings = Vec::new();
        let mut read = |path: &str| {
            load(path).unwrap_or_else(|e| {
                warnings.push(format!("Invalid settings file {path}: {e}"));
                Map::new()
            })
        };
        let global = read(&global_path);
        let project = if project_trusted {
            read(&project_path)
        } else {
            Map::new()
        };
        Settings {
            merged: merge_settings(&global, &project),
            project_trusted,
            warnings,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.merged.get(key).and_then(Value::as_str)
    }

    pub fn default_provider(&self) -> Option<&str> {
        self.str("defaultProvider")
    }

    pub fn default_model(&self) -> Option<&str> {
        self.str("defaultModel")
    }

    pub fn default_thinking_level(&self) -> Option<&str> {
        self.str("defaultThinkingLevel")
    }

    /// `modelThinkingLevels["provider/id"]`.
    pub fn model_thinking_level(&self, provider: &str, id: &str) -> Option<&str> {
        self.merged
            .get("modelThinkingLevels")?
            .get(format!("{provider}/{id}"))?
            .as_str()
    }

    pub fn shell_path(&self) -> Option<String> {
        self.str("shellPath")
            .filter(|s| !s.is_empty())
            .map(|s| normalize_path(s, PathInput::default()))
    }

    pub fn shell_command_prefix(&self) -> Option<&str> {
        self.str("shellCommandPrefix").filter(|s| !s.is_empty())
    }

    pub fn enabled_models(&self) -> Option<Vec<String>> {
        Some(
            self.merged
                .get("enabledModels")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
        )
    }

    pub fn block_images(&self) -> bool {
        self.merged
            .get("images")
            .and_then(|i| i.get("blockImages"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// `getDefaultTools()`: the resolved selection, or None when no layer sets `defaultTools`.
    pub fn default_tools(&self) -> Option<Vec<String>> {
        let entries: Vec<String> = match self.merged.get("defaultTools")? {
            Value::Array(items) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        let plain: Vec<String> = entries
            .iter()
            .filter(|e| !is_modifier(e))
            .cloned()
            .collect();
        let base = if !plain.is_empty() || entries.is_empty() {
            plain
        } else {
            DEFAULT_TOOL_NAMES.iter().map(|s| (*s).to_owned()).collect()
        };
        Some(apply_tool_modifiers(base, &entries))
    }

    /// `defaultProjectTrust`: `ask` (the default), `always` or `never`.
    pub fn default_project_trust(&self) -> &str {
        self.str("defaultProjectTrust").unwrap_or("ask")
    }
}

/// `applyToolModifiers`.
pub fn apply_tool_modifiers(mut tools: Vec<String>, entries: &[String]) -> Vec<String> {
    for entry in entries.iter().filter(|e| is_modifier(e)) {
        let name = &entry[1..];
        let index = tools.iter().position(|t| t == name);
        if entry.starts_with('+') && index.is_none() && !name.is_empty() {
            tools.push(name.to_owned());
        } else if let (true, Some(i)) = (entry.starts_with('-'), index) {
            tools.remove(i);
        }
    }
    tools
}

/// `canonicalizePath`: the real path, or the path as given when it cannot be resolved.
pub fn canonicalize(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned())
}

const TRUST_REQUIRING: [&str; 8] = [
    "settings.json",
    "mcp.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

/// `hasTrustRequiringProjectResources`: project config that runs code or changes the prompt, or a
/// `.agents/skills` directory in the cwd or an ancestor (other than the user's own).
pub fn has_trust_requiring_resources(cwd: &str) -> bool {
    let home = canonicalize(&resolve("/", &home_dir()));
    let user_skills = join(&home, ".agents/skills");
    let mut current = canonicalize(&resolve("/", cwd));
    let config = join(&current, CONFIG_DIR_NAME);
    if TRUST_REQUIRING
        .iter()
        .any(|entry| std::path::Path::new(&join(&config, entry)).exists())
    {
        return true;
    }
    loop {
        let skills = join(&current, ".agents/skills");
        if skills != user_skills && std::path::Path::new(&skills).exists() {
            return true;
        }
        let parent = crate::paths::dirname(&current);
        if parent == current {
            return false;
        }
        current = parent;
    }
}

/// The nearest decision in `<agentDir>/trust.json` for `cwd` or an ancestor.
pub fn stored_trust(agent_dir: &str, cwd: &str) -> Result<Option<bool>, String> {
    let path = join(agent_dir, "trust.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Ok(None),
    };
    let parsed = crate::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
        .map_err(|e| format!("Failed to read trust store {path}: {e}"))?;
    let Value::Object(data) = parsed else {
        return Err(format!("Invalid trust store {path}: expected an object"));
    };
    let mut current = canonicalize(&resolve("/", cwd));
    loop {
        if let Some(Value::Bool(decision)) = data.get(&current) {
            return Ok(Some(*decision));
        }
        let parent = crate::paths::dirname(&current);
        if parent == current {
            return Ok(None);
        }
        current = parent;
    }
}

/// Whether project resources load, without a UI to ask (print mode): `--approve`/`--no-approve`,
/// else trusted when nothing needs trust, else the stored decision, else `defaultProjectTrust`
/// (`ask` with no UI means no).
pub fn resolve_project_trust(
    cwd: &str,
    agent_dir: &str,
    overridden: Option<bool>,
    default_trust: &str,
) -> Result<bool, String> {
    if let Some(trusted) = overridden {
        return Ok(trusted);
    }
    if !has_trust_requiring_resources(cwd) {
        return Ok(true);
    }
    if let Some(decision) = stored_trust(agent_dir, cwd)? {
        return Ok(decision);
    }
    Ok(default_trust == "always")
}
