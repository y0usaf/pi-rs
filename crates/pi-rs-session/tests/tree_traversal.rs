//! Port of `test/session-manager/tree-traversal.test.ts`.
#![allow(clippy::unwrap_used)]

mod common;

use common::{assistant_msg, user_msg};
use pi_rs_session::{SessionError, SessionManager};
use serde_json::{Value, json};

#[test]
fn append_message_creates_entry_with_correct_parent_chain() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("first")).unwrap();
    let id2 = session.append_message(assistant_msg("second")).unwrap();
    let id3 = session.append_message(user_msg("third")).unwrap();

    let entries = session.get_entries();
    assert_eq!(entries.len(), 3);

    assert_eq!(entries[0]["id"], id1.as_str());
    assert_eq!(entries[0]["parentId"], Value::Null);
    assert_eq!(entries[0]["type"], "message");

    assert_eq!(entries[1]["id"], id2.as_str());
    assert_eq!(entries[1]["parentId"], id1.as_str());

    assert_eq!(entries[2]["id"], id3.as_str());
    assert_eq!(entries[2]["parentId"], id2.as_str());
}

#[test]
fn append_thinking_level_change_integrates_into_tree() {
    let mut session = SessionManager::in_memory();

    let msg_id = session.append_message(user_msg("hello")).unwrap();
    let thinking_id = session.append_thinking_level_change("high").unwrap();
    session.append_message(assistant_msg("response")).unwrap();

    let entries = session.get_entries();
    assert_eq!(entries.len(), 3);

    let thinking = entries
        .iter()
        .find(|e| e["type"] == "thinking_level_change")
        .unwrap();
    assert_eq!(thinking["id"], thinking_id.as_str());
    assert_eq!(thinking["parentId"], msg_id.as_str());

    assert_eq!(entries[2]["parentId"], thinking_id.as_str());
}

#[test]
fn append_model_change_integrates_into_tree() {
    let mut session = SessionManager::in_memory();

    let msg_id = session.append_message(user_msg("hello")).unwrap();
    let model_id = session.append_model_change("openai", "gpt-4").unwrap();
    session.append_message(assistant_msg("response")).unwrap();

    let entries = session.get_entries();
    let model_entry = entries
        .iter()
        .find(|e| e["type"] == "model_change")
        .unwrap();
    assert_eq!(model_entry["id"], model_id.as_str());
    assert_eq!(model_entry["parentId"], msg_id.as_str());
    assert_eq!(model_entry["provider"], "openai");
    assert_eq!(model_entry["modelId"], "gpt-4");

    assert_eq!(entries[2]["parentId"], model_id.as_str());
}

#[test]
fn append_compaction_integrates_into_tree() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    let compaction_id = session
        .append_compaction("summary", &id1, 1000, None, None)
        .unwrap();
    session.append_message(user_msg("3")).unwrap();

    let entries = session.get_entries();
    let compaction = entries.iter().find(|e| e["type"] == "compaction").unwrap();
    assert_eq!(compaction["id"], compaction_id.as_str());
    assert_eq!(compaction["parentId"], id2.as_str());
    assert_eq!(compaction["summary"], "summary");
    assert_eq!(compaction["firstKeptEntryId"], id1.as_str());
    assert_eq!(compaction["tokensBefore"], 1000);

    assert_eq!(entries[3]["parentId"], compaction_id.as_str());
}

#[test]
fn append_custom_entry_integrates_into_tree() {
    let mut session = SessionManager::in_memory();

    let msg_id = session.append_message(user_msg("hello")).unwrap();
    let custom_id = session
        .append_custom_entry("my_data", Some(json!({ "key": "value" })))
        .unwrap();
    session.append_message(assistant_msg("response")).unwrap();

    let entries = session.get_entries();
    let custom = entries.iter().find(|e| e["type"] == "custom").unwrap();
    assert_eq!(custom["id"], custom_id.as_str());
    assert_eq!(custom["parentId"], msg_id.as_str());
    assert_eq!(custom["customType"], "my_data");
    assert_eq!(custom["data"], json!({ "key": "value" }));

    assert_eq!(entries[2]["parentId"], custom_id.as_str());
}

#[test]
fn leaf_pointer_advances_after_each_append() {
    let mut session = SessionManager::in_memory();

    assert_eq!(session.get_leaf_id(), None);

    let id1 = session.append_message(user_msg("1")).unwrap();
    assert_eq!(session.get_leaf_id(), Some(id1.as_str()));

    let id2 = session.append_message(assistant_msg("2")).unwrap();
    assert_eq!(session.get_leaf_id(), Some(id2.as_str()));

    let id3 = session.append_thinking_level_change("high").unwrap();
    assert_eq!(session.get_leaf_id(), Some(id3.as_str()));
}

