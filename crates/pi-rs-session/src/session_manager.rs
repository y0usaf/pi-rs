//! Port of `packages/coding-agent/src/core/session-manager.ts` — sessions
//! as append-only trees stored in JSONL files.
//!
//! Entries are `serde_json::Value` objects shaped exactly like the spec's
//! (camelCase keys, `undefined` fields omitted). Values, not structs, so
//! that unknown fields written by pi (or by extensions) survive the
//! load → migrate → rewrite round-trip byte-for-byte in content. The
//! typed `AgentMessage` vocabulary arrives with the WS4 agent port.
//!
//! Recorded divergences from the spec (behavioral parity holds; see the
//! WS3.1 settings-manager precedent):
//! - JSON object keys serialize in lexicographic order (serde_json map),
//!   not insertion order; files parse identically.
//! - `getAgentDir()`/`getSessionsDir()` come from the spec's `config.ts`,
//!   which lives above this crate (`pi-rs-app`); here they are explicit
//!   `agent_dir` / `sessions_dir` parameters supplied by the caller.
//! - `list`/`listAll` are synchronous (the spec bounds fs concurrency at
//!   10 in-flight reads; a mechanism detail, the observable ordering and
//!   progress contract are identical).

use std::collections::{HashMap, HashSet};
use std::io::{BufRead as _, Read as _, Write as _};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::messages::{
    create_branch_summary_message, create_compaction_summary_message, create_custom_message,
};
use crate::paths::{normalize_path, resolve_path, resolve_path_in};
use crate::time::{now_iso, parse_iso_ms, system_time_ms};
use crate::uuid::{random_uuid, uuidv7};

pub const CURRENT_SESSION_VERSION: u64 = 3;

/// Raw file entry (session header or session entry), as parsed JSON.
pub type FileEntry = Value;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Spec: `assertValidSessionId` throw.
    #[error(
        "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character"
    )]
    InvalidSessionId,
    /// Spec: `Entry ${id} not found` throws in branch/label operations.
    #[error("Entry {0} not found")]
    EntryNotFound(String),
    /// Spec: `forkFrom` throw for an empty/invalid source file.
    #[error("Cannot fork: source session file is empty or invalid: {0}")]
    ForkSourceInvalid(String),
    /// Spec: `forkFrom` throw for a source without a header.
    #[error("Cannot fork: source session has no header: {0}")]
    ForkSourceMissingHeader(String),
    #[error("session file I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("session entry encoding: {0}")]
    Json(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, SessionError>;

/// Spec: `NewSessionOptions`.
#[derive(Clone, Debug, Default)]
pub struct NewSessionOptions {
    pub id: Option<String>,
    pub parent_session: Option<String>,
}

/// Spec: `SessionContext` — what gets sent to the LLM.
#[derive(Clone, Debug)]
pub struct SessionContext {
    pub messages: Vec<Value>,
    pub thinking_level: String,
    pub model: Option<SessionModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionModel {
    pub provider: String,
    pub model_id: String,
}

/// Spec: `SessionTreeNode` — defensive copy of session structure.
#[derive(Clone, Debug)]
pub struct SessionTreeNode {
    pub entry: Value,
    pub children: Vec<SessionTreeNode>,
    /// Resolved label for this entry, if any.
    pub label: Option<String>,
    /// Timestamp of the latest label change for this entry, if any.
    pub label_timestamp: Option<String>,
}

/// Spec: `SessionInfo` — listing metadata built by streaming a file.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub path: PathBuf,
    pub id: String,
    /// Working directory where the session was started. Empty string for old sessions.
    pub cwd: String,
    /// User-defined display name from `session_info` entries.
    pub name: Option<String>,
    /// Path to the parent session (if this session was forked).
    pub parent_session_path: Option<String>,
    /// `new Date(header.timestamp)` — `None` where JS has an invalid date.
    pub created_ms: Option<i64>,
    pub modified_ms: i64,
    pub message_count: usize,
    pub first_message: String,
    pub all_messages_text: String,
}

/// Spec: `buildSessionContext`'s `leafId?: string | null` parameter.
#[derive(Clone, Copy, Debug)]
pub enum Leaf<'a> {
    /// `undefined` — fall back to the last entry.
    Latest,
    /// Explicit `null` — navigated to before the first entry.
    None,
    Id(&'a str),
}

// =============================================================================
// Entry field accessors (spec: plain property access on parsed JSON)
// =============================================================================

fn field<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

fn entry_type(entry: &Value) -> &str {
    field(entry, "type").unwrap_or("")
}

fn entry_id(entry: &Value) -> &str {
    field(entry, "id").unwrap_or("")
}

fn parent_id(entry: &Value) -> Option<&str> {
    field(entry, "parentId")
}

fn entry_timestamp(entry: &Value) -> &str {
    field(entry, "timestamp").unwrap_or("")
}

fn is_assistant_message(entry: &Value) -> bool {
    entry_type(entry) == "message" && field(&entry["message"], "role") == Some("assistant")
}

fn set(entry: &mut Value, key: &str, value: Value) {
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(key.to_owned(), value);
    }
}

// =============================================================================
// Ids and validation
// =============================================================================

fn create_session_id() -> String {
    uuidv7()
}

/// Spec: `assertValidSessionId`.
pub fn assert_valid_session_id(id: &str) -> Result<()> {
    let bytes = id.as_bytes();
    let alnum = |b: &u8| b.is_ascii_alphanumeric();
    let interior = |b: &u8| alnum(b) || matches!(b, b'.' | b'_' | b'-');
    let valid = match bytes {
        [] => false,
        [only] => alnum(only),
        [first, mid @ .., last] => alnum(first) && alnum(last) && mid.iter().all(interior),
    };
    if valid {
        Ok(())
    } else {
        Err(SessionError::InvalidSessionId)
    }
}

/// Spec: `generateId` — unique short id (8 hex chars, collision-checked,
/// full UUID fallback).
fn generate_id(has: impl Fn(&str) -> bool) -> String {
    for _ in 0..100 {
        let id: String = random_uuid().chars().take(8).collect();
        if !has(&id) {
            return id;
        }
    }
    random_uuid()
}

// =============================================================================
// Migrations
// =============================================================================

