//! Port of `config.ts` — app identity and user-config paths.
//!
//! WS2.6 subset: the bare binary needs the agent dir, auth/models paths,
//! and the docs path (auth guidance). The install-method /
//! self-update half of the spec file is npm machinery with no Rust
//! analogue yet; it lands with WS9 periphery if it survives the port.
//!
//! Identity: Pi derives `APP_NAME`/`CONFIG_DIR_NAME` from `package.json`
//! `piConfig`; pi-rs uses the same `pi` / `.pi` runtime identity and
//! environment-variable names so it reads Pi's existing configuration.

use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "pi";
pub const CONFIG_DIR_NAME: &str = ".pi";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Spec: `ENV_AGENT_DIR` = `${APP_NAME.toUpperCase()}_CODING_AGENT_DIR`.
pub const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";
/// Spec: `ENV_SESSION_DIR` (consumed by WS3 sessions).
pub const ENV_SESSION_DIR: &str = "PI_CODING_AGENT_SESSION_DIR";

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_owned()))
}

/// Spec: `expandTildePath` (via `normalizePath`) — `~` and `~/...`
/// expand to the home directory.
pub fn expand_tilde_path(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// Spec: `getPackageDir()` — env override (`PI_PACKAGE_DIR`), else the
/// directory containing the executable (the spec's compiled-binary
/// branch; pi-rs is always a compiled binary).
pub fn get_package_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("PI_PACKAGE_DIR")
        && !env_dir.is_empty()
    {
        return expand_tilde_path(&env_dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Spec: `getDocsPath()`.
pub fn get_docs_path() -> PathBuf {
    get_package_dir().join("docs")
}

/// Spec: `getReadmePath()`.
pub fn get_readme_path() -> PathBuf {
    get_package_dir().join("README.md")
}

/// Spec: `getExamplesPath()`.
pub fn get_examples_path() -> PathBuf {
    get_package_dir().join("examples")
}

/// Spec: `getAgentDir()` — e.g. `~/.pi/agent/`.
pub fn get_agent_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var(ENV_AGENT_DIR)
        && !env_dir.is_empty()
    {
        return expand_tilde_path(&env_dir);
    }
    home_dir().join(CONFIG_DIR_NAME).join("agent")
}

/// Spec: `getAuthPath()`.
pub fn get_auth_path() -> PathBuf {
    get_agent_dir().join("auth.json")
}

/// Spec: `getModelsPath()`.
pub fn get_models_path() -> PathBuf {
    get_agent_dir().join("models.json")
}

/// Spec: `getSessionsDir()` — root of the per-cwd session directories
/// (consumed by `SessionManager.listAll`).
pub fn get_sessions_dir() -> PathBuf {
    get_agent_dir().join("sessions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expansion() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
        assert_eq!(expand_tilde_path("~"), PathBuf::from(&home));
        assert_eq!(expand_tilde_path("~/x"), PathBuf::from(&home).join("x"));
        assert_eq!(expand_tilde_path("/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn runtime_identity_matches_pi() {
        assert_eq!(APP_NAME, "pi");
        assert_eq!(CONFIG_DIR_NAME, ".pi");
        assert_eq!(ENV_AGENT_DIR, "PI_CODING_AGENT_DIR");
        assert_eq!(ENV_SESSION_DIR, "PI_CODING_AGENT_SESSION_DIR");
    }
}
