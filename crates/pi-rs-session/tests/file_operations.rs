//! Port of `test/session-manager/file-operations.test.ts`. The
//! larger-than-Node's-max-string-length case is not ported: the Rust
//! reader is chunked by construction (no giant-string code path exists).
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::path::Path;

use common::{assistant_msg, sleep_ms, user_msg};
use pi_rs_session::{SessionManager, find_most_recent_session, load_entries_from_file};
use serde_json::{Value, json};

// -----------------------------------------------------------------------------
// loadEntriesFromFile
// -----------------------------------------------------------------------------

#[test]
fn load_returns_empty_for_nonexistent_file() {
    let temp = tempfile::tempdir().unwrap();
    assert!(load_entries_from_file(&temp.path().join("nonexistent.jsonl")).is_empty());
}

#[test]
fn load_returns_empty_for_empty_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("empty.jsonl");
    fs::write(&file, "").unwrap();
    assert!(load_entries_from_file(&file).is_empty());
}

#[test]
fn load_returns_empty_without_valid_session_header() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("no-header.jsonl");
    fs::write(&file, "{\"type\":\"message\",\"id\":\"1\"}\n").unwrap();
    assert!(load_entries_from_file(&file).is_empty());
}

#[test]
fn load_returns_empty_for_malformed_json() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("malformed.jsonl");
    fs::write(&file, "not json\n").unwrap();
    assert!(load_entries_from_file(&file).is_empty());
}

#[test]
fn load_valid_session_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("valid.jsonl");
    fs::write(
        &file,
        concat!(
            "{\"type\":\"session\",\"id\":\"abc\",\"timestamp\":\"2025-01-01T00:00:00Z\",\"cwd\":\"/tmp\"}\n",
            "{\"type\":\"message\",\"id\":\"1\",\"parentId\":null,\"timestamp\":\"2025-01-01T00:00:01Z\",\"message\":{\"role\":\"user\",\"content\":\"hi\",\"timestamp\":1}}\n",
        ),
    )
    .unwrap();
    let entries = load_entries_from_file(&file);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["type"], "session");
    assert_eq!(entries[1]["type"], "message");
}

#[test]
fn load_skips_malformed_lines_but_keeps_valid_ones() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("mixed.jsonl");
    fs::write(
        &file,
        concat!(
            "{\"type\":\"session\",\"id\":\"abc\",\"timestamp\":\"2025-01-01T00:00:00Z\",\"cwd\":\"/tmp\"}\n",
            "not valid json\n",
            "{\"type\":\"message\",\"id\":\"1\",\"parentId\":null,\"timestamp\":\"2025-01-01T00:00:01Z\",\"message\":{\"role\":\"user\",\"content\":\"hi\",\"timestamp\":1}}\n",
        ),
    )
    .unwrap();
    assert_eq!(load_entries_from_file(&file).len(), 2);
}

// -----------------------------------------------------------------------------
// findMostRecentSession
// -----------------------------------------------------------------------------

fn write_header(path: &Path, id: &str, cwd: &str) {
    fs::write(
        path,
        format!(
            "{}\n",
            json!({ "type": "session", "id": id, "timestamp": "2025-01-01T00:00:00Z", "cwd": cwd })
        ),
    )
    .unwrap();
}

#[test]
fn find_returns_none_for_empty_directory() {
    let temp = tempfile::tempdir().unwrap();
    assert!(find_most_recent_session(temp.path(), None).is_none());
}

#[test]
fn find_returns_none_for_nonexistent_directory() {
    let temp = tempfile::tempdir().unwrap();
    assert!(find_most_recent_session(&temp.path().join("nonexistent"), None).is_none());
}

#[test]
fn find_ignores_non_jsonl_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("file.txt"), "hello").unwrap();
    fs::write(temp.path().join("file.json"), "{}").unwrap();
    assert!(find_most_recent_session(temp.path(), None).is_none());
}

#[test]
fn find_ignores_jsonl_without_valid_header() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("invalid.jsonl"),
        "{\"type\":\"message\"}\n",
    )
    .unwrap();
    assert!(find_most_recent_session(temp.path(), None).is_none());
}

#[test]
fn find_returns_single_valid_session_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("session.jsonl");
    write_header(&file, "abc", "/tmp");
    assert_eq!(find_most_recent_session(temp.path(), None), Some(file));
}

#[test]
fn find_returns_most_recently_modified() {
    let temp = tempfile::tempdir().unwrap();
    let file1 = temp.path().join("older.jsonl");
    let file2 = temp.path().join("newer.jsonl");
    write_header(&file1, "old", "/tmp");
    sleep_ms(10);
    write_header(&file2, "new", "/tmp");
    assert_eq!(find_most_recent_session(temp.path(), None), Some(file2));
}

#[test]
fn find_skips_invalid_files_and_returns_valid_one() {
    let temp = tempfile::tempdir().unwrap();
    let invalid = temp.path().join("invalid.jsonl");
    let valid = temp.path().join("valid.jsonl");
    fs::write(&invalid, "{\"type\":\"not-session\"}\n").unwrap();
    sleep_ms(10);
    write_header(&valid, "abc", "/tmp");
    assert_eq!(find_most_recent_session(temp.path(), None), Some(valid));
}

#[test]
fn find_filters_by_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    let file_a = temp.path().join("a.jsonl");
    let file_b = temp.path().join("b.jsonl");

    write_header(&file_a, "a", &project_a.to_string_lossy());
    sleep_ms(10);
    write_header(&file_b, "b", &project_b.to_string_lossy());

    assert_eq!(
        find_most_recent_session(temp.path(), Some(&project_a.to_string_lossy())),
        Some(file_a)
    );
    assert_eq!(
        find_most_recent_session(temp.path(), Some(&project_b.to_string_lossy())),
        Some(file_b)
    );
}