/// Spec: `migrateV1ToV2` — add id/parentId tree structure. Note the spec
/// checks candidate ids against an empty set here (its `ids` set is never
/// populated); ported as-is.
fn migrate_v1_to_v2(entries: &mut [Value]) {
    let mut prev_id: Option<String> = None;
    for i in 0..entries.len() {
        if entry_type(&entries[i]) == "session" {
            set(&mut entries[i], "version", json!(2));
            continue;
        }
        let id = generate_id(|_| false);
        set(&mut entries[i], "id", json!(id));
        set(
            &mut entries[i],
            "parentId",
            prev_id.as_deref().map_or(Value::Null, Value::from),
        );
        prev_id = Some(id);

        // Convert firstKeptEntryIndex to firstKeptEntryId for compaction
        if entry_type(&entries[i]) == "compaction" {
            let index = entries[i]
                .get("firstKeptEntryIndex")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            if let Some(index) = index {
                let target_id = entries.get(index).and_then(|target| {
                    (entry_type(target) != "session").then(|| entry_id(target).to_owned())
                });
                if let Some(target_id) = target_id.filter(|t| !t.is_empty()) {
                    set(&mut entries[i], "firstKeptEntryId", json!(target_id));
                }
                if let Some(obj) = entries[i].as_object_mut() {
                    obj.remove("firstKeptEntryIndex");
                }
            }
        }
    }
}

/// Spec: `migrateV2ToV3` — rename `hookMessage` role to `custom`.
fn migrate_v2_to_v3(entries: &mut [Value]) {
    for entry in entries.iter_mut() {
        if entry_type(entry) == "session" {
            set(entry, "version", json!(3));
            continue;
        }
        if entry_type(entry) == "message"
            && field(&entry["message"], "role") == Some("hookMessage")
            && let Some(message) = entry.get_mut("message").and_then(Value::as_object_mut)
        {
            message.insert("role".to_owned(), json!("custom"));
        }
    }
}

/// Spec: `migrateToCurrentVersion` — returns true if any migration ran.
fn migrate_to_current_version(entries: &mut [Value]) -> bool {
    let version = entries
        .iter()
        .find(|e| entry_type(e) == "session")
        .and_then(|h| h.get("version").and_then(Value::as_u64))
        .unwrap_or(1);
    if version >= CURRENT_SESSION_VERSION {
        return false;
    }
    if version < 2 {
        migrate_v1_to_v2(entries);
    }
    if version < 3 {
        migrate_v2_to_v3(entries);
    }
    true
}

/// Spec: `migrateSessionEntries` (exported for testing).
pub fn migrate_session_entries(entries: &mut [Value]) {
    migrate_to_current_version(entries);
}

// =============================================================================
// Parsing and file reads
// =============================================================================

fn parse_session_entry_line(line: &str) -> Option<Value> {
    if line.trim().is_empty() {
        return None;
    }
    // Skip malformed lines
    serde_json::from_str(line).ok()
}

/// Spec: `parseSessionEntries` (exported for compaction tests).
pub fn parse_session_entries(content: &str) -> Vec<Value> {
    content
        .trim()
        .split('\n')
        .filter_map(parse_session_entry_line)
        .collect()
}

/// Spec: `getLatestCompactionEntry`.
pub fn get_latest_compaction_entry(entries: &[Value]) -> Option<&Value> {
    entries.iter().rev().find(|e| entry_type(e) == "compaction")
}

/// Spec: `loadEntriesFromFile` — tolerant line-by-line read; returns `[]`
/// unless the first parsed entry is a session header with a string id.
pub fn load_entries_from_file(file_path: &Path) -> Vec<Value> {
    let resolved = PathBuf::from(normalize_path(&file_path.to_string_lossy()));
    let Ok(file) = std::fs::File::open(&resolved) else {
        return Vec::new();
    };
    let mut reader = std::io::BufReader::with_capacity(1024 * 1024, file);
    let mut entries = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                if let Some(entry) = parse_session_entry_line(&String::from_utf8_lossy(&line)) {
                    entries.push(entry);
                }
            }
            Err(_) => return Vec::new(),
        }
    }

    // Validate session header
    if entries.is_empty() {
        return entries;
    }
    let header = &entries[0];
    if entry_type(header) != "session" || field(header, "id").is_none() {
        return Vec::new();
    }
    entries
}

