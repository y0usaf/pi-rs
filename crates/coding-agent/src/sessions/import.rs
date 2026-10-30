//! Upstream's session JSONL into a durable session (`importFromJsonl`): a new `.db` in the
//! session directory holding the file's entries in order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use pi_durable::ai::types::{
    AssistantMessage, Message, SystemMessage, TextContent, TextType, ToolResultMessage, UserBlock,
    UserContent, UserMessage,
};
use pi_durable::cancel::CancelToken;
use pi_durable::entries::{
    assistant_entry, compaction_entry, context_edit_entry, system_entry, tool_result_entry,
    user_entry,
};
use pi_durable::errors::Result as DurableResult;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::compaction::{SUMMARY_PREFIX, SUMMARY_SUFFIX};
use pi_durable::harness::types::ModelRef;
use pi_durable::harness::usage::{UsageBucket, record_usage};
use pi_durable::ids::{ConversationId, EntryId};
use pi_durable::session::kernel::create_session;
use pi_durable::session::types::Tx;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::{ContextAction, ContextEdit, EntryDraft, EntryHead};
use serde_json::{Value, json};

use super::from_iso8601;
use crate::paths::{PathInput, join, resolve_path};

/// What an import produced, and which entry types a durable session cannot keep.
pub struct Imported {
    pub path: String,
    /// The cwd the file records, empty when it records none.
    pub cwd: String,
    pub dropped: BTreeMap<String, usize>,
}

/// `importFromJsonl`: read `input` into a new session file in `session_dir`.
pub fn run(
    input: &str,
    session_dir: &str,
    cwd: &str,
    cancel: &CancelToken,
) -> std::result::Result<Imported, String> {
    let path = resolve_path(input, cwd, PathInput::default());
    let text = std::fs::read_to_string(&path).map_err(|_| format!("File not found: {path}"))?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header: Value = match lines.next() {
        Some(line) => {
            serde_json::from_str(line).map_err(|e| format!("Not a session file: {path}: {e}"))?
        }
        None => return Err(format!("Not a session file: {path}")),
    };
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return Err(format!("Not a session file: {path}"));
    }
    let stored = header
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if !stored.is_empty() && !Path::new(&stored).exists() {
        return Err(format!(
            "Stored session working directory does not exist: {stored}\nSession file: {path}\nCurrent working directory: {cwd}"
        ));
    }
    let entries: Vec<Value> = lines
        .map(|line| {
            serde_json::from_str(line).map_err(|e| format!("Unreadable entry in {path}: {e}"))
        })
        .collect::<std::result::Result<_, _>>()?;
    std::fs::create_dir_all(session_dir).map_err(|e| e.to_string())?;
    // The file keeps the session id its header declares, as upstream's copy does, and a suffix
    // when that name is taken.
    let id = header
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| name_is_safe(id))
        .map(str::to_owned)
        .map_or_else(
            || pi_durable::ai::utils::uuid::uuidv7(None).map_err(|e| format!("Session id: {e}")),
            Ok,
        )?;
    let mut destination = join(session_dir, &format!("{id}.db"));
    let mut suffix = 1;
    while Path::new(&destination).exists() {
        destination = join(session_dir, &format!("{id}-{suffix}.db"));
        suffix += 1;
    }
    let storage =
        SqliteStorage::open(&destination, &Default::default()).map_err(|e| e.to_string())?;
    let session = create_session(storage, None);
    let mut dropped = BTreeMap::new();
    let outcome = session.commit(
        |tx| {
            let conversation = tx.create_root_conversation()?.id;
            write(tx, conversation, &entries, &stored, &mut dropped)
        },
        cancel,
    );
    let closed = session.close(cancel);
    outcome.map_err(|e| e.to_string())?;
    closed.map_err(|e| e.to_string())?;
    Ok(Imported {
        path: destination,
        cwd: stored,
        dropped,
    })
}