// -----------------------------------------------------------------------------
// Custom flat session directory
// -----------------------------------------------------------------------------

fn create_persisted_session(cwd: &str, session_dir: &str, label: &str) -> String {
    let mut session = SessionManager::create(cwd, Some(session_dir), "", None).unwrap();
    session.append_message(user_msg(label)).unwrap();
    session
        .append_message(assistant_msg(&format!("reply to {label}")))
        .unwrap();
    session.get_session_file().unwrap().to_owned()
}

#[test]
fn scopes_current_folder_apis_by_cwd_while_listing_all_flat_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let flat_dir = temp.path().to_string_lossy().into_owned();
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    fs::create_dir_all(&project_a).unwrap();
    fs::create_dir_all(&project_b).unwrap();
    let project_a = project_a.to_string_lossy().into_owned();
    let project_b = project_b.to_string_lossy().into_owned();

    let session_a = create_persisted_session(&project_a, &flat_dir, "from A");
    sleep_ms(10);
    let session_b = create_persisted_session(&project_b, &flat_dir, "from B");

    let current_a = SessionManager::list(&project_a, Some(&flat_dir), "", None).unwrap();
    assert_eq!(
        current_a
            .iter()
            .map(|s| s.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec![session_a.clone()]
    );

    let all = SessionManager::list_all(Some(&flat_dir), Path::new("/nonexistent"), None);
    let all_paths: std::collections::HashSet<String> = all
        .iter()
        .map(|s| s.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        all_paths,
        [session_a.clone(), session_b].into_iter().collect()
    );

    let continued_a = SessionManager::continue_recent(&project_a, Some(&flat_dir), "").unwrap();
    assert_eq!(continued_a.get_session_file(), Some(session_a.as_str()));
}

// -----------------------------------------------------------------------------
// setSessionFile with corrupted files
// -----------------------------------------------------------------------------

#[test]
fn truncates_and_rewrites_empty_file_with_valid_header() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let empty_file = temp.path().join("empty.jsonl");
    fs::write(&empty_file, "").unwrap();

    let sm = SessionManager::open(&empty_file.to_string_lossy(), Some(&dir), None, "").unwrap();

    // Should have created a new session with valid header
    assert!(!sm.get_session_id().is_empty());
    let header = sm.get_header().unwrap();
    assert_eq!(header["type"], "session");

    // File should now contain a valid header
    let content = fs::read_to_string(&empty_file).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let header: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(header["type"], "session");
    assert_eq!(header["id"], sm.get_session_id());
}

#[test]
fn truncates_and_rewrites_file_without_valid_header() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let no_header_file = temp.path().join("no-header.jsonl");
    // File with messages but no session header (corrupted state)
    fs::write(
        &no_header_file,
        "{\"type\":\"message\",\"id\":\"abc\",\"parentId\":\"orphaned\",\"timestamp\":\"2025-01-01T00:00:00Z\",\"message\":{\"role\":\"assistant\",\"content\":\"test\"}}\n",
    )
    .unwrap();

    let sm = SessionManager::open(&no_header_file.to_string_lossy(), Some(&dir), None, "").unwrap();

    assert!(!sm.get_session_id().is_empty());
    assert_eq!(sm.get_header().unwrap()["type"], "session");

    // File should now contain only a valid header (old content truncated)
    let content = fs::read_to_string(&no_header_file).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let header: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(header["type"], "session");
    assert_eq!(header["id"], sm.get_session_id());
}

#[test]
fn preserves_explicit_session_file_path_when_recovering() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let explicit_path = temp.path().join("my-session.jsonl");
    fs::write(&explicit_path, "").unwrap();

    let sm = SessionManager::open(&explicit_path.to_string_lossy(), Some(&dir), None, "").unwrap();
    assert_eq!(
        sm.get_session_file(),
        Some(explicit_path.to_string_lossy().as_ref())
    );
}

#[test]
fn subsequent_loads_of_recovered_file_work() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let corrupted = temp.path().join("corrupted.jsonl");
    fs::write(&corrupted, "garbage content\n").unwrap();

    // First open recovers the file
    let sm1 = SessionManager::open(&corrupted.to_string_lossy(), Some(&dir), None, "").unwrap();
    let session_id = sm1.get_session_id().to_owned();

    // Second open should load the recovered file successfully
    let sm2 = SessionManager::open(&corrupted.to_string_lossy(), Some(&dir), None, "").unwrap();
    assert_eq!(sm2.get_session_id(), session_id);
    assert_eq!(sm2.get_header().unwrap()["type"], "session");
}

#[test]
fn export_branch_jsonl_writes_only_current_linear_branch() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let mut sm = SessionManager::create("/tmp/project", Some(&dir), "", None).unwrap();
    let root = sm.append_message(user_msg("root")).unwrap();
    let abandoned = sm.append_message(assistant_msg("abandoned")).unwrap();
    sm.branch(&root).unwrap();
    let kept = sm.append_message(assistant_msg("kept")).unwrap();

    let output = temp.path().join("nested/export.jsonl");
    let path = sm
        .export_branch_jsonl(&output.to_string_lossy(), "2025-07-11T12:34:56.789Z")
        .unwrap();
    assert_eq!(path, output.to_string_lossy());
    let rows: Vec<Value> = fs::read_to_string(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["timestamp"], "2025-07-11T12:34:56.789Z");
    assert_eq!(rows[1]["id"], root);
    assert_eq!(rows[1]["parentId"], Value::Null);
    assert_eq!(rows[2]["id"], kept);
    assert_eq!(rows[2]["parentId"], root);
    assert!(rows.iter().all(|row| row["id"] != abandoned));
}