/// Spec: `readSessionHeader` — first 512 bytes, first line, or null.
fn read_session_header(file_path: &Path) -> Option<Value> {
    let mut file = std::fs::File::open(file_path).ok()?;
    let mut buffer = [0u8; 512];
    let mut read = 0;
    while read < buffer.len() {
        match file.read(&mut buffer[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&buffer[..read]);
    let first_line = text.split('\n').next()?;
    let header: Value = serde_json::from_str(first_line).ok()?;
    (entry_type(&header) == "session" && field(&header, "id").is_some()).then_some(header)
}

fn session_cwd_matches(cwd: Option<&str>, resolved_cwd: &str) -> bool {
    cwd.is_some_and(|c| !c.is_empty() && resolve_path(c) == resolved_cwd)
}

/// Spec: `findMostRecentSession` (exported for testing).
pub fn find_most_recent_session(session_dir: &Path, cwd: Option<&str>) -> Option<PathBuf> {
    let resolved_dir = PathBuf::from(normalize_path(&session_dir.to_string_lossy()));
    let resolved_cwd = cwd.map(resolve_path);
    let entries = std::fs::read_dir(&resolved_dir).ok()?;
    let mut files: Vec<(PathBuf, i64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().ends_with(".jsonl") {
            continue;
        }
        let Some(header) = read_session_header(&path) else {
            continue;
        };
        if let Some(resolved_cwd) = &resolved_cwd
            && !session_cwd_matches(field(&header, "cwd"), resolved_cwd)
        {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            return None; // spec: statSync throw aborts the listing
        };
        let mtime = meta.modified().map(system_time_ms).unwrap_or(0);
        files.push((path, mtime));
    }
    files.sort_by_key(|file| std::cmp::Reverse(file.1));
    files.into_iter().next().map(|(path, _)| path)
}

// =============================================================================
// buildSessionContext
// =============================================================================

/// Spec: `buildSessionContext` — resolve the message list for the LLM by
/// walking from the leaf to the root, handling compaction and branch
/// summaries along the path. (The spec's optional `byId` parameter is a
/// reuse-the-index optimization; the index is rebuilt here.)
pub fn build_session_context(entries: &[Value], leaf_id: Leaf) -> SessionContext {
    let empty = || SessionContext {
        messages: Vec::new(),
        thinking_level: "off".to_owned(),
        model: None,
    };

    let mut by_id: HashMap<&str, &Value> = HashMap::new();
    for entry in entries {
        by_id.insert(entry_id(entry), entry);
    }

    // Find leaf
    let leaf = match leaf_id {
        // Explicitly null - return no messages (navigated to before first entry)
        Leaf::None => return empty(),
        Leaf::Id(id) => by_id.get(id).copied(),
        Leaf::Latest => None,
    };
    // Fallback to last entry (when leafId is undefined or not found)
    let Some(leaf) = leaf.or_else(|| entries.last()) else {
        return empty();
    };

    // Walk from leaf to root, collecting path
    let mut path: Vec<&Value> = Vec::new();
    let mut current = Some(leaf);
    while let Some(entry) = current {
        path.insert(0, entry);
        current = parent_id(entry).and_then(|p| by_id.get(p).copied());
    }

    // Extract settings and find compaction
    let mut thinking_level = "off".to_owned();
    let mut model: Option<SessionModel> = None;
    let mut compaction: Option<&Value> = None;
    for entry in &path {
        match entry_type(entry) {
            "thinking_level_change" => {
                if let Some(level) = field(entry, "thinkingLevel") {
                    thinking_level = level.to_owned();
                }
            }
            "model_change" => {
                model = Some(SessionModel {
                    provider: field(entry, "provider").unwrap_or("").to_owned(),
                    model_id: field(entry, "modelId").unwrap_or("").to_owned(),
                });
            }
            "message" if field(&entry["message"], "role") == Some("assistant") => {
                model = Some(SessionModel {
                    provider: field(&entry["message"], "provider")
                        .unwrap_or("")
                        .to_owned(),
                    model_id: field(&entry["message"], "model").unwrap_or("").to_owned(),
                });
            }
            "compaction" => compaction = Some(entry),
            _ => {}
        }
    }

    let mut messages: Vec<Value> = Vec::new();
    let append_message = |entry: &Value, messages: &mut Vec<Value>| match entry_type(entry) {
        "message" => messages.push(entry["message"].clone()),
        "custom_message" => messages.push(create_custom_message(
            field(entry, "customType").unwrap_or(""),
            entry.get("content").unwrap_or(&Value::Null),
            entry
                .get("display")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            entry.get("details"),
            entry_timestamp(entry),
        )),
        "branch_summary" => {
            if let Some(summary) = field(entry, "summary").filter(|s| !s.is_empty()) {
                messages.push(create_branch_summary_message(
                    summary,
                    field(entry, "fromId").unwrap_or(""),
                    entry_timestamp(entry),
                ));
            }
        }
        _ => {}
    };

    if let Some(compaction) = compaction {
        // Emit summary first
        messages.push(create_compaction_summary_message(
            field(compaction, "summary").unwrap_or(""),
            compaction
                .get("tokensBefore")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            entry_timestamp(compaction),
        ));

        // Find compaction index in path
        let compaction_idx = path
            .iter()
            .position(|e| entry_type(e) == "compaction" && entry_id(e) == entry_id(compaction))
            .unwrap_or(path.len());

        // Emit kept messages (before compaction, starting from firstKeptEntryId)
        let first_kept = field(compaction, "firstKeptEntryId").unwrap_or("");
        let mut found_first_kept = false;
        for entry in path.iter().take(compaction_idx) {
            if entry_id(entry) == first_kept {
                found_first_kept = true;
            }
            if found_first_kept {
                append_message(entry, &mut messages);
            }
        }

        // Emit messages after compaction
        for entry in path.iter().skip(compaction_idx + 1) {
            append_message(entry, &mut messages);
        }
    } else {
        // No compaction - emit all messages, handle branch summaries and custom messages
        for entry in &path {
            append_message(entry, &mut messages);
        }
    }

    SessionContext {
        messages,
        thinking_level,
        model,
    }
}

// =============================================================================
// Default directories (spec: config.ts seam, here explicit parameters)
// =============================================================================

/// Spec: `getDefaultSessionDirPath` — encode cwd into a safe directory
/// name under `{agentDir}/sessions/`.
pub fn get_default_session_dir_path(cwd: &str, agent_dir: &str) -> PathBuf {
    let resolved_cwd = resolve_path(cwd);
    let resolved_agent_dir = resolve_path(agent_dir);
    let safe_path = format!(
        "--{}--",
        resolved_cwd
            .trim_start_matches(['/', '\\'])
            .replace(['/', '\\', ':'], "-")
    );
    PathBuf::from(resolved_agent_dir)
        .join("sessions")
        .join(safe_path)
}

/// Spec: `getDefaultSessionDir` — the path above, created if missing.
pub fn get_default_session_dir(cwd: &str, agent_dir: &str) -> Result<PathBuf> {
    let session_dir = get_default_session_dir_path(cwd, agent_dir);
    if !session_dir.exists() {
        std::fs::create_dir_all(&session_dir)?;
    }
    Ok(session_dir)
}

// =============================================================================
// Session listing (spec: buildSessionInfo / listSessionsFromDir)
// =============================================================================

/// Spec: `SessionListProgress` — `(loaded, total)` callback.
pub type SessionListProgress<'a> = &'a mut dyn FnMut(usize, usize);

fn is_message_with_content(message: &Value) -> bool {
    field(message, "role").is_some() && message.get("content").is_some()
}

fn extract_text_content(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| field(b, "type") == Some("text"))
            .filter_map(|b| field(b, "text"))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn get_message_activity_time(entry: &Value) -> Option<i64> {
    let message = &entry["message"];
    if !is_message_with_content(message) {
        return None;
    }
    if !matches!(field(message, "role"), Some("user" | "assistant")) {
        return None;
    }
    if let Some(ms) = message.get("timestamp").and_then(Value::as_i64) {
        return Some(ms);
    }
    parse_iso_ms(entry_timestamp(entry))
}

/// Spec: `buildSessionInfo` — stream a session file into listing metadata.
fn build_session_info(file_path: &Path) -> Option<SessionInfo> {
    let stats = std::fs::metadata(file_path).ok()?;
    let file = std::fs::File::open(file_path).ok()?;
    let mut reader = std::io::BufReader::new(file);

    let mut header: Option<Value> = None;
    let mut message_count = 0usize;
    let mut first_message = String::new();
    let mut all_messages: Vec<String> = Vec::new();
    let mut name: Option<String> = None;
    let mut last_activity_time: Option<i64> = None;

    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
        let Some(entry) = parse_session_entry_line(&String::from_utf8_lossy(&line)) else {
            continue;
        };
        if header.is_none() {
            if entry_type(&entry) != "session" {
                return None;
            }
            header = Some(entry);
            continue;
        }

        // Extract session name (use latest, including explicit clears)
        if entry_type(&entry) == "session_info" {
            name = field(&entry, "name")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned);
        }

        if entry_type(&entry) != "message" {
            continue;
        }
        message_count += 1;

        if let Some(activity) = get_message_activity_time(&entry) {
            last_activity_time = Some(last_activity_time.unwrap_or(0).max(activity));
        }

        let message = &entry["message"];
        if !is_message_with_content(message) {
            continue;
        }
        if !matches!(field(message, "role"), Some("user" | "assistant")) {
            continue;
        }
        let text = extract_text_content(message);
        if text.is_empty() {
            continue;
        }
        if first_message.is_empty() && field(message, "role") == Some("user") {
            first_message = text.clone();
        }
        all_messages.push(text);
    }

    let header = header?;
    let cwd = field(&header, "cwd").unwrap_or("").to_owned();
    let parent_session_path = field(&header, "parentSession").map(str::to_owned);
    let header_time = field(&header, "timestamp").and_then(parse_iso_ms);
    let modified_ms = match last_activity_time {
        Some(t) if t > 0 => t,
        _ => header_time.unwrap_or_else(|| stats.modified().map(system_time_ms).unwrap_or(0)),
    };

    Some(SessionInfo {
        path: file_path.to_path_buf(),
        id: field(&header, "id").unwrap_or("").to_owned(),
        cwd,
        name,
        parent_session_path,
        created_ms: field(&header, "timestamp").and_then(parse_iso_ms),
        modified_ms,
        message_count,
        first_message: if first_message.is_empty() {
            "(no messages)".to_owned()
        } else {
            first_message
        },
        all_messages_text: all_messages.join(" "),
    })
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".jsonl"))
        .collect();
    files.sort();
    files
}

