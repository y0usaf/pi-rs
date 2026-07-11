//! Port of `test/session-manager/build-context.test.ts`.
#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};

use pi_rs_session::{Leaf, SessionModel, build_session_context};

fn msg(id: &str, parent_id: Option<&str>, role: &str, text: &str) -> Value {
    let parent = parent_id.map_or(Value::Null, Value::from);
    if role == "user" {
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": "2025-01-01T00:00:00Z",
            "message": { "role": "user", "content": text, "timestamp": 1 },
        })
    } else {
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": "2025-01-01T00:00:00Z",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": text }],
                "api": "anthropic-messages",
                "provider": "anthropic",
                "model": "claude-test",
                "usage": {
                    "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
                    "totalTokens": 2,
                    "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 },
                },
                "stopReason": "stop",
                "timestamp": 1,
            },
        })
    }
}

fn compaction(id: &str, parent_id: Option<&str>, summary: &str, first_kept: &str) -> Value {
    json!({
        "type": "compaction", "id": id, "parentId": parent_id.map_or(Value::Null, Value::from),
        "timestamp": "2025-01-01T00:00:00Z",
        "summary": summary, "firstKeptEntryId": first_kept, "tokensBefore": 1000,
    })
}

fn branch_summary(id: &str, parent_id: Option<&str>, summary: &str, from_id: &str) -> Value {
    json!({
        "type": "branch_summary", "id": id, "parentId": parent_id.map_or(Value::Null, Value::from),
        "timestamp": "2025-01-01T00:00:00Z", "summary": summary, "fromId": from_id,
    })
}

fn thinking_level(id: &str, parent_id: Option<&str>, level: &str) -> Value {
    json!({
        "type": "thinking_level_change", "id": id, "parentId": parent_id.map_or(Value::Null, Value::from),
        "timestamp": "2025-01-01T00:00:00Z", "thinkingLevel": level,
    })
}

fn model_change(id: &str, parent_id: Option<&str>, provider: &str, model_id: &str) -> Value {
    json!({
        "type": "model_change", "id": id, "parentId": parent_id.map_or(Value::Null, Value::from),
        "timestamp": "2025-01-01T00:00:00Z", "provider": provider, "modelId": model_id,
    })
}

#[test]
fn empty_entries_returns_empty_context() {
    let ctx = build_session_context(&[], Leaf::Latest);
    assert!(ctx.messages.is_empty());
    assert_eq!(ctx.thinking_level, "off");
    assert!(ctx.model.is_none());
}

#[test]
fn single_user_message() {
    let entries = vec![msg("1", None, "user", "hello")];
    let ctx = build_session_context(&entries, Leaf::Latest);
    assert_eq!(ctx.messages.len(), 1);
    assert_eq!(ctx.messages[0]["role"], "user");
}

#[test]
fn simple_conversation() {
    let entries = vec![
        msg("1", None, "user", "hello"),
        msg("2", Some("1"), "assistant", "hi there"),
        msg("3", Some("2"), "user", "how are you"),
        msg("4", Some("3"), "assistant", "great"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);
    assert_eq!(ctx.messages.len(), 4);
    let roles: Vec<&str> = ctx
        .messages
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "user", "assistant"]);
}