#[test]
fn get_branch_empty_session() {
    let session = SessionManager::in_memory();
    assert!(session.get_branch(None).is_empty());
}

#[test]
fn get_branch_single_entry() {
    let mut session = SessionManager::in_memory();
    let id = session.append_message(user_msg("hello")).unwrap();

    let path = session.get_branch(None);
    assert_eq!(path.len(), 1);
    assert_eq!(path[0]["id"], id.as_str());
}

#[test]
fn get_branch_full_path_from_root_to_leaf() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    let id3 = session.append_thinking_level_change("high").unwrap();
    let id4 = session.append_message(user_msg("3")).unwrap();

    let path = session.get_branch(None);
    let ids: Vec<&str> = path.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        vec![id1.as_str(), id2.as_str(), id3.as_str(), id4.as_str()]
    );
}

#[test]
fn get_branch_from_specified_entry_to_root() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    session.append_message(user_msg("3")).unwrap();
    session.append_message(assistant_msg("4")).unwrap();

    let path = session.get_branch(Some(&id2));
    let ids: Vec<&str> = path.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![id1.as_str(), id2.as_str()]);
}

#[test]
fn get_tree_empty_session() {
    let session = SessionManager::in_memory();
    assert!(session.get_tree().is_empty());
}

#[test]
fn get_tree_single_root_for_linear_session() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    let id3 = session.append_message(user_msg("3")).unwrap();

    let tree = session.get_tree();
    assert_eq!(tree.len(), 1);

    let root = &tree[0];
    assert_eq!(root.entry["id"], id1.as_str());
    assert_eq!(root.children.len(), 1);
    assert_eq!(root.children[0].entry["id"], id2.as_str());
    assert_eq!(root.children[0].children.len(), 1);
    assert_eq!(root.children[0].children[0].entry["id"], id3.as_str());
    assert!(root.children[0].children[0].children.is_empty());
}

#[test]
fn get_tree_with_branches_after_branch() {
    let mut session = SessionManager::in_memory();

    // Build: 1 -> 2 -> 3
    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    let id3 = session.append_message(user_msg("3")).unwrap();

    // Branch from id2, add new path: 2 -> 4
    session.branch(&id2).unwrap();
    let id4 = session.append_message(user_msg("4-branch")).unwrap();

    let tree = session.get_tree();
    assert_eq!(tree.len(), 1);

    let root = &tree[0];
    assert_eq!(root.entry["id"], id1.as_str());
    assert_eq!(root.children.len(), 1);

    let node2 = &root.children[0];
    assert_eq!(node2.entry["id"], id2.as_str());
    assert_eq!(node2.children.len(), 2); // id3 and id4 are siblings

    let mut child_ids: Vec<&str> = node2
        .children
        .iter()
        .map(|c| c.entry["id"].as_str().unwrap())
        .collect();
    child_ids.sort_unstable();
    let mut expected = [id3.as_str(), id4.as_str()];
    expected.sort_unstable();
    assert_eq!(child_ids, expected);
}

#[test]
fn get_tree_multiple_branches_at_same_point() {
    let mut session = SessionManager::in_memory();

    session.append_message(user_msg("root")).unwrap();
    let id2 = session.append_message(assistant_msg("response")).unwrap();

    session.branch(&id2).unwrap();
    let id_a = session.append_message(user_msg("branch-A")).unwrap();
    session.branch(&id2).unwrap();
    let id_b = session.append_message(user_msg("branch-B")).unwrap();
    session.branch(&id2).unwrap();
    let id_c = session.append_message(user_msg("branch-C")).unwrap();

    let tree = session.get_tree();
    let node2 = &tree[0].children[0];
    assert_eq!(node2.entry["id"], id2.as_str());
    assert_eq!(node2.children.len(), 3);

    let mut branch_ids: Vec<&str> = node2
        .children
        .iter()
        .map(|c| c.entry["id"].as_str().unwrap())
        .collect();
    branch_ids.sort_unstable();
    let mut expected = [id_a.as_str(), id_b.as_str(), id_c.as_str()];
    expected.sort_unstable();
    assert_eq!(branch_ids, expected);
}

