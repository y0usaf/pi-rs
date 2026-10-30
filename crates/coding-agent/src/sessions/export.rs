//! Upstream's session JSONL out of a durable conversation (`core/session-export.ts`): the
//! current branch as the file `/export <path>.jsonl` writes.

use pi_durable::ai::types::{Message, TextType, UserBlock, UserContent};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::compaction::{SUMMARY_PREFIX, SUMMARY_SUFFIX};
use pi_durable::harness::harness::Conversation;
use pi_durable::ids::EntryId;
use pi_durable::types::{ContextAction, EntryQuery, EntryRecord, ScanOrder};
use serde_json::{Map, Value};

use crate::paths::{PathInput, dirname, resolve_path};

/// `CURRENT_SESSION_VERSION`: the header version every reader understands.
const VERSION: u64 = 3;

const MODEL_CHANGE_ID: u64 = 0xffff_ffff;
const THINKING_CHANGE_ID: u64 = 0xffff_fffe;

/// `getPathCommandArgument`: the first argument after `/export` or `/import`, unquoted.
pub fn path_argument(text: &str, command: &str) -> Option<String> {
    let rest = text.strip_prefix(command)?.strip_prefix(' ')?.trim_start();
    let first = rest.chars().next()?;
    if first == '"' || first == '\'' {
        let closing = rest[1..].find(first)? + 1;
        return Some(rest[1..closing].to_owned());
    }
    Some(match rest.find(char::is_whitespace) {
        Some(end) => rest[..end].to_owned(),
        None => rest.to_owned(),
    })
}

