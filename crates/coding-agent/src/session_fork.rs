//! `--fork <path|id>` (`cli/args.ts`, `main.ts`, `core/session-manager.ts` `forkFrom`): a copy of
//! another session, under a new id, in this project's session directory. What the argument names
//! is `session_resolve`'s answer. `/fork` and `/clone` write a new session from the active path
//! instead (`createBranchedSession`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use pi_durable::cancel::CancelToken;
use pi_durable::ids::EntryId;
use pi_durable::session::kernel::{Session, open_session};
use pi_durable::session::transaction::TransactionScope;
use pi_durable::session::types::Tx;
use pi_durable::storage::memory::MemoryStorage;
use pi_durable::storage::sqlite;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::{ContextEdit, EntryDraft, EntryRecord, Storage};

/// Copy `source` into `dir` under a new id and return the new file. A session whose run is still in
/// flight is refused: its copy would resume that run.
pub fn copy(source: &str, dir: &str, id: Option<&str>) -> Result<String, String> {
    let path = Path::new(source);
    if !path.is_file() {
        return Err(format!("Cannot fork: source session is missing: {source}"));
    }
    if sqlite::in_flight(path).map_err(|e| format!("Cannot fork: {e}"))? {
        return Err(format!(
            "Cannot fork: {source} has a run in flight, and its copy would resume it"
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Error: {e}"))?;
    let target = match id {
        Some(id) => format!("{dir}/{id}.db"),
        None => new_path(dir),
    };
    if Path::new(&target).exists() {
        return Err(format!(
            "Session already exists with id '{}'",
            id.unwrap_or_default()
        ));
    }
    sqlite::copy_database(path, Path::new(&target)).map_err(|e| format!("Cannot fork: {e}"))?;
    Ok(target)
}

/// The session `/fork` and `/clone` write: a new file holding `path`'s entries, upstream's
/// `createBranchedSession`. Upstream rewrites the session file from that path alone; durable
/// cannot drop entries from a file, so the path is replayed into a fresh session, which lands in
/// the same place: no abandoned branches, and the leaf is the last entry of `path`. An empty path
/// is a fork before the first entry, a session with only its header.
pub fn branch(dir: &str, path: &[EntryRecord]) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Error: {e}"))?;
    let target = new_path(dir);
    if let Err(e) = replay(&target, path) {
        let _ = std::fs::remove_file(&target);
        return Err(format!("Cannot fork: {e}"));
    }
    Ok(target)
}

/// Where `/fork`, `/clone` and RPC's `fork` write their copy: a new session file holding the
/// active path up to `point`, beside the session it came from. With nothing to attach after, it
/// is a new empty session.
pub fn branch_from(
    source: Option<&str>,
    tree: &crate::session_tree::Tree,
    point: Option<EntryId>,
) -> Result<String, String> {
    let source = source.ok_or_else(|| {
        "This session has not been saved yet. Send a message before cloning or forking it."
            .to_owned()
    })?;
    let dir = Path::new(source)
        .parent()
        .and_then(Path::to_str)
        .map(str::to_owned)
        .unwrap_or_else(|| ".".to_owned());
    branch(&dir, &active_path(tree, point))
}

pub fn branch_in_memory(
    tree: &crate::session_tree::Tree,
    point: Option<EntryId>,
) -> Result<Arc<dyn Storage>, String> {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let session = open_session(Arc::clone(&storage), None, None);
    write(&session, &active_path(tree, point), &CancelToken::new())
        .map_err(|e| format!("Cannot fork: {e}"))?;
    Ok(storage)
}

fn active_path(tree: &crate::session_tree::Tree, point: Option<EntryId>) -> Vec<EntryRecord> {
    point
        .map(|at| tree.path_to(at))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| tree.entry(id).cloned())
        .collect()
}

fn replay(path_out: &str, path: &[EntryRecord]) -> Result<(), String> {
    let storage = SqliteStorage::open(path_out, &Default::default()).map_err(|e| e.to_string())?;
    let session = open_session(Arc::new(storage), None, None);
    let cancel = CancelToken::new();
    let written = write(&session, path, &cancel);
    let closed = session.close(&cancel);
    written?;
    closed.map_err(|e| e.to_string())
}

fn write(session: &Session, path: &[EntryRecord], cancel: &CancelToken) -> Result<(), String> {
    session
        .commit_with(
            |tx| {
                let conversation = tx.create_root_conversation()?;
                pi_durable::harness::harness::seed_conversation(tx, &conversation)?;
                let mut moved: BTreeMap<EntryId, EntryId> = BTreeMap::new();
                for entry in path {
                    let edits = entry
                        .edits
                        .as_ref()
                        .map(|edits| {
                            edits
                                .iter()
                                .filter_map(|edit| {
                                    moved.get(&edit.target).map(|target| ContextEdit {
                                        target: *target,
                                        action: edit.action.clone(),
                                    })
                                })
                                .collect::<Vec<ContextEdit>>()
                        })
                        .filter(|edits| !edits.is_empty());
                    let added = tx.append_entry(
                        conversation.id,
                        EntryDraft {
                            kind: entry.kind.clone(),
                            model: entry.model.clone(),
                            data: entry.data.clone(),
                            head: None,
                            edits,
                        },
                    )?;
                    moved.insert(entry.id, added.id);
                }
                Ok(())
            },
            cancel,
            TransactionScope::default(),
        )
        .map_err(|e| e.to_string())
}

fn new_path(dir: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let id =
        pi_durable::ai::utils::uuid::uuidv7(Some(millis)).unwrap_or_else(|_| millis.to_string());
    format!("{dir}/{id}.db")
}