#[test]
fn get_tree_handles_deep_branching() {
    let mut session = SessionManager::in_memory();

    // Main path: 1 -> 2 -> 3 -> 4
    session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    let id3 = session.append_message(user_msg("3")).unwrap();
    session.append_message(assistant_msg("4")).unwrap();

    // Branch from 2: 2 -> 5 -> 6
    session.branch(&id2).unwrap();
    let id5 = session.append_message(user_msg("5")).unwrap();
    session.append_message(assistant_msg("6")).unwrap();

    // Branch from 5: 5 -> 7
    session.branch(&id5).unwrap();
    session.append_message(user_msg("7")).unwrap();

    let tree = session.get_tree();

    let node2 = &tree[0].children[0];
    assert_eq!(node2.children.len(), 2); // id3 and id5

    let node5 = node2
        .children
        .iter()
        .find(|c| c.entry["id"] == id5.as_str())
        .unwrap();
    assert_eq!(node5.children.len(), 2); // id6 and id7

    let node3 = node2
        .children
        .iter()
        .find(|c| c.entry["id"] == id3.as_str())
        .unwrap();
    assert_eq!(node3.children.len(), 1); // id4
}

#[test]
fn branch_moves_leaf_pointer() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    session.append_message(assistant_msg("2")).unwrap();
    let id3 = session.append_message(user_msg("3")).unwrap();

    assert_eq!(session.get_leaf_id(), Some(id3.as_str()));

    session.branch(&id1).unwrap();
    assert_eq!(session.get_leaf_id(), Some(id1.as_str()));
}

#[test]
fn branch_throws_for_nonexistent_entry() {
    let mut session = SessionManager::in_memory();
    session.append_message(user_msg("hello")).unwrap();

    let err = session.branch("nonexistent").unwrap_err();
    assert!(matches!(err, SessionError::EntryNotFound(_)));
    assert_eq!(err.to_string(), "Entry nonexistent not found");
}

#[test]
fn new_appends_become_children_of_branch_point() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    session.append_message(assistant_msg("2")).unwrap();

    session.branch(&id1).unwrap();
    let id3 = session.append_message(user_msg("branched")).unwrap();

    let entries = session.get_entries();
    let branched = entries.iter().find(|e| e["id"] == id3.as_str()).unwrap();
    assert_eq!(branched["parentId"], id1.as_str()); // sibling of id2
}

#[test]
fn branch_with_summary_inserts_summary_and_advances_leaf() {
    let mut session = SessionManager::in_memory();

    let id1 = session.append_message(user_msg("1")).unwrap();
    session.append_message(assistant_msg("2")).unwrap();
    session.append_message(user_msg("3")).unwrap();

    let summary_id = session
        .branch_with_summary(Some(&id1), "Summary of abandoned work", None, None)
        .unwrap();

    assert_eq!(session.get_leaf_id(), Some(summary_id.as_str()));

    let entries = session.get_entries();
    let summary = entries
        .iter()
        .find(|e| e["type"] == "branch_summary")
        .unwrap();
    assert_eq!(summary["parentId"], id1.as_str());
    assert_eq!(summary["summary"], "Summary of abandoned work");
}

#[test]
fn branch_with_summary_throws_for_nonexistent_entry() {
    let mut session = SessionManager::in_memory();
    session.append_message(user_msg("hello")).unwrap();

    let err = session
        .branch_with_summary(Some("nonexistent"), "summary", None, None)
        .unwrap_err();
    assert_eq!(err.to_string(), "Entry nonexistent not found");
}

#[test]
fn get_leaf_entry() {
    let mut session = SessionManager::in_memory();
    assert!(session.get_leaf_entry().is_none());

    session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();

    let leaf = session.get_leaf_entry().unwrap();
    assert_eq!(leaf["id"], id2.as_str());
}

#[test]
fn get_entry() {
    let mut session = SessionManager::in_memory();
    assert!(session.get_entry("nonexistent").is_none());

    let id1 = session.append_message(user_msg("first")).unwrap();
    let id2 = session.append_message(assistant_msg("second")).unwrap();

    let entry1 = session.get_entry(&id1).unwrap();
    assert_eq!(entry1["type"], "message");
    assert_eq!(entry1["message"]["content"], "first");

    let entry2 = session.get_entry(&id2).unwrap();
    assert_eq!(entry2["message"]["content"][0]["text"], "second");
}

#[test]
fn build_session_context_returns_current_branch_only() {
    let mut session = SessionManager::in_memory();

    // Main: 1 -> 2 -> 3
    session.append_message(user_msg("msg1")).unwrap();
    let id2 = session.append_message(assistant_msg("msg2")).unwrap();
    session.append_message(user_msg("msg3")).unwrap();

    // Branch from 2: 2 -> 4
    session.branch(&id2).unwrap();
    session
        .append_message(assistant_msg("msg4-branch"))
        .unwrap();

    let ctx = session.build_session_context();
    assert_eq!(ctx.messages.len(), 3); // msg1, msg2, msg4-branch (not msg3)

    assert_eq!(ctx.messages[0]["content"], "msg1");
    assert_eq!(ctx.messages[1]["content"][0]["text"], "msg2");
    assert_eq!(ctx.messages[2]["content"][0]["text"], "msg4-branch");
}