/// `serializeSessionBranch`: the header, then the branch entries re-chained.
pub fn jsonl(
    root: &Conversation,
    cancel: &CancelToken,
    session_id: &str,
    cwd: &str,
) -> Result<String, String> {
    let mut out = String::new();
    for line in &lines(root, cancel, session_id, cwd)? {
        out.push_str(&serde_json::to_string(line).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    Ok(out)
}

pub fn lines(
    root: &Conversation,
    cancel: &CancelToken,
    session_id: &str,
    cwd: &str,
) -> Result<Vec<Value>, String> {
    let agent = root.agent(cancel).map_err(|e| e.to_string())?;
    let view = root.context(None, cancel).map_err(|e| e.to_string())?;
    let now = now_ms();
    let timestamp = iso8601(now);
    let mut lines: Vec<Value> = vec![header(session_id, cwd, &timestamp)];
    let mut parent: Option<String> = None;
    let mut push = |entry: Map<String, Value>, lines: &mut Vec<Value>| {
        let mut entry = entry;
        entry.insert(
            "parentId".into(),
            match &parent {
                Some(id) => id.as_str().into(),
                None => Value::Null,
            },
        );
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        lines.push(Value::Object(entry));
        parent = Some(id);
    };
    // Durable records no model or thinking change: both live in the agent document, so the
    // branch carries the conversation's current pair at its head, where upstream writes them
    // for a new session.
    if let Some(model) = &agent.model {
        let mut change = plain("model_change", &short_id(MODEL_CHANGE_ID), &timestamp);
        change.insert("provider".into(), model.provider.as_str().into());
        change.insert("modelId".into(), model.model_id.as_str().into());
        push(change, &mut lines);
    }
    let phrase = crate::run_config::level_name(agent.thinking_level);
    let mut change = plain(
        "thinking_level_change",
        &short_id(THINKING_CHANGE_ID),
        &timestamp,
    );
    change.insert("thinkingLevel".into(), phrase.as_str().into());
    push(change, &mut lines);
    let head = view
        .head
        .as_ref()
        .filter(|head| head.kind == "pi.compaction")
        .map(|head| head.id);
    let restated = head.and_then(|head| {
        view.entries
            .iter()
            .filter(|entry| entry.kind == "pi.system" && entry.id > head)
            .min_by_key(|entry| entry.id)
    });
    // The system prompt is appended when the first run starts, after the message that started
    // it; upstream stores it first and pi-rs leads the transcript with it, so it goes first here.
    let (system, rest): (Vec<_>, Vec<_>) = view
        .entries
        .iter()
        .filter(|entry| restated.is_none_or(|restated| restated.id != entry.id))
        .partition(|entry| entry.kind == "pi.system" && head.is_none_or(|head| entry.id < head));
    for record in system.into_iter().chain(rest) {
        if let Some(mut entry) = mapped(record, &timestamp, &phrase)? {
            if Some(record.id) == head
                && let Some(prompt) = restated
                && let Some(mut message) =
                    mapped(prompt, &timestamp, &phrase)?.and_then(|mut e| e.remove("message"))
            {
                message["timestamp"] = now.into();
                entry.insert("systemMessage".into(), message);
            }
            push(entry, &mut lines);
        }
    }
    Ok(lines)
}

fn header(session_id: &str, cwd: &str, timestamp: &str) -> Value {
    let mut header = Map::new();
    header.insert("type".into(), "session".into());
    header.insert("version".into(), VERSION.into());
    header.insert("id".into(), session_id.into());
    header.insert("timestamp".into(), timestamp.into());
    header.insert("cwd".into(), cwd.into());
    Value::Object(header)
}

fn plain(kind: &str, id: &str, timestamp: &str) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("type".into(), kind.into());
    entry.insert("id".into(), id.into());
    entry.insert("timestamp".into(), timestamp.into());
    entry
}

/// One branch entry as upstream keeps it, or none when it has no upstream type.
fn mapped(
    record: &EntryRecord,
    timestamp: &str,
    thinking: &str,
) -> Result<Option<Map<String, Value>>, String> {
    let mut entry = plain(&record.kind, "", timestamp);
    entry.insert("id".into(), short_id(record.id.0).into());
    match record.kind.as_str() {
        "pi.user" | "pi.assistant" | "pi.tool-result" | "pi.system" => {
            let message = record
                .model
                .as_ref()
                .and_then(|messages| messages.first())
                .ok_or_else(|| format!("Entry {} carries no message", record.id.0))?;
            entry.insert("type".into(), "message".into());
            let mut written = serde_json::to_value(message).map_err(|e| e.to_string())?;
            upstream_message(&mut written, thinking);
            entry.insert("message".into(), written);
        }
        "pi.compaction" => {
            let summary = record
                .model
                .as_ref()
                .and_then(|messages| messages.first())
                .map(summary_text)
                .unwrap_or_default();
            entry.insert("type".into(), "compaction".into());
            entry.insert("summary".into(), summary.into());
            entry.insert(
                "firstKeptEntryId".into(),
                match record.head {
                    Some(head) => short_id(head.0).into(),
                    None => Value::Null,
                },
            );
            let data = record.data.as_ref();
            entry.insert(
                "tokensBefore".into(),
                data.and_then(|data| data.get("tokensBefore"))
                    .cloned()
                    .unwrap_or_else(|| 0.into()),
            );
            for key in ["details", "usage"] {
                if let Some(value) = data.and_then(|data| data.get(key)) {
                    entry.insert(key.into(), value.clone());
                }
            }
            entry.insert(
                "fromHook".into(),
                data.and_then(|data| data.get("fromHook"))
                    .cloned()
                    .unwrap_or_else(|| false.into()),
            );
        }
        "pi.context-edit" => {
            let edit = record
                .edits
                .as_ref()
                .and_then(|edits| edits.first())
                .ok_or_else(|| format!("Entry {} carries no context edit", record.id.0))?;
            entry.insert("type".into(), "context_edit".into());
            entry.insert("targetId".into(), short_id(edit.target.0).into());
            let stored = record
                .data
                .as_ref()
                .and_then(|data| data.get("replacement"));
            entry.insert(
                "replacement".into(),
                match (&edit.action, stored) {
                    (ContextAction::Omit, _) => Value::Null,
                    (ContextAction::Replace { .. }, Some(stored)) => stored.clone(),
                    (ContextAction::Replace { messages }, None) => match messages.first() {
                        Some(message) => {
                            let value = serde_json::to_value(message).map_err(|e| e.to_string())?;
                            let mut replacement = Map::new();
                            replacement.insert(
                                "content".into(),
                                value.get("content").cloned().unwrap_or(Value::Null),
                            );
                            Value::Object(replacement)
                        }
                        None => Value::Null,
                    },
                },
            );
        }
        kind if kind == super::drafts::MESSAGE_KIND => {
            let data = record
                .data
                .as_ref()
                .ok_or_else(|| format!("Entry {} carries no custom message", record.id.0))?;
            entry.insert("type".into(), "custom_message".into());
            for key in ["customType", "content", "display", "details"] {
                if let Some(value) = data.get(key) {
                    entry.insert(key.into(), value.clone());
                }
            }
            if let Some(timestamp) = data.get("timestamp").filter(|value| value.is_string()) {
                entry.insert("timestamp".into(), timestamp.clone());
            }
        }
        kind if kind == super::drafts::CUSTOM_KIND => {
            let data = record.data.clone().unwrap_or(Value::Null);
            entry.insert("type".into(), "custom".into());
            entry.insert(
                "customType".into(),
                data.get("customType").cloned().unwrap_or(Value::Null),
            );
            if let Some(value) = data.get("data") {
                entry.insert("data".into(), value.clone());
            }
            if let Some(timestamp) = data.get("timestamp").filter(|value| value.is_string()) {
                entry.insert("timestamp".into(), timestamp.clone());
            }
        }
        kind if kind == super::drafts::LABEL_KIND => {
            let data = record.data.clone().unwrap_or(Value::Null);
            entry.insert("type".into(), "label".into());
            entry.insert(
                "targetId".into(),
                data.get("targetId").cloned().unwrap_or(Value::Null),
            );
            if let Some(label) = data.get("label").filter(|label| !label.is_null()) {
                entry.insert("label".into(), label.clone());
            }
            if let Some(timestamp) = data.get("timestamp").filter(|value| value.is_string()) {
                entry.insert("timestamp".into(), timestamp.clone());
            }
        }
        kind if kind == crate::session_info::KIND => {
            let data = record.data.clone().unwrap_or(Value::Null);
            entry.insert("type".into(), "session_info".into());
            entry.insert(
                "name".into(),
                data.get("name").cloned().unwrap_or_else(|| "".into()),
            );
            if let Some(timestamp) = data.get("timestamp").filter(|value| value.is_string()) {
                entry.insert("timestamp".into(), timestamp.clone());
            }
        }
        kind if kind == crate::bash_exec::KIND => {
            let data = record
                .data
                .as_ref()
                .ok_or_else(|| format!("Entry {} carries no bash execution", record.id.0))?;
            entry.insert("type".into(), "message".into());
            let mut message = Map::new();
            message.insert("role".into(), "bashExecution".into());
            for key in [
                "command",
                "output",
                "exitCode",
                "cancelled",
                "truncated",
                "fullOutputPath",
                "excludeFromContext",
            ] {
                if let Some(value) = data.get(key).filter(|value| !value.is_null()) {
                    message.insert(key.into(), value.clone());
                }
            }
            message.insert("timestamp".into(), timestamp.into());
            entry.insert("message".into(), Value::Object(message));
        }
        _ => return Ok(None),
    }
    Ok(Some(entry))
}

pub fn tip(record: &EntryRecord) -> Option<String> {
    let kept = record.kind != "pi.system" && mapped(record, "", "").ok().flatten().is_some();
    kept.then(|| short_id(record.id.0))
}

pub fn appended(record: &EntryRecord, parent: Option<&str>) -> Option<Value> {
    let mut entry = mapped(record, &iso8601(now_ms()), "").ok()??;
    entry.insert(
        "parentId".into(),
        parent
            .map(str::to_owned)
            .unwrap_or_else(|| short_id(THINKING_CHANGE_ID))
            .into(),
    );
    Some(Value::Object(entry))
}

pub fn entries_after(
    root: &Conversation,
    after: Option<EntryId>,
    cancel: &CancelToken,
) -> Result<Vec<EntryRecord>, String> {
    let query = EntryQuery {
        conversation_id: root.id(),
        min_entry_id: after.map(|id| EntryId(id.0 + 1)),
        max_entry_id: None,
        order: Some(ScanOrder::Ascending),
    };
    let mut records = Vec::new();
    let mut cursor = None;
    loop {
        let page = root
            .entries(query.clone(), 256, cursor, cancel)
            .map_err(|e| e.to_string())?;
        records.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(records),
        }
    }
}

