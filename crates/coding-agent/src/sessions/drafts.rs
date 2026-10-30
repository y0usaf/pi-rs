use std::sync::Arc;

use pi_durable::ai::types::{
    Message, TextContent, TextType, UserBlock, UserContent, UserMessage, UserRole,
};
use pi_durable::cancel::CancelToken;
use pi_durable::entries::{CompactionData, compaction_entry, context_edit_entry};
use pi_durable::harness::compaction::{SUMMARY_PREFIX, SUMMARY_SUFFIX};
use pi_durable::harness::harness::Conversation;
use pi_durable::ids::EntryId;
use pi_durable::types::{
    CompactionReason, ContextAction, ContextEdit, EntryDraft, EntryHead, EntryQuery,
};
use serde_json::{Map, Value, json};

pub const MESSAGE_KIND: &str = "pi.custom-message";
pub const CUSTOM_KIND: &str = "pi.custom";
pub const LABEL_KIND: &str = "pi.label";

fn text_block(text: &str) -> UserBlock {
    UserBlock::Text(TextContent {
        kind: TextType,
        text: text.to_owned(),
        text_signature: None,
    })
}

fn user(content: UserContent, now: u64) -> Message {
    Message::User(Arc::new(UserMessage {
        role: UserRole,
        content,
        timestamp: now,
    }))
}

pub fn content_of(content: &Value) -> UserContent {
    match content {
        Value::String(text) => UserContent::Blocks(vec![text_block(text)]),
        other => serde_json::from_value(other.clone())
            .unwrap_or_else(|_| UserContent::Blocks(Vec::new())),
    }
}
pub fn message(draft: &Value, now: u64) -> EntryDraft {
    let content = draft
        .get("content")
        .filter(|content| !content.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let mut data = Map::new();
    data.insert(
        "customType".into(),
        draft.get("customType").cloned().unwrap_or(Value::Null),
    );
    data.insert("content".into(), content.clone());
    for key in ["display", "details"] {
        if let Some(value) = draft.get(key) {
            data.insert(key.into(), value.clone());
        }
    }
    EntryDraft {
        kind: MESSAGE_KIND.to_owned(),
        model: Some(vec![user(content_of(&content), now)]),
        data: Some(Value::Object(data)),
        head: None,
        edits: None,
    }
}

pub(crate) fn entry_id(value: Option<&Value>) -> Result<EntryId, String> {
    let text = value.and_then(Value::as_str).unwrap_or_default();
    u64::from_str_radix(text, 16)
        .ok()
        .filter(|id| *id < 0xffff_fffe)
        .map(EntryId)
        .ok_or_else(|| format!("no stored entry has the id {text:?}"))
}

fn replaced(
    root: &Conversation,
    target: EntryId,
    replacement: &Value,
    cancel: &CancelToken,
) -> Result<Message, String> {
    let query = EntryQuery {
        conversation_id: root.id(),
        min_entry_id: Some(target),
        max_entry_id: Some(target),
        order: None,
    };
    let page = root
        .entries(query, 1, None, cancel)
        .map_err(|e| e.to_string())?;
    let message = page
        .items
        .into_iter()
        .next()
        .and_then(|record| record.model)
        .and_then(|mut messages| messages.pop())
        .ok_or_else(|| format!("entry {:08x} holds no message to replace", target.0))?;
    let mut value = serde_json::to_value(message).map_err(|e| e.to_string())?;
    value["content"] = replacement.get("content").cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| format!("the replacement does not fit: {e}"))
}

fn boundary(
    root: &Conversation,
    draft: &Value,
    now: u64,
    cancel: &CancelToken,
) -> Result<EntryDraft, String> {
    match draft.get("type").and_then(Value::as_str) {
        Some("custom") => {
            let mut data = json!({
                "customType": draft.get("customType").cloned().unwrap_or(Value::Null),
                "timestamp": super::iso8601(now),
            });
            if let Some(value) = draft.get("data") {
                data["data"] = value.clone();
            }
            Ok(EntryDraft {
                kind: CUSTOM_KIND.to_owned(),
                model: None,
                data: Some(data),
                head: None,
                edits: None,
            })
        }
        Some("custom_message") => Ok(message(draft, now)),
        Some("label") => Ok(EntryDraft {
            kind: LABEL_KIND.to_owned(),
            model: None,
            data: Some(json!({
                "targetId": draft.get("targetId").cloned().unwrap_or(Value::Null),
                "label": draft.get("label").cloned().unwrap_or(Value::Null),
                "timestamp": super::iso8601(now),
            })),
            head: None,
            edits: None,
        }),
        Some("context_edit") => {
            let target = entry_id(draft.get("targetId"))?;
            let action = match draft.get("replacement") {
                None | Some(Value::Null) => ContextAction::Omit,
                Some(replacement) => ContextAction::Replace {
                    messages: vec![replaced(root, target, replacement, cancel)?],
                },
            };
            Ok(EntryDraft {
                kind: context_edit_entry().kind().to_owned(),
                model: None,
                data: None,
                head: None,
                edits: Some(vec![ContextEdit { target, action }]),
            })
        }
        Some("compaction") => {
            let first = entry_id(draft.get("firstKeptEntryId"))?;
            let summary = draft
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut data = serde_json::to_value(CompactionData {
                reason: CompactionReason::Manual,
            })
            .map_err(|e| e.to_string())?;
            for key in ["tokensBefore", "details", "usage"] {
                if let Some(value) = draft.get(key) {
                    data[key] = value.clone();
                }
            }
            data["fromHook"] = true.into();
            Ok(EntryDraft {
                kind: compaction_entry().kind().to_owned(),
                model: Some(vec![user(
                    UserContent::Blocks(vec![text_block(&format!(
                        "{SUMMARY_PREFIX}{summary}{SUMMARY_SUFFIX}"
                    ))]),
                    now,
                )]),
                data: Some(data),
                head: Some(EntryHead::Entry(first)),
                edits: None,
            })
        }
        other => Err(format!(
            "a boundary entry of type {} is not one upstream writes",
            other.unwrap_or("(none)")
        )),
    }
}

pub fn append_entries(
    root: &Conversation,
    entries: Vec<EntryDraft>,
    cancel: &CancelToken,
) -> Result<Vec<EntryId>, String> {
    let id = root.id();
    root.commit(
        move |tx| {
            let mut appended = Vec::new();
            for entry in entries {
                appended.push(tx.append_entry(id, entry)?.id);
            }
            Ok(appended)
        },
        cancel,
    )
    .map_err(|e| e.to_string())
}
pub fn append(
    root: &Conversation,
    drafts: &[Value],
    cancel: &CancelToken,
) -> Result<Vec<EntryId>, String> {
    let now = super::export::now_ms();
    let entries = drafts
        .iter()
        .map(|draft| boundary(root, draft, now, cancel))
        .collect::<Result<Vec<_>, _>>()?;
    append_entries(root, entries, cancel)
}
