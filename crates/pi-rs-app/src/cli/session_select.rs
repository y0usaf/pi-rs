//! Port of `main.ts`'s session-selection slice — `resolveSessionPath`
//! and the `createSessionManager` routing for the landed flags
//! (`--continue`, `--session`). The `--resume` selector
//! (`cli/session-picker.ts` + `SessionSelectorComponent`) lands with the
//! session-UI rung (PLAN 6.3), which pins the shared selector with
//! Pi-derived frame fixtures.
//!
//! This module returns data; prompts, colored output, and exit codes
//! stay in `main.rs` (the spec's side effects live in `main()`).

use std::path::Path;

use pi_rs_session::paths::{normalize_path, resolve_path, resolve_path_in};
use pi_rs_session::{SessionManager, find_most_recent_session, load_entries_from_file};

/// Spec: `ResolvedSession`.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedSession {
    /// Direct file path.
    Path(String),
    /// Found in current project.
    Local(String),
    /// Found in different project.
    Global { path: String, cwd: String },
    /// Not found anywhere.
    NotFound(String),
}

/// Spec: `resolveSessionPath(sessionArg, cwd, sessionDir)`.
pub fn resolve_session_path(
    session_arg: &str,
    cwd: &str,
    session_dir: Option<&str>,
    agent_dir: &str,
) -> ResolvedSession {
    // If it looks like a file path, resolve it before handing it to the
    // session manager.
    if session_arg.contains('/') || session_arg.contains('\\') || session_arg.ends_with(".jsonl") {
        return ResolvedSession::Path(resolve_path_in(session_arg, cwd));
    }

    // Try to match as session ID in current project first.
    let local_sessions =
        SessionManager::list(cwd, session_dir, agent_dir, None).unwrap_or_default();
    let local_match = local_sessions
        .iter()
        .find(|s| s.id == session_arg)
        .or_else(|| {
            local_sessions
                .iter()
                .find(|s| s.id.starts_with(session_arg))
        });
    if let Some(found) = local_match {
        return ResolvedSession::Local(found.path.to_string_lossy().into_owned());
    }

    // Try global search across all projects (spec: `getSessionsDir()` =
    // `{agentDir}/sessions`).
    let all_sessions = SessionManager::list_all(
        session_dir,
        &Path::new(&resolve_path(agent_dir)).join("sessions"),
        None,
    );
    let global_match = all_sessions
        .iter()
        .find(|s| s.id == session_arg)
        .or_else(|| all_sessions.iter().find(|s| s.id.starts_with(session_arg)));
    if let Some(found) = global_match {
        return ResolvedSession::Global {
            path: found.path.to_string_lossy().into_owned(),
            cwd: found.cwd.clone(),
        };
    }

    ResolvedSession::NotFound(session_arg.to_owned())
}

/// The `createSessionManager` outcome for the landed flags, as data the
/// caller turns into session construction (in the Lua packs), prompts,
/// or exits.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionChoice {
    /// Open this session file (`SessionManager.open`).
    Open { path: String },
    /// Start a new session (`SessionManager.create`).
    Create,
    /// `--session` matched a session from another project: the caller
    /// confirms before forking it into the current directory.
    ConfirmFork { path: String, cwd: String },
    /// `--session` matched nothing.
    NotFound { arg: String },
}

/// Spec: `createSessionManager(parsed, cwd, sessionDir)` for the landed
/// flags (`--session` before `--continue`, then the default create).
pub fn choose_session(
    continue_recent: bool,
    session_arg: Option<&str>,
    cwd: &str,
    session_dir: Option<&str>,
    agent_dir: &str,
) -> SessionChoice {
    if let Some(arg) = session_arg {
        return match resolve_session_path(arg, cwd, session_dir, agent_dir) {
            ResolvedSession::Path(path) | ResolvedSession::Local(path) => {
                SessionChoice::Open { path }
            }
            ResolvedSession::Global { path, cwd } => SessionChoice::ConfirmFork { path, cwd },
            ResolvedSession::NotFound(arg) => SessionChoice::NotFound { arg },
        };
    }

    if continue_recent {
        // Spec: `SessionManager.continueRecent` — most recent session in
        // the effective session dir (cwd-filtered only for a custom,
        // non-default dir), or a new session when none exists.
        let dir = match session_dir {
            Some(dir) => normalize_path(dir),
            None => match pi_rs_session::get_default_session_dir(cwd, agent_dir) {
                Ok(dir) => dir.to_string_lossy().into_owned(),
                Err(_) => return SessionChoice::Create,
            },
        };
        let filter_cwd = session_dir.is_some()
            && Path::new(&dir) != pi_rs_session::get_default_session_dir_path(cwd, agent_dir);
        return match find_most_recent_session(Path::new(&dir), filter_cwd.then_some(cwd)) {
            Some(path) => SessionChoice::Open {
                path: path.to_string_lossy().into_owned(),
            },
            None => SessionChoice::Create,
        };
    }

    SessionChoice::Create
}