pub fn run_messages(records: &[EntryRecord], thinking: &str) -> Result<Vec<Value>, String> {
    let timestamp = iso8601(now_ms());
    let mut messages: Vec<Value> = Vec::new();
    let mut inputs = 0;
    for record in records {
        let Some(mut entry) = mapped(record, &timestamp, thinking)? else {
            continue;
        };
        let message = match entry.get("type").and_then(Value::as_str) {
            Some("message") => entry.remove("message").unwrap_or(Value::Null),
            Some("custom_message") => {
                let mut message = Map::new();
                message.insert("role".into(), "custom".into());
                for key in ["customType", "content", "display", "details"] {
                    if let Some(value) = entry.remove(key) {
                        message.insert(key.into(), value);
                    }
                }
                message.insert("timestamp".into(), now_ms().into());
                Value::Object(message)
            }
            _ => continue,
        };
        match message.get("role").and_then(Value::as_str) {
            Some("system") => messages.insert(messages.len() - inputs, message),
            Some("user" | "custom") => {
                inputs += 1;
                messages.push(message);
            }
            _ => {
                inputs = 0;
                messages.push(message);
            }
        }
    }
    Ok(messages)
}

/// A message as upstream writes it: pi-rs's own fields dropped, the thinking level upstream
/// records on an assistant message added, and whole-number costs printed as JavaScript prints
/// them, without the fraction Rust would write.
pub fn upstream_message(message: &mut Value, thinking: &str) {
    let Some(object) = message.as_object_mut() else {
        return;
    };
    if object.get("role").and_then(Value::as_str) == Some("assistant") {
        object
            .entry("thinkingLevel")
            .or_insert_with(|| thinking.into());
    }
    let Some(cost) = object
        .get_mut("usage")
        .and_then(|usage| usage.get_mut("cost"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    cost.remove(PROVIDER_TOTAL);
    for value in cost.values_mut() {
        let whole = value.as_f64().filter(|n| n.fract() == 0.0);
        if let Some(whole) = whole {
            *value = (whole as i64).into();
        }
    }
}

const PROVIDER_TOTAL: &str = "totalFromProvider";

pub fn upstream_usage(value: &mut Value) {
    let usage = if value.get("usage").is_some() {
        &mut value["usage"]
    } else {
        value
    };
    if let Some(cost) = usage.get_mut("cost").and_then(Value::as_object_mut) {
        cost.remove(PROVIDER_TOTAL);
    }
}

/// The summary inside a compaction message, without the wrapper pi-rs adds.
fn summary_text(message: &Message) -> String {
    let written = match message {
        Message::User(user) => match &user.content {
            UserContent::Text(text) => Some(text.clone()),
            UserContent::Blocks(blocks) => blocks.iter().find_map(|block| match block {
                UserBlock::Text(text) if text.kind == TextType => Some(text.text.clone()),
                _ => None,
            }),
        },
        _ => None,
    }
    .unwrap_or_default();
    written
        .strip_prefix(SUMMARY_PREFIX)
        .and_then(|rest| rest.strip_suffix(SUMMARY_SUFFIX))
        .unwrap_or(&written)
        .to_owned()
}

/// The session's id: the session file's name, or a fresh one for a run that keeps no file.
pub fn session_id(path: Option<&str>) -> String {
    path.and_then(|path| path.rsplit('/').next())
        .and_then(|name| name.strip_suffix(".db"))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            pi_durable::ai::utils::uuid::uuidv7(None).unwrap_or_else(|_| now_ms().to_string())
        })
}

/// `exportToJsonl`: write the branch, making the file's directory, and return the path.
pub fn write(
    root: &Conversation,
    cancel: &CancelToken,
    session_id: &str,
    cwd: &str,
    argument: Option<&str>,
) -> Result<String, String> {
    // Upstream writes HTML for every other path, and for none; that export is not ported.
    let Some(argument) = argument.filter(|path| path.ends_with(".jsonl")) else {
        return Err("HTML export is not ported; name a path ending in .jsonl".to_owned());
    };
    let path = resolve_path(argument, cwd, PathInput::default());
    std::fs::create_dir_all(dirname(&path)).map_err(|e| e.to_string())?;
    std::fs::write(&path, jsonl(root, cancel, session_id, cwd)?).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn short_id(id: u64) -> String {
    format!("{id:08x}")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

use super::iso8601;