fn write(
    tx: &mut dyn Tx,
    conversation: ConversationId,
    entries: &[Value],
    cwd: &str,
    dropped: &mut BTreeMap<String, usize>,
) -> DurableResult<()> {
    let mut ids: BTreeMap<String, EntryId> = BTreeMap::new();
    let mut written: BTreeMap<String, Message> = BTreeMap::new();
    let mut customs: BTreeSet<String> = BTreeSet::new();
    let mut prompts: Vec<EntryId> = Vec::new();
    let mut model: Option<ModelRef> = None;
    let mut thinking = None;
    for entry in entries {
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let source = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        let stamp = entry.get("timestamp").and_then(Value::as_str);
        let stamped: Option<u64> = stamp
            .and_then(from_iso8601)
            .and_then(|time| time.try_into().ok());
        let time = stamped.unwrap_or_default();
        let id = match kind {
            "message" => {
                let Some(raw) = entry.get("message") else {
                    *dropped.entry(kind.to_owned()).or_default() += 1;
                    continue;
                };
                if raw.get("role").and_then(Value::as_str) == Some("bashExecution") {
                    let draft = crate::bash_exec::entry(
                        raw.get("command")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        &bash_outcome(raw),
                        raw.get("excludeFromContext")
                            .and_then(Value::as_bool)
                            .unwrap_or_default(),
                    );
                    tx.append_entry(
                        conversation,
                        EntryDraft {
                            kind: draft.kind,
                            model: draft.model,
                            data: draft.data,
                            head: None,
                            edits: None,
                        },
                    )?
                    .id
                } else {
                    let message: Message = serde_json::from_value(raw.clone())?;
                    let draft = EntryDraft {
                        kind: durable_kind(&message),
                        data: matches!(message, Message::ToolResult(_))
                            .then(|| json!({ "diagnostics": [] })),
                        model: Some(vec![message.clone()]),
                        head: None,
                        edits: None,
                    };
                    record(&mut *tx, conversation, &message)?;
                    let prompt = matches!(message, Message::System(_));
                    written.insert(source.to_owned(), message);
                    let id = tx.append_entry(conversation, draft)?.id;
                    if prompt {
                        prompts.push(id);
                    }
                    id
                }
            }
            "compaction" => {
                let summary = entry
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let head = entry
                    .get("firstKeptEntryId")
                    .and_then(Value::as_str)
                    .and_then(|first| ids.get(first).copied());
                let message = Message::User(Arc::new(UserMessage {
                    role: pi_durable::ai::types::UserRole,
                    content: UserContent::Blocks(vec![UserBlock::Text(TextContent {
                        kind: TextType,
                        text: format!("{SUMMARY_PREFIX}{summary}{SUMMARY_SUFFIX}"),
                        text_signature: None,
                    })]),
                    timestamp: time,
                }));
                let mut data = json!({ "reason": "manual" });
                for key in ["tokensBefore", "details", "usage", "fromHook"] {
                    if let Some(value) = entry.get(key) {
                        data[key] = value.clone();
                    }
                }
                let draft = EntryDraft {
                    kind: compaction_entry().kind().to_owned(),
                    model: Some(vec![message]),
                    data: Some(data),
                    head: head.map(EntryHead::Entry),
                    edits: None,
                };
                let id = tx.append_entry(conversation, draft)?.id;
                if let Some(raw) = entry.get("systemMessage") {
                    let message: Message = serde_json::from_value(raw.clone())?;
                    let edits: Vec<ContextEdit> = prompts
                        .iter()
                        .filter(|prompt| head.is_some_and(|head| **prompt >= head))
                        .map(|&target| ContextEdit {
                            target,
                            action: ContextAction::Omit,
                        })
                        .collect();
                    let restated = EntryDraft {
                        kind: durable_kind(&message),
                        model: Some(vec![message]),
                        data: None,
                        head: None,
                        edits: (!edits.is_empty()).then_some(edits),
                    };
                    prompts.push(tx.append_entry(conversation, restated)?.id);
                }
                id
            }
            "context_edit" => {
                let mentioned = entry.get("targetId").and_then(Value::as_str);
                let Some(target) = mentioned.and_then(|target| ids.get(target).copied()) else {
                    *dropped.entry(kind.to_owned()).or_default() += 1;
                    continue;
                };
                // A replacement carries only content; the target's own message supplies the rest.
                let custom = mentioned.is_some_and(|target| customs.contains(target));
                let action = match (
                    entry.get("replacement").and_then(|r| r.get("content")),
                    mentioned.and_then(|target| written.get(target)),
                ) {
                    (Some(content), Some(message)) => replace_content(
                        message,
                        &super::drafts::edited_content(message, custom, content),
                    )
                    .map(|message| ContextAction::Replace {
                        messages: vec![message],
                    })
                    .unwrap_or(ContextAction::Omit),
                    _ => ContextAction::Omit,
                };
                let data = match action {
                    ContextAction::Replace { .. } => entry
                        .get("replacement")
                        .map(|replacement| json!({ "replacement": replacement })),
                    ContextAction::Omit => None,
                };
                let draft = EntryDraft {
                    kind: context_edit_entry().kind().to_owned(),
                    model: None,
                    data,
                    head: None,
                    edits: Some(vec![ContextEdit { target, action }]),
                };
                tx.append_entry(conversation, draft)?.id
            }
            "label" => {
                let Some(target) = entry
                    .get("targetId")
                    .and_then(Value::as_str)
                    .and_then(|target| ids.get(target))
                else {
                    *dropped.entry(kind.to_owned()).or_default() += 1;
                    continue;
                };
                let mut data = json!({
                    "targetId": super::export::short_id(target.0),
                    "label": entry.get("label").cloned().unwrap_or(Value::Null),
                });
                if let Some(timestamp) = entry.get("timestamp").filter(|value| value.is_string()) {
                    data["timestamp"] = timestamp.clone();
                }
                let draft = EntryDraft {
                    kind: super::drafts::LABEL_KIND.to_owned(),
                    model: None,
                    data: Some(data),
                    head: None,
                    edits: None,
                };
                tx.append_entry(conversation, draft)?.id
            }
            "session_info" => crate::session_info::record(
                &mut *tx,
                conversation,
                entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &stamp.map_or_else(|| super::iso8601(super::export::now_ms()), str::to_owned),
            )?,
            "custom" => {
                let draft = super::drafts::custom(
                    entry.get("customType").cloned().unwrap_or(Value::Null),
                    entry.get("data").cloned(),
                    stamp.map_or_else(|| super::iso8601(super::export::now_ms()), str::to_owned),
                );
                tx.append_entry(conversation, draft)?.id
            }
            "custom_message" => {
                let now = stamped.unwrap_or_else(super::export::now_ms);
                let draft = super::drafts::message(
                    entry,
                    now,
                    stamp.map_or_else(|| super::iso8601(now), str::to_owned),
                );
                if let Some(message) = draft.model.as_ref().and_then(|model| model.first()) {
                    written.insert(source.to_owned(), message.clone());
                    customs.insert(source.to_owned());
                }
                tx.append_entry(conversation, draft)?.id
            }
            "model_change" => {
                model = Some(ModelRef {
                    provider: entry
                        .get("provider")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    model_id: entry
                        .get("modelId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
                *dropped.entry(kind.to_owned()).or_default() += 1;
                continue;
            }
            "thinking_level_change" => {
                thinking = entry
                    .get("thinkingLevel")
                    .and_then(Value::as_str)
                    .and_then(crate::run_config::parse_level);
                *dropped.entry(kind.to_owned()).or_default() += 1;
                continue;
            }
            _ => {
                *dropped.entry(kind.to_owned()).or_default() += 1;
                continue;
            }
        };
        ids.insert(source.to_owned(), id);
    }
    pi_durable::harness::agent::configure(
        tx,
        conversation,
        &AgentChange {
            model: model.map_or(Change::Keep, Change::Set),
            thinking_level: thinking.map_or(Change::Keep, Change::Set),
            cwd: if cwd.is_empty() {
                Change::Keep
            } else {
                Change::Set(cwd.to_owned())
            },
            ..AgentChange::default()
        },
    )
}

/// The token totals a live run would have recorded, so an imported session shows them.
fn record(tx: &mut dyn Tx, conversation: ConversationId, message: &Message) -> DurableResult<()> {
    match message {
        Message::Assistant(assistant) => record_usage(
            tx,
            conversation,
            UsageBucket::Models,
            &format!("{}/{}", assistant.provider, assistant.model),
            &assistant.usage,
        ),
        Message::ToolResult(result) => match &result.usage {
            Some(usage) => record_usage(
                tx,
                conversation,
                UsageBucket::Tools,
                &result.tool_name,
                usage,
            ),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Whether a session id can name a file: alphanumeric, and `-`, `_` or `.` inside.
fn name_is_safe(id: &str) -> bool {
    let alnum = |b: &u8| b.is_ascii_alphanumeric();
    match (id.as_bytes().first(), id.as_bytes().last()) {
        (Some(first), Some(last)) if alnum(first) && alnum(last) => id
            .as_bytes()
            .iter()
            .all(|b| alnum(b) || matches!(b, b'.' | b'_' | b'-')),
        _ => false,
    }
}

/// The run upstream recorded in a `bashExecution` message.
fn bash_outcome(raw: &Value) -> crate::bash_exec::Outcome {
    crate::bash_exec::Outcome {
        output: raw
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        exit_code: raw
            .get("exitCode")
            .cloned()
            .and_then(|code| serde_json::from_value(code).ok()),
        cancelled: raw
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        truncated: raw
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        full_output_path: raw
            .get("fullOutputPath")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// The durable entry kind an upstream message becomes.
fn durable_kind(message: &Message) -> String {
    match message {
        Message::User(_) => user_entry().kind().to_owned(),
        Message::Assistant(_) => assistant_entry().kind().to_owned(),
        Message::ToolResult(_) => tool_result_entry().kind().to_owned(),
        Message::System(_) => system_entry().kind().to_owned(),
    }
}

/// The target message with the replacement's content, when the content fits its role.
fn replace_content(message: &Message, content: &Value) -> Option<Message> {
    Some(match message {
        Message::User(user) => {
            let mut user: UserMessage = (**user).clone();
            user.content = serde_json::from_value(content.clone()).ok()?;
            Message::User(Arc::new(user))
        }
        Message::Assistant(assistant) => {
            let mut assistant: AssistantMessage = (**assistant).clone();
            assistant.content = serde_json::from_value(content.clone()).ok()?;
            Message::Assistant(Arc::new(assistant))
        }
        Message::ToolResult(result) => {
            let mut result: ToolResultMessage = (**result).clone();
            result.content = serde_json::from_value(content.clone()).ok()?;
            Message::ToolResult(Arc::new(result))
        }
        Message::System(system) => {
            let mut system: SystemMessage = system.clone();
            system.content = serde_json::from_value(content.clone()).ok()?;
            Message::System(system)
        }
    })
}
