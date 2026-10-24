//! `core/settings-manager.ts`, `core/settings-schema.ts` and
//! `core/settings-diagnostics.ts`, and the project trust decision of `core/project-trust.ts`
//! and `core/trust-manager.ts`.

mod diagnostics;
mod manager;
mod schema;

pub use diagnostics::{
    DiagnosticKind, SettingsDiagnostic, collect_diagnostics, deduplicate_diagnostics,
};
pub use manager::{
    DEFAULT_TOOL_NAMES, FileSettingsStorage, InMemorySettingsStorage, Settings, SettingsError,
    SettingsManager, SettingsScope, SettingsStorage, Update, apply_tool_modifiers,
};
pub use schema::{
    BranchSummarySettings, CompactionSettings, ProviderRetrySettings, QuietStartup, RetrySettings,
    ThinkingBudgets, WarningSettings, WheelScrollLines, defaults,
};

use crate::config::CONFIG_DIR_NAME;
use crate::paths::{home_dir, join, resolve};

/// `canonicalizePath`: the real path, or the path as given when it cannot be resolved.
pub fn canonicalize(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
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

/// `hasTrustRequiringProjectResources`: project config that runs code or changes the prompt, or
/// a `.agents/skills` directory in the cwd or an ancestor (other than the user's own).
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

fn read_trust_store(
    agent_dir: &str,
) -> Result<(String, serde_json::Map<String, serde_json::Value>), String> {
    let path = join(agent_dir, "trust.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Ok((path, serde_json::Map::new())),
    };
    let parsed = crate::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
        .map_err(|error| format!("Failed to read trust store {path}: {error}"))?;
    let serde_json::Value::Object(data) = parsed else {
        return Err(format!("Invalid trust store {path}: expected an object"));
    };
    Ok((path, data))
}

/// The nearest `(path, decision)` in `<agentDir>/trust.json` for `cwd` or an ancestor.
pub fn stored_trust_entry(agent_dir: &str, cwd: &str) -> Result<Option<(String, bool)>, String> {
    let (_, data) = read_trust_store(agent_dir)?;
    let mut current = canonicalize(&resolve("/", cwd));
    loop {
        if let Some(serde_json::Value::Bool(decision)) = data.get(&current) {
            return Ok(Some((current, *decision)));
        }
        let parent = crate::paths::dirname(&current);
        if parent == current {
            return Ok(None);
        }
        current = parent;
    }
}

/// The nearest decision in `<agentDir>/trust.json` for `cwd` or an ancestor.
pub fn stored_trust(agent_dir: &str, cwd: &str) -> Result<Option<bool>, String> {
    Ok(stored_trust_entry(agent_dir, cwd)?.map(|(_, decision)| decision))
}

/// One choice in the /trust overlay: the paths it writes and the decision it stands for.
pub struct TrustOption {
    pub label: String,
    pub trusted: bool,
    /// `(path, decision)`; `None` clears the entry.
    pub updates: Vec<(String, Option<bool>)>,
    pub saved_path: Option<String>,
}

/// `getProjectTrustOptions`: trust this folder, trust its parent, or do not trust.
pub fn project_trust_options(cwd: &str, include_parent: bool) -> Vec<TrustOption> {
    let path = canonicalize(&resolve("/", cwd));
    let mut options = vec![TrustOption {
        label: "Trust".to_string(),
        trusted: true,
        updates: vec![(path.clone(), Some(true))],
        saved_path: Some(path.clone()),
    }];
    let parent = crate::paths::dirname(&path);
    if include_parent && parent != path {
        options.push(TrustOption {
            label: format!("Trust parent folder ({parent})"),
            trusted: true,
            updates: vec![(parent.clone(), Some(true)), (path.clone(), None)],
            saved_path: Some(parent),
        });
    }
    options.push(TrustOption {
        label: "Do not trust".to_string(),
        trusted: false,
        updates: vec![(path.clone(), Some(false))],
        saved_path: Some(path),
    });
    options
}

/// `ProjectTrustStore.setMany`: apply decisions, `None` clearing an entry, and write the store
/// sorted by path.
pub fn write_trust(agent_dir: &str, updates: &[(String, Option<bool>)]) -> Result<(), String> {
    let (path, mut data) = read_trust_store(agent_dir)?;
    for (entry, decision) in updates {
        match decision {
            Some(decision) => {
                data.insert(entry.clone(), serde_json::Value::Bool(*decision));
            }
            None => {
                data.remove(entry);
            }
        }
    }
    let sorted: serde_json::Map<String, serde_json::Value> = {
        let mut keys: Vec<&String> = data.keys().collect();
        keys.sort();
        keys.into_iter()
            .map(|key| (key.clone(), data[key].clone()))
            .collect()
    };
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(sorted))
        .map_err(|e| e.to_string())?;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, text).map_err(|e| format!("Failed to write trust store {path}: {e}"))
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
