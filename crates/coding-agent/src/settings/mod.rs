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

/// The nearest decision in `<agentDir>/trust.json` for `cwd` or an ancestor.
pub fn stored_trust(agent_dir: &str, cwd: &str) -> Result<Option<bool>, String> {
    let path = join(agent_dir, "trust.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Ok(None),
    };
    let parsed = crate::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
        .map_err(|error| format!("Failed to read trust store {path}: {error}"))?;
    let serde_json::Value::Object(data) = parsed else {
        return Err(format!("Invalid trust store {path}: expected an object"));
    };
    let mut current = canonicalize(&resolve("/", cwd));
    loop {
        if let Some(serde_json::Value::Bool(decision)) = data.get(&current) {
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
