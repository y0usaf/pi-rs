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

/// A session file as the picker lists it: what the file says about itself plus what the
/// filesystem knows.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEntry {
    /// The file stem, which is the session id.
    pub id: String,
    pub path: String,
    pub modified: std::time::SystemTime,
    /// Empty for a session that never recorded one.
    pub cwd: String,
    pub name: Option<String>,
    /// The session this one was forked from.
    pub parent: Option<String>,
    pub message_count: u64,
    /// The first user message, or `(no messages)` as upstream shows an empty session.
    pub first_message: String,
    /// Every user and assistant message, for the picker's search.
    pub all_text: String,
}

fn read(path: &std::path::Path, modified: std::time::SystemTime) -> Option<SessionEntry> {
    let overview = pi_durable::storage::sqlite::read_session_overview(path).ok()?;
    let info = overview
        .documents
        .get(crate::session_info::KIND)
        .and_then(|value| {
            serde_json::from_value::<crate::session_info::SessionInfoState>(
                pi_durable::types::JsonValue::Object(value.clone()),
            )
            .ok()
        })
        .unwrap_or_default();
    Some(SessionEntry {
        id: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.to_string_lossy().into_owned(),
        modified,
        cwd: overview.cwd.unwrap_or_default(),
        name: info
            .name
            .map(|name| name.trim().to_owned())
            .filter(|n| !n.is_empty()),
        parent: info.parent_session,
        message_count: overview.message_count,
        first_message: overview
            .first_user_text
            .unwrap_or_else(|| "(no messages)".to_owned()),
        all_text: overview.all_text,
    })
}

/// The session files directly under `dir`, newest first. A file another pi-rs holds open, or one
/// that is not a pi-rs session, is skipped rather than waited on.
pub fn sessions(dir: &str) -> Vec<SessionEntry> {
    let mut found: Vec<SessionEntry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".db"))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            read(&entry.path(), modified)
        })
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    found
}

/// Every session file under `<agentDir>/sessions`, across every folder's directory, newest
/// first.
pub fn all(agent_dir: &str) -> Vec<SessionEntry> {
    let root = join(agent_dir, "sessions");
    let mut found: Vec<SessionEntry> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .flat_map(|entry| sessions(&entry.path().to_string_lossy()))
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    found
}

/// Delete a session file: through trash(1) when it is there, else unlinking it. Ok(true) when it
/// went to the trash.
pub fn delete(path: &str) -> Result<bool, String> {
    let trashed = std::process::Command::new("trash")
        .arg(path)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if trashed || !std::path::Path::new(path).exists() {
        return Ok(trashed);
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    Ok(false)
}

/// Rename a session file that is not open: a harness on it, the name written, the harness closed.
pub fn rename(path: &str, name: &str) -> Result<(), String> {
    let cancel = pi_durable::cancel::CancelToken::new();
    let storage: std::sync::Arc<dyn pi_durable::types::Storage> = std::sync::Arc::new(
        pi_durable::storage::sqlite::SqliteStorage::open(path, &Default::default())
            .map_err(|e| e.to_string())?,
    );
    let settings = crate::print::ToolSettings {
        cwd: String::new(),
        shell_path: None,
        command_prefix: None,
        env: Vec::new(),
        model_takes_images: false,
    };
    let harness = crate::print::open_harness(
        storage,
        pi_durable::ai::registry::Providers::builtin(),
        crate::print::extension(&[], Vec::new(), settings),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let outcome = crate::print::root(&harness, &cancel)
        .map_err(|e| e.to_string())
        .and_then(|root| {
            crate::session_info::set_name(&root, &cancel, name).map_err(|e| e.to_string())
        });
    crate::print::close(&harness, &cancel).map_err(|e| e.to_string())?;
    outcome
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
