//! `config.ts`: where the agent keeps its files.

use crate::paths::{PathInput, home_dir, join, normalize_path};

/// The environment variable naming the agent directory.
pub const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";
pub const CONFIG_DIR_NAME: &str = ".pi";
/// The upstream version this crate follows; `--version` prints it.
pub const VERSION: &str = "1.4.0";
pub const APP_TITLE: &str = "π";

/// `getAgentDir()`: `$PI_CODING_AGENT_DIR` (with `~` expanded), else `~/.pi/agent`.
pub fn agent_dir() -> String {
    match std::env::var(ENV_AGENT_DIR) {
        Ok(dir) if !dir.is_empty() => normalize_path(&dir, PathInput::default()),
        _ => join(&home_dir(), &format!("{CONFIG_DIR_NAME}/agent")),
    }
}

/// `getBinDir()`: where pi keeps the fd and rg it downloads.
pub fn bin_dir() -> String {
    join(&agent_dir(), "bin")
}

fn on_path(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// `getToolPath()`: fd or rg from the agent's bin directory, else the command name when it runs
/// from PATH. Upstream downloads a missing tool; this port does not, and reports it missing.
pub fn tool_path(tool: &str) -> Option<String> {
    let local = join(&bin_dir(), tool);
    if std::path::Path::new(&local).exists() {
        return Some(local);
    }
    let names: &[&str] = if tool == "fd" {
        &["fd", "fdfind"]
    } else {
        &["rg"]
    };
    names.iter().find(|n| on_path(n)).map(|n| (*n).to_owned())
}