#[test]
fn create_branched_session_throws_for_nonexistent_entry() {
    let mut session = SessionManager::in_memory();
    session.append_message(user_msg("hello")).unwrap();

    let err = session.create_branched_session("nonexistent").unwrap_err();
    assert_eq!(err.to_string(), "Entry nonexistent not found");
}

#[test]
fn create_branched_session_in_memory() {
    let mut session = SessionManager::in_memory();

    // Build: 1 -> 2 -> 3 -> 4
    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    let id3 = session.append_message(user_msg("3")).unwrap();
    session.append_message(assistant_msg("4")).unwrap();

    // Branch from 3: 3 -> 5
    session.branch(&id3).unwrap();
    session.append_message(user_msg("5")).unwrap();

    // Create branched session from id2 (should only have 1 -> 2)
    let result = session.create_branched_session(&id2).unwrap();
    assert!(result.is_none()); // in-memory returns null

    let entries = session.get_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["id"], id1.as_str());
    assert_eq!(entries[1]["id"], id2.as_str());
}

#[test]
fn create_branched_session_extracts_correct_path() {
    let mut session = SessionManager::in_memory();

    // Build: 1 -> 2 -> 3
    let id1 = session.append_message(user_msg("1")).unwrap();
    let id2 = session.append_message(assistant_msg("2")).unwrap();
    session.append_message(user_msg("3")).unwrap();

    // Branch from 2: 2 -> 4 -> 5
    session.branch(&id2).unwrap();
    let id4 = session.append_message(user_msg("4")).unwrap();
    let id5 = session.append_message(assistant_msg("5")).unwrap();

    // Create branched session from id5 (should have 1 -> 2 -> 4 -> 5)
    session.create_branched_session(&id5).unwrap();

    let entries = session.get_entries();
    let ids: Vec<&str> = entries.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        vec![id1.as_str(), id2.as_str(), id4.as_str(), id5.as_str()]
    );
}

#[test]
fn create_branched_session_defers_file_until_assistant() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();

    // Create a persisted session with a couple of turns
    let mut session = SessionManager::create(&dir, Some(&dir), "", None).unwrap();
    let id1 = session.append_message(user_msg("first question")).unwrap();
    session
        .append_message(assistant_msg("first answer"))
        .unwrap();
    session.append_message(user_msg("second question")).unwrap();
    session
        .append_message(assistant_msg("second answer"))
        .unwrap();

    // Fork from the very first user message (no assistant in the branched path)
    let new_file = session.create_branched_session(&id1).unwrap().unwrap();

    // The branched path has no assistant, so the file should not exist yet
    assert!(!std::path::Path::new(&new_file).exists());

    // Simulate extension adding entry before assistant (like preset on turn_start)
    session
        .append_custom_entry("preset-state", Some(json!({ "name": "plan" })))
        .unwrap();

    // Now the assistant responds
    session.append_message(assistant_msg("new answer")).unwrap();

    // File should now exist with exactly one header and no duplicate IDs
    assert!(std::path::Path::new(&new_file).exists());
    let content = std::fs::read_to_string(&new_file).unwrap();
    let records: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    assert_eq!(records.iter().filter(|r| r["type"] == "session").count(), 1);

    let entry_ids: Vec<&str> = records
        .iter()
        .filter(|r| r["type"] != "session")
        .filter_map(|r| r["id"].as_str())
        .collect();
    let unique: std::collections::HashSet<&str> = entry_ids.iter().copied().collect();
    assert_eq!(unique.len(), entry_ids.len());
}

#[test]
fn create_branched_session_writes_immediately_with_assistant() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();

    let mut session = SessionManager::create(&dir, Some(&dir), "", None).unwrap();
    session.append_message(user_msg("first question")).unwrap();
    let id2 = session
        .append_message(assistant_msg("first answer"))
        .unwrap();
    session.append_message(user_msg("second question")).unwrap();
    session
        .append_message(assistant_msg("second answer"))
        .unwrap();

    // Fork including the assistant message
    let new_file = session.create_branched_session(&id2).unwrap().unwrap();

    // Path includes an assistant, so file should be written immediately
    assert!(std::path::Path::new(&new_file).exists());
    let content = std::fs::read_to_string(&new_file).unwrap();
    let records: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.iter().filter(|r| r["type"] == "session").count(), 1);
}