/// The session header's `cwd` — the effective runtime cwd after opening
/// a session (spec: `sessionManager.getCwd()` feeds every cwd-bound
/// runtime service). Falls back to the process cwd like the spec's
/// header-less open.
pub fn session_header_cwd(path: &str) -> Option<String> {
    let entries = load_entries_from_file(Path::new(&resolve_path(path)));
    entries
        .iter()
        .find(|entry| entry.get("type").and_then(|t| t.as_str()) == Some("session"))
        .and_then(|header| header.get("cwd").and_then(|cwd| cwd.as_str()))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn write_session(dir: &Path, name: &str, id: &str, cwd: &str) -> String {
        let path = dir.join(name);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "session", "version": 3, "id": id,
                    "timestamp": "2026-01-01T00:00:00.000Z", "cwd": cwd,
                })
            ),
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn session_arg_with_path_separator_resolves_as_path() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy().into_owned();
        assert_eq!(
            resolve_session_path("sessions/a.jsonl", &cwd, None, &cwd),
            ResolvedSession::Path(resolve_path_in("sessions/a.jsonl", &cwd)),
        );
    }

    #[test]
    fn session_id_prefix_matches_local_then_global() {
        let temp = tempfile::tempdir().unwrap();
        let agent_dir = temp.path().join("agent");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.to_string_lossy().into_owned();
        let agent_dir_string = agent_dir.to_string_lossy().into_owned();
        let local_dir = pi_rs_session::get_default_session_dir(&cwd, &agent_dir_string).unwrap();
        let local = write_session(&local_dir, "a.jsonl", "aaaa1111", &cwd);

        let other_cwd = temp.path().join("other").to_string_lossy().into_owned();
        let other_dir =
            pi_rs_session::get_default_session_dir(&other_cwd, &agent_dir_string).unwrap();
        let global = write_session(&other_dir, "b.jsonl", "bbbb2222", &other_cwd);

        assert_eq!(
            resolve_session_path("aaaa", &cwd, None, &agent_dir_string),
            ResolvedSession::Local(local),
        );
        assert_eq!(
            resolve_session_path("bbbb", &cwd, None, &agent_dir_string),
            ResolvedSession::Global {
                path: global,
                cwd: resolve_path(&other_cwd),
            },
        );
        assert_eq!(
            resolve_session_path("cccc", &cwd, None, &agent_dir_string),
            ResolvedSession::NotFound("cccc".to_owned()),
        );
    }

    #[test]
    fn continue_picks_most_recent_or_creates() {
        let temp = tempfile::tempdir().unwrap();
        let agent_dir = temp.path().join("agent").to_string_lossy().into_owned();
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.to_string_lossy().into_owned();

        assert_eq!(
            choose_session(true, None, &cwd, None, &agent_dir),
            SessionChoice::Create
        );

        let dir = pi_rs_session::get_default_session_dir(&cwd, &agent_dir).unwrap();
        let older = write_session(&dir, "older.jsonl", "aaaa1111", &cwd);
        let newer = write_session(&dir, "newer.jsonl", "bbbb2222", &cwd);
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        let file = std::fs::File::options().append(true).open(&older).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(past))
            .unwrap();
        assert_eq!(
            choose_session(true, None, &cwd, None, &agent_dir),
            SessionChoice::Open { path: newer },
        );
    }

    #[test]
    fn header_cwd_reads_the_session_header() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_session(temp.path(), "s.jsonl", "aaaa1111", "/home/user/project");
        assert_eq!(
            session_header_cwd(&path).as_deref(),
            Some("/home/user/project")
        );
        assert_eq!(session_header_cwd("/nonexistent/x.jsonl"), None);
    }
}