#[test]
fn tracks_thinking_level_changes() {
    let entries = vec![
        msg("1", None, "user", "hello"),
        thinking_level("2", Some("1"), "high"),
        msg("3", Some("2"), "assistant", "thinking hard"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);
    assert_eq!(ctx.thinking_level, "high");
    assert_eq!(ctx.messages.len(), 2);
}

#[test]
fn tracks_model_from_assistant_message() {
    let entries = vec![
        msg("1", None, "user", "hello"),
        msg("2", Some("1"), "assistant", "hi"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);
    assert_eq!(
        ctx.model,
        Some(SessionModel {
            provider: "anthropic".into(),
            model_id: "claude-test".into()
        })
    );
}

#[test]
fn tracks_model_from_model_change_entry() {
    let entries = vec![
        msg("1", None, "user", "hello"),
        model_change("2", Some("1"), "openai", "gpt-4"),
        msg("3", Some("2"), "assistant", "hi"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);
    // Assistant message overwrites model change
    assert_eq!(
        ctx.model,
        Some(SessionModel {
            provider: "anthropic".into(),
            model_id: "claude-test".into()
        })
    );
}

#[test]
fn compaction_includes_summary_before_kept_messages() {
    let entries = vec![
        msg("1", None, "user", "first"),
        msg("2", Some("1"), "assistant", "response1"),
        msg("3", Some("2"), "user", "second"),
        msg("4", Some("3"), "assistant", "response2"),
        compaction("5", Some("4"), "Summary of first two turns", "3"),
        msg("6", Some("5"), "user", "third"),
        msg("7", Some("6"), "assistant", "response3"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);

    // Should have: summary + kept (3,4) + after (6,7) = 5 messages
    assert_eq!(ctx.messages.len(), 5);
    assert!(
        ctx.messages[0]["summary"]
            .as_str()
            .unwrap()
            .contains("Summary of first two turns")
    );
    assert_eq!(ctx.messages[1]["content"], "second");
    assert_eq!(ctx.messages[2]["content"][0]["text"], "response2");
    assert_eq!(ctx.messages[3]["content"], "third");
    assert_eq!(ctx.messages[4]["content"][0]["text"], "response3");
}

#[test]
fn compaction_keeping_from_first_message() {
    let entries = vec![
        msg("1", None, "user", "first"),
        msg("2", Some("1"), "assistant", "response"),
        compaction("3", Some("2"), "Empty summary", "1"),
        msg("4", Some("3"), "user", "second"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);

    // Summary + all messages (1,2,4)
    assert_eq!(ctx.messages.len(), 4);
    assert!(
        ctx.messages[0]["summary"]
            .as_str()
            .unwrap()
            .contains("Empty summary")
    );
}

#[test]
fn multiple_compactions_uses_latest() {
    let entries = vec![
        msg("1", None, "user", "a"),
        msg("2", Some("1"), "assistant", "b"),
        compaction("3", Some("2"), "First summary", "1"),
        msg("4", Some("3"), "user", "c"),
        msg("5", Some("4"), "assistant", "d"),
        compaction("6", Some("5"), "Second summary", "4"),
        msg("7", Some("6"), "user", "e"),
    ];
    let ctx = build_session_context(&entries, Leaf::Latest);

    // Should use second summary, keep from 4
    assert_eq!(ctx.messages.len(), 4);
    assert!(
        ctx.messages[0]["summary"]
            .as_str()
            .unwrap()
            .contains("Second summary")
    );
}

#[test]
fn follows_path_to_specified_leaf() {
    // Tree:
    //   1 -> 2 -> 3 (branch A)
    //         \-> 4 (branch B)
    let entries = vec![
        msg("1", None, "user", "start"),
        msg("2", Some("1"), "assistant", "response"),
        msg("3", Some("2"), "user", "branch A"),
        msg("4", Some("2"), "user", "branch B"),
    ];

    let ctx_a = build_session_context(&entries, Leaf::Id("3"));
    assert_eq!(ctx_a.messages.len(), 3);
    assert_eq!(ctx_a.messages[2]["content"], "branch A");

    let ctx_b = build_session_context(&entries, Leaf::Id("4"));
    assert_eq!(ctx_b.messages.len(), 3);
    assert_eq!(ctx_b.messages[2]["content"], "branch B");
}

#[test]
fn includes_branch_summary_in_path() {
    let entries = vec![
        msg("1", None, "user", "start"),
        msg("2", Some("1"), "assistant", "response"),
        msg("3", Some("2"), "user", "abandoned path"),
        branch_summary("4", Some("2"), "Summary of abandoned work", "3"),
        msg("5", Some("4"), "user", "new direction"),
    ];
    let ctx = build_session_context(&entries, Leaf::Id("5"));

    assert_eq!(ctx.messages.len(), 4);
    assert!(
        ctx.messages[2]["summary"]
            .as_str()
            .unwrap()
            .contains("Summary of abandoned work")
    );
    assert_eq!(ctx.messages[3]["content"], "new direction");
}

#[test]
fn complex_tree_with_multiple_branches_and_compaction() {
    // Tree:
    //   1 -> 2 -> 3 -> 4 -> compaction(5) -> 6 -> 7 (main path)
    //              \-> 8 -> 9 (abandoned branch)
    //                    \-> branchSummary(10) -> 11 (resumed from 3)
    let entries = vec![
        msg("1", None, "user", "start"),
        msg("2", Some("1"), "assistant", "r1"),
        msg("3", Some("2"), "user", "q2"),
        msg("4", Some("3"), "assistant", "r2"),
        compaction("5", Some("4"), "Compacted history", "3"),
        msg("6", Some("5"), "user", "q3"),
        msg("7", Some("6"), "assistant", "r3"),
        // Abandoned branch from 3
        msg("8", Some("3"), "user", "wrong path"),
        msg("9", Some("8"), "assistant", "wrong response"),
        // Branch summary resuming from 3
        branch_summary("10", Some("3"), "Tried wrong approach", "9"),
        msg("11", Some("10"), "user", "better approach"),
    ];

    // Main path to 7: summary + kept(3,4) + after(6,7)
    let ctx_main = build_session_context(&entries, Leaf::Id("7"));
    assert_eq!(ctx_main.messages.len(), 5);
    assert!(
        ctx_main.messages[0]["summary"]
            .as_str()
            .unwrap()
            .contains("Compacted history")
    );
    assert_eq!(ctx_main.messages[1]["content"], "q2");
    assert_eq!(ctx_main.messages[2]["content"][0]["text"], "r2");
    assert_eq!(ctx_main.messages[3]["content"], "q3");
    assert_eq!(ctx_main.messages[4]["content"][0]["text"], "r3");

    // Branch path to 11: 1,2,3 + branch_summary + 11
    let ctx_branch = build_session_context(&entries, Leaf::Id("11"));
    assert_eq!(ctx_branch.messages.len(), 5);
    assert_eq!(ctx_branch.messages[0]["content"], "start");
    assert_eq!(ctx_branch.messages[1]["content"][0]["text"], "r1");
    assert_eq!(ctx_branch.messages[2]["content"], "q2");
    assert!(
        ctx_branch.messages[3]["summary"]
            .as_str()
            .unwrap()
            .contains("Tried wrong approach")
    );
    assert_eq!(ctx_branch.messages[4]["content"], "better approach");
}

#[test]
fn uses_last_entry_when_leaf_id_not_found() {
    let entries = vec![
        msg("1", None, "user", "hello"),
        msg("2", Some("1"), "assistant", "hi"),
    ];
    let ctx = build_session_context(&entries, Leaf::Id("nonexistent"));
    assert_eq!(ctx.messages.len(), 2);
}

#[test]
fn explicit_null_leaf_returns_no_messages() {
    let entries = vec![msg("1", None, "user", "hello")];
    let ctx = build_session_context(&entries, Leaf::None);
    assert!(ctx.messages.is_empty());
    assert_eq!(ctx.thinking_level, "off");
}

#[test]
fn handles_orphaned_entries_gracefully() {
    let entries = vec![
        msg("1", None, "user", "hello"),
        msg("2", Some("missing"), "assistant", "orphan"), // parent doesn't exist
    ];
    let ctx = build_session_context(&entries, Leaf::Id("2"));
    // Should only get the orphan since parent chain is broken
    assert_eq!(ctx.messages.len(), 1);
}
