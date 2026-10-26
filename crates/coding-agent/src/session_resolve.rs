//! `--session` and `--fork` (`main.ts` `resolveSessionPath`): a path-like argument names a file,
//! any other names a session by id, in this project first and then in every project.

use std::path::Path;

use crate::paths::{PathInput, join, normalize_path};
use crate::session_list::{self, Dir, SessionEntry};

/// What an argument names (`main.ts` `ResolvedSession`).
pub enum Resolved {
    /// A file path, as given.
    Path(String),
    /// A session of this project.
    Local(String),
    /// A session of another project, and the project it belongs to.
    Global { path: String, cwd: String },
    /// No session matches.
    NotFound,
}

/// `resolveSessionPath`: a `/`, a `\\` or a `.jsonl` ending makes the argument a path. Any other is
/// a session id, matched exactly first and then as a prefix, in this project's session directory
/// and then in every project's. A match never creates a file.
pub fn resolve(dir: &Dir, agent_dir: &str, cwd: &str, arg: &str) -> Resolved {
    if arg.contains(['/', '\\']) || arg.ends_with(".jsonl") {
        return Resolved::Path(normalize_path(arg, PathInput::default()));
    }
    if !dir.custom {
        let exact = join(&dir.path, &format!("{arg}.db"));
        if Path::new(&exact).is_file() {
            return Resolved::Local(exact);
        }
    }
    let listed = session_list::sessions(&dir.path);
    let here = normalize_path(cwd, PathInput::default());
    let mine = |entry: &SessionEntry| {
        !dir.custom
            || (!entry.cwd.is_empty() && normalize_path(&entry.cwd, PathInput::default()) == here)
    };
    if let Some(found) = find(listed.iter().filter(|entry| mine(entry)), arg) {
        return Resolved::Local(found.path.clone());
    }
    let all = if dir.custom {
        listed
    } else {
        session_list::all(agent_dir)
    };
    match find(all.iter(), arg) {
        Some(entry) => Resolved::Global {
            path: entry.path.clone(),
            cwd: entry.cwd.clone(),
        },
        None => Resolved::NotFound,
    }
}

/// The session `arg` names: the one whose id is `arg`, else the most recent whose id starts with it,
/// which is the order the listings come in.
fn find<'a>(
    mut list: impl Iterator<Item = &'a SessionEntry> + Clone,
    arg: &str,
) -> Option<&'a SessionEntry> {
    list.clone()
        .find(|entry| entry.id == arg)
        .or_else(|| list.find(|entry| entry.id.starts_with(arg)))
}