/// Spec: `listSessionsFromDir`.
fn list_sessions_from_dir(
    dir: &Path,
    mut on_progress: Option<SessionListProgress<'_>>,
    progress_offset: usize,
    progress_total: Option<usize>,
) -> Vec<SessionInfo> {
    if !dir.exists() {
        return Vec::new();
    }
    let files = jsonl_files(dir);
    let total = progress_total.unwrap_or(files.len());
    let mut sessions = Vec::new();
    for (loaded, file) in files.iter().enumerate() {
        if let Some(info) = build_session_info(file) {
            sessions.push(info);
        }
        if let Some(cb) = on_progress.as_deref_mut() {
            cb(progress_offset + loaded + 1, total);
        }
    }
    sessions
}

// =============================================================================
// SessionManager
// =============================================================================

/// Manages conversation sessions as append-only trees stored in JSONL
/// files (spec: `SessionManager`). Every entry has an id and parentId
/// forming a tree; the leaf pointer tracks the current position.
pub struct SessionManager {
    session_id: String,
    session_file: Option<String>,
    session_dir: String,
    cwd: String,
    /// Config seam: the spec reaches into `config.ts` for the default
    /// agent dir; here the constructor receives it.
    agent_dir: String,
    persist: bool,
    flushed: bool,
    file_entries: Vec<Value>,
    by_id: HashMap<String, usize>,
    /// Insertion-ordered, matching the spec's `Map` iteration order
    /// (label re-writes keep their original position).
    labels_by_id: IndexMap<String, String>,
    label_timestamps_by_id: HashMap<String, String>,
    leaf_id: Option<String>,
}

impl SessionManager {
    fn new(
        cwd: &str,
        session_dir: &str,
        session_file: Option<&str>,
        persist: bool,
        agent_dir: &str,
        new_session_options: Option<NewSessionOptions>,
    ) -> Result<Self> {
        let mut manager = Self {
            session_id: String::new(),
            session_file: None,
            session_dir: normalize_path(session_dir),
            cwd: resolve_path(cwd),
            agent_dir: agent_dir.to_owned(),
            persist,
            flushed: false,
            file_entries: Vec::new(),
            by_id: HashMap::new(),
            labels_by_id: IndexMap::new(),
            label_timestamps_by_id: HashMap::new(),
            leaf_id: None,
        };
        if persist && !manager.session_dir.is_empty() && !Path::new(&manager.session_dir).exists() {
            std::fs::create_dir_all(&manager.session_dir)?;
        }
        if let Some(file) = session_file {
            manager.set_session_file(file)?;
        } else {
            manager.new_session(new_session_options)?;
        }
        Ok(manager)
    }

    /// Spec: `setSessionFile` — switch to a different session file (used
    /// for resume and branching).
    pub fn set_session_file(&mut self, session_file: &str) -> Result<()> {
        let resolved = resolve_path(session_file);
        self.session_file = Some(resolved.clone());
        if Path::new(&resolved).exists() {
            self.file_entries = load_entries_from_file(Path::new(&resolved));

            // If file was empty or corrupted (no valid header), truncate and
            // start fresh to avoid appending messages without a header.
            if self.file_entries.is_empty() {
                self.new_session(None)?;
                self.session_file = Some(resolved);
                self.rewrite_file()?;
                self.flushed = true;
                return Ok(());
            }

            self.session_id = self
                .file_entries
                .iter()
                .find(|e| entry_type(e) == "session")
                .and_then(|h| field(h, "id"))
                .map_or_else(create_session_id, str::to_owned);

            if migrate_to_current_version(&mut self.file_entries) {
                self.rewrite_file()?;
            }

            self.build_index();
            self.flushed = true;
        } else {
            self.new_session(None)?;
            self.session_file = Some(resolved); // preserve explicit path from --session flag
        }
        Ok(())
    }

