//! pi-rs-session — sessions as append-only JSONL trees.
//!
//! Port of pi's `packages/coding-agent/src/core/session-manager.ts`
//! (locked workspace-layout row: `crates/pi-rs-session` ←
//! `core/session-manager`), plus the modules it rides on:
//!
//! - [`session_manager`] ← `core/session-manager.ts`
//! - [`messages`] ← `core/messages.ts` (custom message vocabulary +
//!   `convertToLlm`)
//! - [`paths`] ← `utils/paths.ts` (the slice the session manager uses;
//!   the display helpers land with the WS3.3 tools)
//! - [`uuid`] ← `packages/agent/src/harness/session/uuid.ts` (uuidv7)
//! - [`time`] — JS `Date` mechanism (`toISOString`, ISO parse), written
//!   once for the crate
//!
//! Entries are `serde_json::Value` objects shaped exactly like the
//! spec's JSONL lines; see `session_manager`'s module doc for the
//! recorded divergences (sorted JSON keys, explicit config-dir
//! parameters, synchronous listing).

pub mod messages;
pub mod paths;
pub mod session_manager;
pub mod time;
pub mod uuid;

pub use session_manager::{
    CURRENT_SESSION_VERSION, FileEntry, Leaf, NewSessionOptions, SessionContext, SessionError,
    SessionInfo, SessionListProgress, SessionManager, SessionModel, SessionTreeNode,
    assert_valid_session_id, build_session_context, find_most_recent_session,
    get_default_session_dir, get_default_session_dir_path, get_latest_compaction_entry,
    load_entries_from_file, migrate_session_entries, parse_session_entries,
};
