//! Port of `test/session-manager/migration.test.ts`.
#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};

use pi_rs_session::migrate_session_entries;

#[test]
fn adds_id_parent_id_to_v1_entries() {
    let mut entries: Vec<Value> = vec![
        json!({ "type": "session", "id": "sess-1", "timestamp": "2025-01-01T00:00:00Z", "cwd": "/tmp" }),
        json!({
            "type": "message",
            "timestamp": "2025-01-01T00:00:01Z",
            "message": { "role": "user", "content": "hi", "timestamp": 1 },
        }),
        json!({
            "type": "message",
            "timestamp": "2025-01-01T00:00:02Z",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "hello" }],
                "api": "test",
                "provider": "test",
                "model": "test",
                "usage": { "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0 },
                "stopReason": "stop",
                "timestamp": 2,
            },
        }),
    ];

    migrate_session_entries(&mut entries);

    // Header should have version set (v3 is current after hookMessage->custom migration)
    assert_eq!(entries[0]["version"], 3);

    // Entries should have id/parentId
    let id1 = entries[1]["id"].as_str().unwrap();
    assert_eq!(id1.len(), 8);
    assert_eq!(entries[1]["parentId"], Value::Null);

    let id2 = entries[2]["id"].as_str().unwrap();
    assert_eq!(id2.len(), 8);
    assert_eq!(entries[2]["parentId"].as_str().unwrap(), id1);
}

#[test]
fn idempotent_skips_already_migrated() {
    let mut entries: Vec<Value> = vec![
        json!({ "type": "session", "id": "sess-1", "version": 2, "timestamp": "2025-01-01T00:00:00Z", "cwd": "/tmp" }),
        json!({
            "type": "message",
            "id": "abc12345",
            "parentId": null,
            "timestamp": "2025-01-01T00:00:01Z",
            "message": { "role": "user", "content": "hi", "timestamp": 1 },
        }),
        json!({
            "type": "message",
            "id": "def67890",
            "parentId": "abc12345",
            "timestamp": "2025-01-01T00:00:02Z",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "hello" }],
                "api": "test",
                "provider": "test",
                "model": "test",
                "usage": { "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0 },
                "stopReason": "stop",
                "timestamp": 2,
            },
        }),
    ];

    migrate_session_entries(&mut entries);

    // IDs should be unchanged
    assert_eq!(entries[1]["id"], "abc12345");
    assert_eq!(entries[2]["id"], "def67890");
    assert_eq!(entries[2]["parentId"], "abc12345");
}

#[test]
fn v2_to_v3_renames_hook_message_role() {
    let mut entries: Vec<Value> = vec![
        json!({ "type": "session", "id": "s", "version": 2, "timestamp": "2025-01-01T00:00:00Z", "cwd": "/tmp" }),
        json!({
            "type": "message",
            "id": "abc12345",
            "parentId": null,
            "timestamp": "2025-01-01T00:00:01Z",
            "message": { "role": "hookMessage", "customType": "x", "content": "hi", "display": true, "timestamp": 1 },
        }),
    ];
    migrate_session_entries(&mut entries);
    assert_eq!(entries[0]["version"], 3);
    assert_eq!(entries[1]["message"]["role"], "custom");
}