    /// Spec: `newSession` — returns the new session file path when persisting.
    pub fn new_session(&mut self, options: Option<NewSessionOptions>) -> Result<Option<String>> {
        let options = options.unwrap_or_default();
        if let Some(id) = &options.id {
            assert_valid_session_id(id)?;
        }
        self.session_id = options.id.unwrap_or_else(create_session_id);
        let timestamp = now_iso();
        let mut header = Map::new();
        header.insert("type".into(), "session".into());
        header.insert("version".into(), CURRENT_SESSION_VERSION.into());
        header.insert("id".into(), self.session_id.clone().into());
        header.insert("timestamp".into(), timestamp.clone().into());
        header.insert("cwd".into(), self.cwd.clone().into());
        if let Some(parent) = options.parent_session {
            header.insert("parentSession".into(), parent.into());
        }
        self.file_entries = vec![Value::Object(header)];
        self.by_id.clear();
        self.labels_by_id.clear();
        self.leaf_id = None;
        self.flushed = false;

        if self.persist {
            let file_timestamp = timestamp.replace([':', '.'], "-");
            self.session_file = Some(
                Path::new(&self.session_dir)
                    .join(format!("{file_timestamp}_{}.jsonl", self.session_id))
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        Ok(self.session_file.clone())
    }

    fn build_index(&mut self) {
        self.by_id.clear();
        self.labels_by_id.clear();
        self.label_timestamps_by_id.clear();
        self.leaf_id = None;
        for (index, entry) in self.file_entries.iter().enumerate() {
            if entry_type(entry) == "session" {
                continue;
            }
            self.by_id.insert(entry_id(entry).to_owned(), index);
            self.leaf_id = Some(entry_id(entry).to_owned());
            if entry_type(entry) == "label" {
                let target = field(entry, "targetId").unwrap_or("").to_owned();
                match field(entry, "label").filter(|l| !l.is_empty()) {
                    Some(label) => {
                        self.labels_by_id.insert(target.clone(), label.to_owned());
                        self.label_timestamps_by_id
                            .insert(target, entry_timestamp(entry).to_owned());
                    }
                    None => {
                        self.labels_by_id.shift_remove(&target);
                        self.label_timestamps_by_id.remove(&target);
                    }
                }
            }
        }
    }

    fn rewrite_file(&self) -> Result<()> {
        let Some(file) = self.persist.then_some(self.session_file.as_ref()).flatten() else {
            return Ok(());
        };
        let mut fd = std::fs::File::create(file)?;
        for entry in &self.file_entries {
            writeln!(fd, "{}", serde_json::to_string(entry)?)?;
        }
        Ok(())
    }

    pub fn is_persisted(&self) -> bool {
        self.persist
    }

    pub fn get_cwd(&self) -> &str {
        &self.cwd
    }

    pub fn get_session_dir(&self) -> &str {
        &self.session_dir
    }

    /// Spec: `usesDefaultSessionDir`.
    pub fn uses_default_session_dir(&self) -> bool {
        Path::new(&self.session_dir) == get_default_session_dir_path(&self.cwd, &self.agent_dir)
    }

    pub fn get_session_id(&self) -> &str {
        &self.session_id
    }

    pub fn get_session_file(&self) -> Option<&str> {
        self.session_file.as_deref()
    }

    /// Spec: `_persist` — defer file creation until the first assistant
    /// message; flush everything on that boundary, append afterwards.
    fn persist_entry(&mut self, entry: &Value) -> Result<()> {
        let Some(file) = self.persist.then_some(self.session_file.clone()).flatten() else {
            return Ok(());
        };

        let has_assistant = self.file_entries.iter().any(is_assistant_message);
        if !has_assistant {
            if self.flushed {
                let mut fd = std::fs::OpenOptions::new().append(true).open(&file)?;
                writeln!(fd, "{}", serde_json::to_string(entry)?)?;
            } else {
                // Mark as not flushed so when assistant arrives, all entries get written
                self.flushed = false;
            }
            return Ok(());
        }

        if self.flushed {
            let mut fd = std::fs::OpenOptions::new().append(true).open(&file)?;
            writeln!(fd, "{}", serde_json::to_string(entry)?)?;
        } else {
            let mut fd = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?;
            for e in &self.file_entries {
                writeln!(fd, "{}", serde_json::to_string(e)?)?;
            }
            self.flushed = true;
        }
        Ok(())
    }

    fn append_entry(&mut self, entry: Value) -> Result<String> {
        let id = entry_id(&entry).to_owned();
        self.file_entries.push(entry.clone());
        self.by_id.insert(id.clone(), self.file_entries.len() - 1);
        self.leaf_id = Some(id.clone());
        self.persist_entry(&entry)?;
        Ok(id)
    }

    /// Commit entries held by a short-lived binding consumer.
    pub fn flush(&mut self) -> Result<()> {
        let Some(file) = self.persist.then_some(self.session_file.clone()).flatten() else {
            return Ok(());
        };
        if self.flushed {
            return Ok(());
        }
        let mut fd = std::fs::File::create(file)?;
        for entry in &self.file_entries {
            writeln!(fd, "{}", serde_json::to_string(entry)?)?;
        }
        self.flushed = true;
        Ok(())
    }

    fn next_id(&self) -> String {
        generate_id(|id| self.by_id.contains_key(id))
    }

    fn base_fields(&self, entry: &mut Map<String, Value>, entry_type: &str) {
        entry.insert("type".into(), entry_type.into());
        entry.insert("id".into(), self.next_id().into());
        entry.insert(
            "parentId".into(),
            self.leaf_id.as_deref().map_or(Value::Null, Value::from),
        );
        entry.insert("timestamp".into(), now_iso().into());
    }

    /// Spec: `appendMessage` — append as child of the current leaf, then
    /// advance the leaf. Returns the entry id. Compaction and branch
    /// summaries must go through their dedicated append methods.
    pub fn append_message(&mut self, message: Value) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "message");
        entry.insert("message".into(), message);
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `appendThinkingLevelChange`.
    pub fn append_thinking_level_change(&mut self, thinking_level: &str) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "thinking_level_change");
        entry.insert("thinkingLevel".into(), thinking_level.into());
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `appendModelChange`.
    pub fn append_model_change(&mut self, provider: &str, model_id: &str) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "model_change");
        entry.insert("provider".into(), provider.into());
        entry.insert("modelId".into(), model_id.into());
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `appendCompaction`.
    pub fn append_compaction(
        &mut self,
        summary: &str,
        first_kept_entry_id: &str,
        tokens_before: i64,
        details: Option<Value>,
        from_hook: Option<bool>,
    ) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "compaction");
        entry.insert("summary".into(), summary.into());
        entry.insert("firstKeptEntryId".into(), first_kept_entry_id.into());
        entry.insert("tokensBefore".into(), tokens_before.into());
        if let Some(details) = details {
            entry.insert("details".into(), details);
        }
        if let Some(from_hook) = from_hook {
            entry.insert("fromHook".into(), from_hook.into());
        }
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `appendCustomEntry` (extension data; not in LLM context).
    pub fn append_custom_entry(
        &mut self,
        custom_type: &str,
        data: Option<Value>,
    ) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "custom");
        entry.insert("customType".into(), custom_type.into());
        if let Some(data) = data {
            entry.insert("data".into(), data);
        }
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `appendSessionInfo` (e.g. display name).
    pub fn append_session_info(&mut self, name: &str) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "session_info");
        entry.insert("name".into(), name.trim().into());
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `getSessionName` — latest `session_info` wins; empty names
    /// explicitly clear the title.
    pub fn get_session_name(&self) -> Option<String> {
        for entry in self.file_entries.iter().rev() {
            if entry_type(entry) == "session_info" {
                return field(entry, "name")
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned);
            }
        }
        None
    }

    /// Spec: `appendCustomMessageEntry` (extension message; participates
    /// in LLM context via `buildSessionContext`).
    pub fn append_custom_message_entry(
        &mut self,
        custom_type: &str,
        content: Value,
        display: bool,
        details: Option<Value>,
    ) -> Result<String> {
        let mut entry = Map::new();
        self.base_fields(&mut entry, "custom_message");
        entry.insert("customType".into(), custom_type.into());
        entry.insert("content".into(), content);
        entry.insert("display".into(), display.into());
        if let Some(details) = details {
            entry.insert("details".into(), details);
        }
        self.append_entry(Value::Object(entry))
    }

    // =========================================================================
    // Tree Traversal
    // =========================================================================

    pub fn get_leaf_id(&self) -> Option<&str> {
        self.leaf_id.as_deref()
    }

    pub fn get_leaf_entry(&self) -> Option<&Value> {
        self.leaf_id.as_deref().and_then(|id| self.get_entry(id))
    }

    pub fn get_entry(&self, id: &str) -> Option<&Value> {
        self.by_id.get(id).map(|&index| &self.file_entries[index])
    }

    /// Spec: `getChildren` — all direct children of an entry.
    pub fn get_children(&self, parent: &str) -> Vec<Value> {
        self.file_entries
            .iter()
            .filter(|e| entry_type(e) != "session" && parent_id(e) == Some(parent))
            .cloned()
            .collect()
    }

    /// Spec: `getLabel`.
    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.labels_by_id.get(id).map(String::as_str)
    }

    /// Spec: `appendLabelChange` — set or clear (None / empty) a label.
    pub fn append_label_change(&mut self, target_id: &str, label: Option<&str>) -> Result<String> {
        if !self.by_id.contains_key(target_id) {
            return Err(SessionError::EntryNotFound(target_id.to_owned()));
        }
        let mut entry = Map::new();
        self.base_fields(&mut entry, "label");
        entry.insert("targetId".into(), target_id.into());
        if let Some(label) = label {
            entry.insert("label".into(), label.into());
        }
        let timestamp = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let id = self.append_entry(Value::Object(entry))?;
        match label.filter(|l| !l.is_empty()) {
            Some(label) => {
                self.labels_by_id
                    .insert(target_id.to_owned(), label.to_owned());
                self.label_timestamps_by_id
                    .insert(target_id.to_owned(), timestamp);
            }
            None => {
                self.labels_by_id.shift_remove(target_id);
                self.label_timestamps_by_id.remove(target_id);
            }
        }
        Ok(id)
    }

    /// Spec: `getBranch` — walk from entry (default: leaf) to root, in
    /// path order, all entry types included.
    pub fn get_branch(&self, from_id: Option<&str>) -> Vec<Value> {
        let mut path = Vec::new();
        let start = from_id.map(str::to_owned).or_else(|| self.leaf_id.clone());
        let mut current = start.as_deref().and_then(|id| self.get_entry(id));
        while let Some(entry) = current {
            path.insert(0, entry.clone());
            current = parent_id(entry).and_then(|p| self.get_entry(p));
        }
        path
    }

    /// Spec: `buildSessionContext` (instance) — tree traversal from the
    /// current leaf.
    pub fn build_session_context(&self) -> SessionContext {
        let entries = self.get_entries();
        let leaf = match &self.leaf_id {
            Some(id) => Leaf::Id(id),
            None => Leaf::None,
        };
        build_session_context(&entries, leaf)
    }

    /// Spec: `getHeader`.
    pub fn get_header(&self) -> Option<&Value> {
        self.file_entries
            .iter()
            .find(|e| entry_type(e) == "session")
    }

    /// Spec: `getEntries` — all session entries (excludes header).
    pub fn get_entries(&self) -> Vec<Value> {
        self.file_entries
            .iter()
            .filter(|e| entry_type(e) != "session")
            .cloned()
            .collect()
    }

    /// Spec: `getTree` — the session as a tree; orphaned entries are
    /// returned as roots; children sorted by timestamp (oldest first).
    pub fn get_tree(&self) -> Vec<SessionTreeNode> {
        let entries = self.get_entries();
        let mut index_of: HashMap<&str, usize> = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            index_of.insert(entry_id(entry), i);
        }

        let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
        let mut root_indices: Vec<usize> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            match parent_id(entry) {
                None => root_indices.push(i),
                Some(parent) if parent == entry_id(entry) => root_indices.push(i),
                Some(parent) => match index_of.get(parent) {
                    Some(&p) => children_of[p].push(i),
                    // Orphan - treat as root
                    None => root_indices.push(i),
                },
            }
        }

        // Parent-before-child order via explicit stack (spec avoids
        // recursion on deep trees), then assemble bottom-up.
        let mut topo: Vec<usize> = Vec::with_capacity(entries.len());
        let mut stack: Vec<usize> = root_indices.clone();
        while let Some(i) = stack.pop() {
            topo.push(i);
            stack.extend(children_of[i].iter().copied());
        }

        let mut nodes: Vec<Option<SessionTreeNode>> = entries
            .iter()
            .map(|entry| {
                let id = entry_id(entry);
                Some(SessionTreeNode {
                    entry: entry.clone(),
                    children: Vec::new(),
                    label: self.labels_by_id.get(id).cloned(),
                    label_timestamp: self.label_timestamps_by_id.get(id).cloned(),
                })
            })
            .collect();

        for &i in topo.iter().rev() {
            let mut children: Vec<SessionTreeNode> = children_of[i]
                .iter()
                .filter_map(|&c| nodes[c].take())
                .collect();
            // Sort children by timestamp (oldest first, newest at bottom)
            children.sort_by(|a, b| {
                let ta = parse_iso_ms(entry_timestamp(&a.entry));
                let tb = parse_iso_ms(entry_timestamp(&b.entry));
                match (ta, tb) {
                    (Some(ta), Some(tb)) => ta.cmp(&tb),
                    _ => std::cmp::Ordering::Equal,
                }
            });
            if let Some(node) = nodes[i].as_mut() {
                node.children = children;
            }
        }

        root_indices
            .into_iter()
            .filter_map(|i| nodes[i].take())
            .collect()
    }

    // =========================================================================
    // Branching
    // =========================================================================

    /// Spec: `branch` — move the leaf pointer to an earlier entry.
    pub fn branch(&mut self, branch_from_id: &str) -> Result<()> {
        if !self.by_id.contains_key(branch_from_id) {
            return Err(SessionError::EntryNotFound(branch_from_id.to_owned()));
        }
        self.leaf_id = Some(branch_from_id.to_owned());
        Ok(())
    }

    /// Spec: `resetLeaf` — next append creates a new root entry.
    pub fn reset_leaf(&mut self) {
        self.leaf_id = None;
    }

    /// Spec: `branchWithSummary` — branch plus a `branch_summary` entry
    /// capturing the abandoned path.
    pub fn branch_with_summary(
        &mut self,
        branch_from_id: Option<&str>,
        summary: &str,
        details: Option<Value>,
        from_hook: Option<bool>,
    ) -> Result<String> {
        if let Some(id) = branch_from_id
            && !self.by_id.contains_key(id)
        {
            return Err(SessionError::EntryNotFound(id.to_owned()));
        }
        self.leaf_id = branch_from_id.map(str::to_owned);
        let mut entry = Map::new();
        self.base_fields(&mut entry, "branch_summary");
        entry.insert("fromId".into(), branch_from_id.unwrap_or("root").into());
        entry.insert("summary".into(), summary.into());
        if let Some(details) = details {
            entry.insert("details".into(), details);
        }
        if let Some(from_hook) = from_hook {
            entry.insert("fromHook".into(), from_hook.into());
        }
        self.append_entry(Value::Object(entry))
    }

    /// Spec: `createBranchedSession` — new session containing only the
    /// path from root to `leaf_id` (plus labels on that path). Returns
    /// the new file path, or `None` when not persisting.
    pub fn create_branched_session(&mut self, leaf_id: &str) -> Result<Option<String>> {
        let previous_session_file = self.session_file.clone();
        let path = self.get_branch(Some(leaf_id));
        if path.is_empty() {
            return Err(SessionError::EntryNotFound(leaf_id.to_owned()));
        }

        // Filter out label entries - recreated from the resolved map below
        let path_without_labels: Vec<Value> = path
            .into_iter()
            .filter(|e| entry_type(e) != "label")
            .collect();

        let new_session_id = create_session_id();
        let timestamp = now_iso();
        let file_timestamp = timestamp.replace([':', '.'], "-");
        let new_session_file = Path::new(&self.session_dir)
            .join(format!("{file_timestamp}_{new_session_id}.jsonl"))
            .to_string_lossy()
            .into_owned();

        let mut header = Map::new();
        header.insert("type".into(), "session".into());
        header.insert("version".into(), CURRENT_SESSION_VERSION.into());
        header.insert("id".into(), new_session_id.clone().into());
        header.insert("timestamp".into(), timestamp.into());
        header.insert("cwd".into(), self.cwd.clone().into());
        if self.persist
            && let Some(previous) = &previous_session_file
        {
            header.insert("parentSession".into(), previous.clone().into());
        }

        // Collect labels for entries in the path
        let mut path_entry_ids: HashSet<String> = path_without_labels
            .iter()
            .map(|e| entry_id(e).to_owned())
            .collect();
        let labels_to_write: Vec<(String, String, Option<String>)> = self
            .labels_by_id
            .iter()
            .filter(|(target, _)| path_entry_ids.contains(*target))
            .map(|(target, label)| {
                (
                    target.clone(),
                    label.clone(),
                    self.label_timestamps_by_id.get(target).cloned(),
                )
            })
            .collect();

        // Build label entries chained after the last path entry
        let mut parent: Option<String> = path_without_labels.last().map(|e| entry_id(e).to_owned());
        let mut label_entries: Vec<Value> = Vec::new();
        for (target_id, label, label_timestamp) in labels_to_write {
            let id = generate_id(|candidate| path_entry_ids.contains(candidate));
            path_entry_ids.insert(id.clone());
            let mut entry = Map::new();
            entry.insert("type".into(), "label".into());
            entry.insert("id".into(), id.clone().into());
            entry.insert(
                "parentId".into(),
                parent.as_deref().map_or(Value::Null, Value::from),
            );
            if let Some(ts) = label_timestamp {
                entry.insert("timestamp".into(), ts.into());
            }
            entry.insert("targetId".into(), target_id.into());
            entry.insert("label".into(), label.into());
            label_entries.push(Value::Object(entry));
            parent = Some(id);
        }

        self.file_entries = std::iter::once(Value::Object(header))
            .chain(path_without_labels)
            .chain(label_entries)
            .collect();
        self.session_id = new_session_id;
        if self.persist {
            self.session_file = Some(new_session_file.clone());
        }
        self.build_index();

        if self.persist {
            // Only write the file now if it contains an assistant message;
            // otherwise defer to persist_entry, matching newSession()'s
            // contract and avoiding the duplicate-header bug.
            let has_assistant = self.file_entries.iter().any(is_assistant_message);
            if has_assistant {
                self.rewrite_file()?;
                self.flushed = true;
            } else {
                self.flushed = false;
            }
            return Ok(Some(new_session_file));
        }
        Ok(None)
    }

    // =========================================================================
    // Statics
    // =========================================================================

    /// Spec: `SessionManager.create` — new persisted session; the default
    /// session dir is `{agent_dir}/sessions/<encoded-cwd>/`.
    pub fn create(
        cwd: &str,
        session_dir: Option<&str>,
        agent_dir: &str,
        options: Option<NewSessionOptions>,
    ) -> Result<Self> {
        let dir = match session_dir {
            Some(dir) => normalize_path(dir),
            None => get_default_session_dir(cwd, agent_dir)?
                .to_string_lossy()
                .into_owned(),
        };
        Self::new(cwd, &dir, None, true, agent_dir, options)
    }

    /// Spec: `SessionManager.open` — open a specific session file.
    pub fn open(
        path: &str,
        session_dir: Option<&str>,
        cwd_override: Option<&str>,
        agent_dir: &str,
    ) -> Result<Self> {
        let resolved_path = resolve_path(path);
        // Extract cwd from the session header if possible
        let entries = load_entries_from_file(Path::new(&resolved_path));
        let header_cwd = entries
            .iter()
            .find(|e| entry_type(e) == "session")
            .and_then(|h| field(h, "cwd"))
            .map(str::to_owned);
        let cwd = cwd_override
            .map(str::to_owned)
            .or(header_cwd)
            .unwrap_or_else(process_cwd);
        // If no sessionDir provided, derive from the file's parent directory
        let dir = match session_dir {
            Some(dir) => normalize_path(dir),
            None => resolve_path_in("..", &resolved_path),
        };
        Self::new(&cwd, &dir, Some(&resolved_path), true, agent_dir, None)
    }

    /// Spec: `SessionManager.continueRecent` — most recent session, or new.
    pub fn continue_recent(cwd: &str, session_dir: Option<&str>, agent_dir: &str) -> Result<Self> {
        let dir = match session_dir {
            Some(dir) => normalize_path(dir),
            None => get_default_session_dir(cwd, agent_dir)?
                .to_string_lossy()
                .into_owned(),
        };
        let filter_cwd = session_dir.is_some()
            && Path::new(&dir) != get_default_session_dir_path(cwd, agent_dir);
        let most_recent = find_most_recent_session(Path::new(&dir), filter_cwd.then_some(cwd));
        match most_recent {
            Some(file) => Self::new(
                cwd,
                &dir,
                Some(&file.to_string_lossy()),
                true,
                agent_dir,
                None,
            ),
            None => Self::new(cwd, &dir, None, true, agent_dir, None),
        }
    }

    /// Spec: `SessionManager.inMemory` — no file persistence.
    pub fn in_memory() -> Self {
        Self::in_memory_at(&process_cwd())
    }

    /// `inMemory` with an explicit cwd.
    pub fn in_memory_at(cwd: &str) -> Self {
        // Infallible: no session dir to create, no file to read, no id to
        // validate — new_session with defaults cannot fail.
        Self::new(cwd, "", None, false, "", None).unwrap_or_else(|_| Self {
            session_id: create_session_id(),
            session_file: None,
            session_dir: String::new(),
            cwd: resolve_path(cwd),
            agent_dir: String::new(),
            persist: false,
            flushed: false,
            file_entries: Vec::new(),
            by_id: HashMap::new(),
            labels_by_id: IndexMap::new(),
            label_timestamps_by_id: HashMap::new(),
            leaf_id: None,
        })
    }

    /// Spec: `SessionManager.forkFrom` — fork a session from another
    /// project directory into `target_cwd` with full history.
    pub fn fork_from(
        source_path: &str,
        target_cwd: &str,
        session_dir: Option<&str>,
        agent_dir: &str,
        options: Option<NewSessionOptions>,
    ) -> Result<Self> {
        let options = options.unwrap_or_default();
        let resolved_source = resolve_path(source_path);
        let resolved_target_cwd = resolve_path(target_cwd);
        let source_entries = load_entries_from_file(Path::new(&resolved_source));
        if source_entries.is_empty() {
            return Err(SessionError::ForkSourceInvalid(resolved_source));
        }
        if !source_entries.iter().any(|e| entry_type(e) == "session") {
            return Err(SessionError::ForkSourceMissingHeader(resolved_source));
        }

        let dir = match session_dir {
            Some(dir) => normalize_path(dir),
            None => get_default_session_dir(&resolved_target_cwd, agent_dir)?
                .to_string_lossy()
                .into_owned(),
        };
        if !Path::new(&dir).exists() {
            std::fs::create_dir_all(&dir)?;
        }

        if let Some(id) = &options.id {
            assert_valid_session_id(id)?;
        }
        let new_session_id = options.id.unwrap_or_else(create_session_id);
        let timestamp = now_iso();
        let file_timestamp = timestamp.replace([':', '.'], "-");
        let new_session_file = Path::new(&dir)
            .join(format!("{file_timestamp}_{new_session_id}.jsonl"))
            .to_string_lossy()
            .into_owned();

        // Write new header pointing to source as parent, with updated cwd
        let mut header = Map::new();
        header.insert("type".into(), "session".into());
        header.insert("version".into(), CURRENT_SESSION_VERSION.into());
        header.insert("id".into(), new_session_id.into());
        header.insert("timestamp".into(), timestamp.into());
        header.insert("cwd".into(), resolved_target_cwd.clone().into());
        header.insert("parentSession".into(), resolved_source.clone().into());
        let mut fd = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&new_session_file)?;
        writeln!(fd, "{}", serde_json::to_string(&Value::Object(header))?)?;
        // Copy all non-header entries from source
        for entry in &source_entries {
            if entry_type(entry) != "session" {
                writeln!(fd, "{}", serde_json::to_string(entry)?)?;
            }
        }
        drop(fd);

        Self::new(
            &resolved_target_cwd,
            &dir,
            Some(&new_session_file),
            true,
            agent_dir,
            None,
        )
    }

    /// Spec: `SessionManager.list` — sessions for a directory, most
    /// recently modified first.
    pub fn list(
        cwd: &str,
        session_dir: Option<&str>,
        agent_dir: &str,
        on_progress: Option<SessionListProgress<'_>>,
    ) -> Result<Vec<SessionInfo>> {
        let dir = match session_dir {
            Some(dir) => normalize_path(dir),
            None => get_default_session_dir(cwd, agent_dir)?
                .to_string_lossy()
                .into_owned(),
        };
        let filter_cwd = session_dir.is_some()
            && Path::new(&dir) != get_default_session_dir_path(cwd, agent_dir);
        let resolved_cwd = resolve_path(cwd);
        let mut sessions: Vec<SessionInfo> =
            list_sessions_from_dir(Path::new(&dir), on_progress, 0, None)
                .into_iter()
                .filter(|s| !filter_cwd || session_cwd_matches(Some(&s.cwd), &resolved_cwd))
                .collect();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.modified_ms));
        Ok(sessions)
    }

    /// Spec: `SessionManager.listAll` — all sessions across all project
    /// directories under `default_sessions_dir` (the spec's
    /// `getSessionsDir()`), or a single custom flat directory.
    pub fn list_all(
        custom_session_dir: Option<&str>,
        default_sessions_dir: &Path,
        mut on_progress: Option<SessionListProgress<'_>>,
    ) -> Vec<SessionInfo> {
        if let Some(custom) = custom_session_dir {
            let dir = normalize_path(custom);
            let mut sessions = list_sessions_from_dir(Path::new(&dir), on_progress, 0, None);
            sessions.sort_by_key(|session| std::cmp::Reverse(session.modified_ms));
            return sessions;
        }

        if !default_sessions_dir.exists() {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(default_sessions_dir) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        dirs.sort();

        // Count total files first for accurate progress
        let dir_files: Vec<Vec<PathBuf>> = dirs.iter().map(|d| jsonl_files(d)).collect();
        let total_files: usize = dir_files.iter().map(Vec::len).sum();

        let mut loaded = 0usize;
        let mut sessions: Vec<SessionInfo> = Vec::new();
        for file in dir_files.iter().flatten() {
            if let Some(info) = build_session_info(file) {
                sessions.push(info);
            }
            loaded += 1;
            if let Some(cb) = on_progress.as_deref_mut() {
                cb(loaded, total_files);
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.modified_ms));
        sessions
    }
}

fn process_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".to_owned())
}
