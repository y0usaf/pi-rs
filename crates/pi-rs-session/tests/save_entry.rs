//! Port of `test/session-manager/save-entry.test.ts`.
#![allow(clippy::unwrap_used)]

mod common;

use common::{assistant_msg, user_msg};
use pi_rs_session::SessionManager;
use serde_json::json;

#[test]
fn saves_custom_entries_and_includes_them_in_tree_traversal() {
    let mut session = SessionManager::in_memory();

    // Save a message
    let msg_id = session.append_message(user_msg("hello")).unwrap();

    // Save a custom entry
    let custom_id = session
        .append_custom_entry("my_data", Some(json!({ "foo": "bar" })))
        .unwrap();

    // Save another message
    let msg2_id = session.append_message(assistant_msg("hi")).unwrap();

    // Custom entry should be in entries
    let entries = session.get_entries();
    assert_eq!(entries.len(), 3);

    let custom = entries.iter().find(|e| e["type"] == "custom").unwrap();
    assert_eq!(custom["customType"], "my_data");
    assert_eq!(custom["data"], json!({ "foo": "bar" }));
    assert_eq!(custom["id"], custom_id.as_str());
    assert_eq!(custom["parentId"], msg_id.as_str());

    // Tree structure should be correct
    let path = session.get_branch(None);
    assert_eq!(path.len(), 3);
    assert_eq!(path[0]["id"], msg_id.as_str());
    assert_eq!(path[1]["id"], custom_id.as_str());
    assert_eq!(path[2]["id"], msg2_id.as_str());

    // buildSessionContext should work (custom entries skipped in messages)
    let ctx = session.build_session_context();
    assert_eq!(ctx.messages.len(), 2); // only message entries
}
