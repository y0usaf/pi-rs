//! Port of `test/session-manager/labels.test.ts`.
#![allow(clippy::unwrap_used)]

mod common;

use common::{assistant_msg, user_msg};
use pi_rs_session::SessionManager;

#[test]
fn sets_and_gets_labels() {
    let mut session = SessionManager::in_memory();
    let msg_id = session.append_message(user_msg("hello")).unwrap();

    // No label initially
    assert_eq!(session.get_label(&msg_id), None);

    // Set a label
    let label_id = session
        .append_label_change(&msg_id, Some("checkpoint"))
        .unwrap();
    assert_eq!(session.get_label(&msg_id), Some("checkpoint"));

    // Label entry should be in entries
    let entries = session.get_entries();
    let label_entry = entries.iter().find(|e| e["type"] == "label").unwrap();
    assert_eq!(label_entry["id"], label_id.as_str());
    assert_eq!(label_entry["targetId"], msg_id.as_str());
    assert_eq!(label_entry["label"], "checkpoint");
}

#[test]
fn clears_labels_with_none() {
    let mut session = SessionManager::in_memory();
    let msg_id = session.append_message(user_msg("hello")).unwrap();

    session
        .append_label_change(&msg_id, Some("checkpoint"))
        .unwrap();
    assert_eq!(session.get_label(&msg_id), Some("checkpoint"));

    // Clear the label
    session.append_label_change(&msg_id, None).unwrap();
    assert_eq!(session.get_label(&msg_id), None);
}

#[test]
fn last_label_wins() {
    let mut session = SessionManager::in_memory();
    let msg_id = session.append_message(user_msg("hello")).unwrap();

    session.append_label_change(&msg_id, Some("first")).unwrap();
    session
        .append_label_change(&msg_id, Some("second"))
        .unwrap();
    let last_label_id = session.append_label_change(&msg_id, Some("third")).unwrap();

    assert_eq!(session.get_label(&msg_id), Some("third"));

    let entries = session.get_entries();
    let last_label = entries
        .iter()
        .find(|e| e["id"] == last_label_id.as_str())
        .unwrap();
    let tree = session.get_tree();
    let msg_node = tree
        .iter()
        .find(|n| n.entry["id"] == msg_id.as_str())
        .unwrap();
    assert_eq!(
        msg_node.label_timestamp.as_deref(),
        last_label["timestamp"].as_str()
    );
}

#[test]
fn labels_are_included_in_tree_nodes() {
    let mut session = SessionManager::in_memory();

    let msg1_id = session.append_message(user_msg("hello")).unwrap();
    let msg2_id = session.append_message(assistant_msg("hi")).unwrap();

    let msg1_label_id = session
        .append_label_change(&msg1_id, Some("start"))
        .unwrap();
    let msg2_label_id = session
        .append_label_change(&msg2_id, Some("response"))
        .unwrap();

    let entries = session.get_entries();
    let msg1_label = entries
        .iter()
        .find(|e| e["id"] == msg1_label_id.as_str())
        .unwrap();
    let msg2_label = entries
        .iter()
        .find(|e| e["id"] == msg2_label_id.as_str())
        .unwrap();
    let tree = session.get_tree();

    // Find the message nodes (skip label entries)
    let msg1_node = tree
        .iter()
        .find(|n| n.entry["id"] == msg1_id.as_str())
        .unwrap();
    assert_eq!(msg1_node.label.as_deref(), Some("start"));
    assert_eq!(
        msg1_node.label_timestamp.as_deref(),
        msg1_label["timestamp"].as_str()
    );

    // msg2 is a child of msg1
    let msg2_node = msg1_node
        .children
        .iter()
        .find(|n| n.entry["id"] == msg2_id.as_str())
        .unwrap();
    assert_eq!(msg2_node.label.as_deref(), Some("response"));
    assert_eq!(
        msg2_node.label_timestamp.as_deref(),
        msg2_label["timestamp"].as_str()
    );
}

#[test]
fn labels_are_preserved_in_create_branched_session() {
    let mut session = SessionManager::in_memory();

    let msg1_id = session.append_message(user_msg("hello")).unwrap();
    let msg2_id = session.append_message(assistant_msg("hi")).unwrap();

    let msg1_label_id = session
        .append_label_change(&msg1_id, Some("important"))
        .unwrap();
    let msg2_label_id = session
        .append_label_change(&msg2_id, Some("also-important"))
        .unwrap();
    let original_entries = session.get_entries();
    let msg1_label = original_entries
        .iter()
        .find(|e| e["id"] == msg1_label_id.as_str())
        .unwrap()
        .clone();
    let msg2_label = original_entries
        .iter()
        .find(|e| e["id"] == msg2_label_id.as_str())
        .unwrap()
        .clone();

    // Branch from msg2 (in-memory mode returns None, but updates internal state)
    session.create_branched_session(&msg2_id).unwrap();

    // Labels should be preserved
    assert_eq!(session.get_label(&msg1_id), Some("important"));
    assert_eq!(session.get_label(&msg2_id), Some("also-important"));

    // New label entries should exist
    let entries = session.get_entries();
    let label_entries: Vec<_> = entries.iter().filter(|e| e["type"] == "label").collect();
    assert_eq!(label_entries.len(), 2);

    let tree = session.get_tree();
    let msg1_node = tree
        .iter()
        .find(|n| n.entry["id"] == msg1_id.as_str())
        .unwrap();
    let msg2_node = msg1_node
        .children
        .iter()
        .find(|n| n.entry["id"] == msg2_id.as_str())
        .unwrap();
    assert_eq!(
        msg1_node.label_timestamp.as_deref(),
        msg1_label["timestamp"].as_str()
    );
    assert_eq!(
        msg2_node.label_timestamp.as_deref(),
        msg2_label["timestamp"].as_str()
    );
}

#[test]
fn labels_not_on_path_are_not_preserved() {
    let mut session = SessionManager::in_memory();

    let msg1_id = session.append_message(user_msg("hello")).unwrap();
    let msg2_id = session.append_message(assistant_msg("hi")).unwrap();
    let msg3_id = session.append_message(user_msg("followup")).unwrap();

    // Label all messages
    session
        .append_label_change(&msg1_id, Some("first"))
        .unwrap();
    session
        .append_label_change(&msg2_id, Some("second"))
        .unwrap();
    session
        .append_label_change(&msg3_id, Some("third"))
        .unwrap();

    // Branch from msg2 (excludes msg3)
    session.create_branched_session(&msg2_id).unwrap();

    // Only labels for msg1 and msg2 should be preserved
    assert_eq!(session.get_label(&msg1_id), Some("first"));
    assert_eq!(session.get_label(&msg2_id), Some("second"));
    assert_eq!(session.get_label(&msg3_id), None);
}

#[test]
fn labels_are_not_included_in_build_session_context() {
    let mut session = SessionManager::in_memory();

    let msg_id = session.append_message(user_msg("hello")).unwrap();
    session
        .append_label_change(&msg_id, Some("checkpoint"))
        .unwrap();

    let ctx = session.build_session_context();
    assert_eq!(ctx.messages.len(), 1);
    assert_eq!(ctx.messages[0]["role"], "user");
}

#[test]
fn throws_when_labeling_nonexistent_entry() {
    let mut session = SessionManager::in_memory();
    let err = session
        .append_label_change("non-existent", Some("label"))
        .unwrap_err();
    assert_eq!(err.to_string(), "Entry non-existent not found");
}
