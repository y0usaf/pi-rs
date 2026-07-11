//! Port of `packages/coding-agent/src/core/messages.ts` — custom message
//! types and transformers. It rides with the session crate because the
//! session tree's `buildSessionContext` mints these messages (locked
//! workspace-layout row: `pi-rs-session` ← `core/session-manager`).
//!
//! Messages are `serde_json::Value` objects shaped exactly like the
//! spec's (camelCase keys, `undefined` fields omitted); the typed
//! `AgentMessage` vocabulary arrives with the WS4 agent port. Timestamps
//! that JS turns into `NaN` (unparsable ISO strings) serialize as `null`,
//! matching `JSON.stringify(NaN)`.

use serde_json::{Map, Value, json};

use crate::time::parse_iso_ms;

pub const COMPACTION_SUMMARY_PREFIX: &str = "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";

pub const COMPACTION_SUMMARY_SUFFIX: &str = "\n</summary>";

pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";

pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";

fn ms_value(timestamp: &str) -> Value {
    parse_iso_ms(timestamp).map_or(Value::Null, Value::from)
}

/// Spec: `bashExecutionToText` — convert a `bashExecution` message to
/// user-message text for LLM context.
pub fn bash_execution_to_text(msg: &Value) -> String {
    let command = msg.get("command").and_then(Value::as_str).unwrap_or("");
    let output = msg.get("output").and_then(Value::as_str).unwrap_or("");
    let mut text = format!("Ran `{command}`\n");
    if !output.is_empty() {
        text.push_str(&format!("```\n{output}\n```"));
    } else {
        text.push_str("(no output)");
    }
    if msg.get("cancelled").and_then(Value::as_bool) == Some(true) {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(code) = msg.get("exitCode").and_then(Value::as_i64)
        && code != 0
    {
        text.push_str(&format!("\n\nCommand exited with code {code}"));
    }
    if msg.get("truncated").and_then(Value::as_bool) == Some(true)
        && let Some(path) = msg.get("fullOutputPath").and_then(Value::as_str)
    {
        text.push_str(&format!("\n\n[Output truncated. Full output: {path}]"));
    }
    text
}

/// Spec: `createBranchSummaryMessage`.
pub fn create_branch_summary_message(summary: &str, from_id: &str, timestamp: &str) -> Value {
    json!({
        "role": "branchSummary",
        "summary": summary,
        "fromId": from_id,
        "timestamp": ms_value(timestamp),
    })
}

/// Spec: `createCompactionSummaryMessage`.
pub fn create_compaction_summary_message(
    summary: &str,
    tokens_before: i64,
    timestamp: &str,
) -> Value {
    json!({
        "role": "compactionSummary",
        "summary": summary,
        "tokensBefore": tokens_before,
        "timestamp": ms_value(timestamp),
    })
}

/// Spec: `createCustomMessage` — convert a `custom_message` entry to a
/// message. `details: undefined` is omitted, matching the spec's
/// serialized shape.
pub fn create_custom_message(
    custom_type: &str,
    content: &Value,
    display: bool,
    details: Option<&Value>,
    timestamp: &str,
) -> Value {
    let mut msg = Map::new();
    msg.insert("role".into(), "custom".into());
    msg.insert("customType".into(), custom_type.into());
    msg.insert("content".into(), content.clone());
    msg.insert("display".into(), display.into());
    if let Some(details) = details {
        msg.insert("details".into(), details.clone());
    }
    msg.insert("timestamp".into(), ms_value(timestamp));
    Value::Object(msg)
}

/// Spec: `convertToLlm` — transform agent messages (including the custom
/// roles above) to LLM-compatible messages. Unknown roles are dropped.
pub fn convert_to_llm(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|m| {
            let timestamp = m.get("timestamp").cloned().unwrap_or(Value::Null);
            match m.get("role").and_then(Value::as_str) {
                Some("bashExecution") => {
                    // Skip messages excluded from context (!! prefix)
                    if m.get("excludeFromContext").and_then(Value::as_bool) == Some(true) {
                        return None;
                    }
                    Some(json!({
                        "role": "user",
                        "content": [{ "type": "text", "text": bash_execution_to_text(m) }],
                        "timestamp": timestamp,
                    }))
                }
                Some("custom") => {
                    let content = match m.get("content") {
                        Some(Value::String(text)) => json!([{ "type": "text", "text": text }]),
                        Some(other) => other.clone(),
                        None => Value::Null,
                    };
                    Some(json!({
                        "role": "user",
                        "content": content,
                        "timestamp": timestamp,
                    }))
                }
                Some("branchSummary") => {
                    let summary = m.get("summary").and_then(Value::as_str).unwrap_or("");
                    Some(json!({
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": format!("{BRANCH_SUMMARY_PREFIX}{summary}{BRANCH_SUMMARY_SUFFIX}"),
                        }],
                        "timestamp": timestamp,
                    }))
                }
                Some("compactionSummary") => {
                    let summary = m.get("summary").and_then(Value::as_str).unwrap_or("");
                    Some(json!({
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": format!("{COMPACTION_SUMMARY_PREFIX}{summary}{COMPACTION_SUMMARY_SUFFIX}"),
                        }],
                        "timestamp": timestamp,
                    }))
                }
                Some("user" | "assistant" | "toolResult") => Some(m.clone()),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_execution_text_shapes() {
        let msg = json!({
            "role": "bashExecution",
            "command": "ls",
            "output": "a\nb",
            "exitCode": 0,
            "cancelled": false,
            "truncated": false,
            "timestamp": 1,
        });
        assert_eq!(bash_execution_to_text(&msg), "Ran `ls`\n```\na\nb\n```");

        let failed = json!({
            "command": "false", "output": "", "exitCode": 1,
            "cancelled": false, "truncated": true, "fullOutputPath": "/tmp/x",
        });
        assert_eq!(
            bash_execution_to_text(&failed),
            "Ran `false`\n(no output)\n\nCommand exited with code 1\n\n[Output truncated. Full output: /tmp/x]"
        );

        let cancelled = json!({
            "command": "sleep 9", "output": "", "cancelled": true, "truncated": false,
        });
        assert_eq!(
            bash_execution_to_text(&cancelled),
            "Ran `sleep 9`\n(no output)\n\n(command cancelled)"
        );
    }

    #[test]
    fn convert_to_llm_roles() {
        let out = convert_to_llm(&[
            json!({ "role": "user", "content": "hi", "timestamp": 1 }),
            json!({ "role": "custom", "customType": "x", "content": "note", "display": true, "timestamp": 2 }),
            json!({ "role": "branchSummary", "summary": "s", "fromId": "1", "timestamp": 3 }),
            json!({ "role": "compactionSummary", "summary": "c", "tokensBefore": 10, "timestamp": 4 }),
            json!({ "role": "bashExecution", "command": "ls", "output": "", "excludeFromContext": true, "timestamp": 5 }),
            json!({ "role": "mystery", "timestamp": 6 }),
        ]);
        assert_eq!(out.len(), 4);
        assert!(
            out.iter()
                .all(|m| m["role"] == "user" || m["role"] == "assistant")
        );
        assert_eq!(out[1]["content"][0]["text"], "note");
        assert!(out[2]["content"][0]["text"].as_str().is_some_and(|t| {
            t.starts_with(BRANCH_SUMMARY_PREFIX) && t.ends_with(BRANCH_SUMMARY_SUFFIX)
        }));
    }

    #[test]
    fn custom_message_omits_missing_details() {
        let msg = create_custom_message(
            "t",
            &json!("hello"),
            false,
            None,
            "2025-01-01T00:00:00.000Z",
        );
        assert!(msg.get("details").is_none());
        assert_eq!(msg["timestamp"], 1_735_689_600_000_i64);

        let bad_ts = create_branch_summary_message("s", "1", "garbage");
        assert_eq!(bad_ts["timestamp"], Value::Null);
    }
}
