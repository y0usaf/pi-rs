//! P2 contract tests: named-record CRUD over the session JSONL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::OpenOptions;
use std::io::Write;

use pi_rs_session::records::{RecordError, RecordOp, RecordStore};

fn temp_store() -> (tempfile::TempDir, RecordStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let store = RecordStore::new(&path);
    (dir, store)
}

#[test]
fn put_get_round_trip() {
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!({"text": "hello"})).unwrap();
    let v = store.get("memories", "m1").unwrap().unwrap();
    assert_eq!(v, serde_json::json!({"text": "hello"}));
}

#[test]
fn latest_value_wins() {
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!({"n": 1})).unwrap();
    store.put("memories", "m1", serde_json::json!({"n": 2})).unwrap();
    let v = store.get("memories", "m1").unwrap().unwrap();
    assert_eq!(v, serde_json::json!({"n": 2}));
}

#[test]
fn tombstone_hides_key() {
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!({"n": 1})).unwrap();
    store.delete("memories", "m1").unwrap();
    assert!(store.get("memories", "m1").unwrap().is_none());
    assert!(store.list("memories").unwrap().is_empty());
    // A put after a delete revives the key (latest op wins).
    store.put("memories", "m1", serde_json::json!({"n": 3})).unwrap();
    assert!(store.get("memories", "m1").unwrap().is_some());
}

#[test]
fn list_collections_are_isolated() {
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!(1)).unwrap();
    store.put("skills", "s1", serde_json::json!(2)).unwrap();
    store.put("memories", "m2", serde_json::json!(3)).unwrap();
    let memories = store.list("memories").unwrap();
    assert_eq!(memories.len(), 2);
    assert!(memories.iter().all(|(k, _)| k == "m1" || k == "m2"));
    let skills = store.list("skills").unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].0, "s1");
}

#[test]
fn partial_trailing_line_is_tolerated() {
    // Simulates a kill mid-append: a truncated JSON line at EOF.
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!(1)).unwrap();
    let mut f = OpenOptions::new()
        .append(true)
        .open(store.path())
        .unwrap();
    f.write_all(b"{\"type\": \"record\", \"collection\": \"memories\", \"ke").unwrap();
    f.flush().unwrap();
    drop(f);
    // Fold still returns the complete record; the partial line is skipped.
    let v = store.get("memories", "m1").unwrap().unwrap();
    assert_eq!(v, serde_json::json!(1));
    assert_eq!(store.count_entries().unwrap(), 1);
}

#[test]
fn malformed_record_entry_reports_loudly() {
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!(1)).unwrap();
    let mut f = OpenOptions::new()
        .append(true)
        .open(store.path())
        .unwrap();
    f.write_all(b"{\"type\": \"record\", \"collection\": \"memories\"}\n").unwrap();
    f.flush().unwrap();
    drop(f);
    // A record entry missing its key/op is malformed -> loud error, not a
    // silent skip.
    let err = store.fold().unwrap_err();
    assert!(matches!(err, RecordError::Malformed(_)), "expected Malformed, got {err:?}");
}

#[test]
fn non_record_entries_are_ignored() {
    let (_d, store) = temp_store();
    store.put("memories", "m1", serde_json::json!(1)).unwrap();
    let mut f = OpenOptions::new()
        .append(true)
        .open(store.path())
        .unwrap();
    f.write_all(b"{\"type\": \"message\", \"message\": {\"role\": \"user\"}}\n").unwrap();
    f.write_all(b"{\"type\": \"compaction\", \"summary\": \"x\"}\n").unwrap();
    f.flush().unwrap();
    drop(f);
    assert_eq!(store.count_entries().unwrap(), 1);
    assert!(store.get("memories", "m1").unwrap().is_some());
}

#[test]
fn missing_store_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("absent.jsonl"));
    assert!(store.get("memories", "m1").unwrap().is_none());
    assert!(store.list("memories").unwrap().is_empty());
    assert_eq!(store.count_entries().unwrap(), 0);
}

#[test]
fn record_op_serialization_shape() {
    // Wire shape: type/collection/key/op snake_case, value present only on put.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let store = RecordStore::new(&path);
    store.put("memories", "m1", serde_json::json!({"a": 1})).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let entry: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(entry["type"], "record");
    assert_eq!(entry["collection"], "memories");
    assert_eq!(entry["key"], "m1");
    assert_eq!(entry["op"], "put");
    assert_eq!(entry["value"], serde_json::json!({"a": 1}));
    assert_eq!(entry["op"].as_str(), Some("put"));
}
