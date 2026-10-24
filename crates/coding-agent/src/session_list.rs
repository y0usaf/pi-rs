//! Where pi-rs keeps session files: one sqlite database per session under
//! `<agentDir>/sessions/--<cwd>--`, upstream's `getDefaultSessionDirPath`.

use crate::args::Args;
use crate::paths::{PathInput, join, normalize_path};
use crate::settings::SettingsManager;

/// `<agentDir>/sessions/--<cwd with / as ->--`, upstream's session directory for a cwd.
pub fn dir(agent_dir: &str, cwd: &str, args: &Args, settings: &SettingsManager) -> String {
    if let Some(dir) = &args.session_dir {
        return normalize_path(dir, PathInput::default());
    }
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_SESSION_DIR")
        && !dir.is_empty()
    {
        return normalize_path(&dir, PathInput::default());
    }
    if let Some(dir) = settings.session_dir() {
        return dir;
    }
    let safe = format!(
        "--{}--",
        cwd.trim_start_matches('/').replace(['/', '\\', ':'], "-")
    );
    join(agent_dir, &format!("sessions/{safe}"))
}

/// A new session file name: a time-ordered id, as upstream's `createSessionId` (`uuidv7`).
pub fn new_path(dir: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let id =
        pi_durable::ai::utils::uuid::uuidv7(Some(millis)).unwrap_or_else(|_| millis.to_string());
    join(dir, &format!("{id}.db"))
}

/// The newest `*.db` session in `dir`.
pub fn latest(dir: &str) -> Option<String> {
    let mut found: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".db"))
        .filter_map(|e| {
            Some((
                e.metadata().ok()?.modified().ok()?,
                e.path().to_string_lossy().into_owned(),
            ))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}
