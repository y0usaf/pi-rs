//! `--mode rpc`: commands as JSON lines on stdin, responses and the session's events as JSON
//! lines on stdout (`modes/rpc/rpc-mode.ts`). Events are the same lines `--mode json` prints;
//! responses answer a command by its `id`. A command whose type pi-rs does not carry answers
//! `"<type> is not implemented"`; a type no mode has answers `"Unknown command: <type>"`, as
//! upstream does.

use std::io::BufRead;
use std::sync::{Arc, Mutex};

use pi_durable::ai::types::{
    AssistantBlock, ImageContent, Message, TextContent, TextType, UserBlock, UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::events::AgentEvent;
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::ModelRef;
use pi_durable::types::{EntryQuery, ScanOrder};
use serde_json::{Value, json};

use super::{Output, json_event, value};
use crate::args::{VALID_THINKING_LEVELS, is_valid_thinking_level};
use crate::models::ModelRegistry;
use crate::print;
use crate::run_config::level_name;

pub struct Options {
    pub model: pi_durable::ai::types::Model,
    pub thinking: pi_durable::ai::types::ModelThinkingLevel,
    pub registry: Arc<ModelRegistry>,
    /// The SQLite file, or `None` under `--no-session`.
    pub session_path: Option<String>,
    pub cwd: String,
    pub auto_compaction: bool,
    /// The commands a `prompt` may invoke, as `get_commands` reports them.
    pub commands: Vec<Value>,
}

/// What the session state is, kept from the events as they arrive.
struct State {
    model: pi_durable::ai::types::Model,
    thinking: pi_durable::ai::types::ModelThinkingLevel,
    streaming: bool,
    compacting: u32,
    pending: usize,
    session_name: Option<String>,
}

impl State {
    fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::RunStart { .. } => self.streaming = true,
            AgentEvent::RunEnd { .. } => self.streaming = false,
            AgentEvent::CompactionStart { .. } => self.compacting += 1,
            AgentEvent::CompactionEnd { .. } => self.compacting = self.compacting.saturating_sub(1),
            AgentEvent::InboxUpdate { items } => self.pending = items.len(),
            _ => {}
        }
    }
}

fn success(id: Option<&Value>, command: &str, data: Option<Value>) -> Value {
    let mut line = response(id, command, true);
    if let Some(data) = data {
        line["data"] = data;
    }
    line
}

fn failure(id: Option<&Value>, command: &str, message: impl Into<String>) -> Value {
    let mut line = response(id, command, false);
    line["error"] = json!(message.into());
    line
}

/// Upstream writes a response's `id` first, then `type`, `command` and `success`.
fn response(id: Option<&Value>, command: &str, ok: bool) -> Value {
    let mut line = serde_json::Map::new();
    if let Some(id) = id {
        line.insert("id".into(), id.clone());
    }
    line.insert("type".into(), json!("response"));
    line.insert("command".into(), json!(command));
    line.insert("success".into(), json!(ok));
    Value::Object(line)
}

fn user_content(message: &Value) -> UserContent {
    let mut blocks: Vec<UserBlock> = Vec::new();
    if let Some(text) = message["message"].as_str() {
        blocks.push(UserBlock::Text(TextContent {
            kind: TextType,
            text: text.to_owned(),
            text_signature: None,
        }));
    }
    for image in message["images"].as_array().into_iter().flatten() {
        if let Ok(image) = serde_json::from_value::<ImageContent>(image.clone()) {
            blocks.push(UserBlock::Image(image));
        }
    }
    UserContent::Blocks(blocks)
}

fn transcript(root: &Conversation, cancel: &CancelToken) -> Result<Vec<Message>, String> {
    print::transcript(root, cancel).map_err(|e| e.to_string())
}

/// The answers `get_commands` gives: what a typed message may invoke.
fn commands(options: &Options) -> Value {
    json!({ "commands": options.commands })
}

pub fn run(
    harness: &Harness,
    root: &Conversation,
    options: Options,
    cancel: &CancelToken,
) -> Result<(), String> {
    let output = Output::default();
    let state = Arc::new(Mutex::new(State {
        model: options.model.clone(),
        thinking: options.thinking,
        streaming: false,
        compacting: 0,
        pending: 0,
        session_name: None,
    }));

    // The conversation's agent starts with no model; the run's settings are the session's.
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: options.model.provider.clone(),
                model_id: options.model.id.clone(),
            }),
            thinking_level: Change::Set(options.thinking),
            cwd: Change::Set(options.cwd.clone()),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;

    let stream = root.events(cancel).map_err(|e| e.to_string())?;
    {
        let printer = output.clone();
        let shared = Arc::clone(&state);
        let mut mapper = json_event::Mapper::default();
        stream
            .start(move |events, _| {
                for event in events {
                    if let Ok(mut state) = shared.lock() {
                        state.apply(event);
                    }
                    for line in mapper.lines(event) {
                        printer.line(&line);
                    }
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
    }

    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin.lock());
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| format!("Error: {e}"))?;
        if read == 0 {
            break;
        }
        let text = line.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            continue;
        }
        let parsed: Value = match serde_json::from_str(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                output.line(&failure(
                    None,
                    "parse",
                    format!("Failed to parse command: {error}"),
                ));
                continue;
            }
        };
        if parsed["type"] == "extension_ui_response" {
            continue;
        }
        if let Some(response) = handle(root, &options, &state, &parsed, cancel) {
            output.line(&response);
        }
    }

    stream.stop();
    let _ = harness.wait_for_idle(cancel);
    Ok(())
}

fn handle(
    root: &Conversation,
    options: &Options,
    state: &Arc<Mutex<State>>,
    command: &Value,
    cancel: &CancelToken,
) -> Option<Value> {
    let id = command.get("id").cloned();
    let id = id.as_ref();
    let kind = command["type"].as_str().unwrap_or_default();
    let response = match kind {
        "prompt" | "steer" | "follow_up" => {
            let when_busy = match kind {
                "steer" => WhenBusy::Steer,
                _ => match command["streamingBehavior"].as_str() {
                    Some("steer") => WhenBusy::Steer,
                    _ => WhenBusy::FollowUp,
                },
            };
            let draft = SubmissionDraft::Input {
                request_id: id.map(|id| id.to_string()),
                content: user_content(command),
                when_busy,
            };
            match root.submit(draft, cancel) {
                Ok(handle) => match handle.status(cancel) {
                    Ok(record) => {
                        let queued = matches!(
                            record.body,
                            pi_durable::types::SubmissionBody::Input(
                                pi_durable::types::InputState::Queued
                            )
                        );
                        let disposition = if queued {
                            "queued"
                        } else if kind == "prompt" {
                            "started"
                        } else {
                            "handled"
                        };
                        success(id, kind, Some(json!({ "disposition": disposition })))
                    }
                    Err(error) => failure(id, kind, error.to_string()),
                },
                Err(error) => failure(id, kind, error.to_string()),
            }
        }

        "abort" => match root.abort(false, cancel) {
            Ok(()) => success(id, kind, None),
            Err(error) => failure(id, kind, error.to_string()),
        },

        "new_session" => match root.reset(None, cancel) {
            Ok(()) => success(id, kind, Some(json!({ "cancelled": false }))),
            Err(error) => failure(id, kind, error.to_string()),
        },

        "get_state" => {
            let mut shared = state.lock().map_err(|_| ()).ok();
            let (model, thinking, streaming, compacting, pending, name, messages) =
                match &mut shared {
                    Some(state) => (
                        Some(value(&state.model)),
                        json!(level_name(state.thinking)),
                        state.streaming,
                        state.compacting > 0,
                        state.pending,
                        state.session_name.clone(),
                        transcript(root, cancel).map(|m| m.len()),
                    ),
                    None => (None, json!("off"), false, false, 0, None, Ok(0)),
                };
            let count = messages.unwrap_or(0);
            success(
                id,
                kind,
                Some(json!({
                    "model": model,
                    "thinkingLevel": thinking,
                    "isStreaming": streaming,
                    "isCompacting": compacting,
                    "steeringMode": "all",
                    "followUpMode": "all",
                    "sessionFile": options.session_path,
                    "sessionId": root.id().0,
                    "sessionName": name,
                    "autoCompactionEnabled": options.auto_compaction,
                    "messageCount": count,
                    "pendingMessageCount": pending,
                })),
            )
        }

        "set_model" => {
            let provider = command["provider"].as_str().unwrap_or_default();
            let model_id = command["modelId"].as_str().unwrap_or_default();
            match options.registry.get_model(provider, model_id) {
                None => failure(id, kind, format!("Model not found: {provider}/{model_id}")),
                Some(model) => {
                    let model = model.clone();
                    let change = AgentChange {
                        model: Change::Set(ModelRef {
                            provider: model.provider.clone(),
                            model_id: model.id.clone(),
                        }),
                        ..AgentChange::default()
                    };
                    match root.configure(&change, cancel) {
                        Err(error) => failure(id, kind, error.to_string()),
                        Ok(()) => {
                            if let Ok(mut state) = state.lock() {
                                state.model = model.clone();
                            }
                            success(id, kind, Some(value(&model)))
                        }
                    }
                }
            }
        }

        "get_available_models" => {
            let models: Vec<Value> = options
                .registry
                .available()
                .into_iter()
                .map(value)
                .collect();
            success(id, kind, Some(json!({ "models": models })))
        }

        "set_thinking_level" => match command["level"].as_str() {
            Some(level) if is_valid_thinking_level(level) => {
                match serde_json::from_value::<pi_durable::ai::types::ModelThinkingLevel>(json!(
                    level
                )) {
                    Err(error) => failure(id, kind, error.to_string()),
                    Ok(level) => {
                        let change = AgentChange {
                            thinking_level: Change::Set(level),
                            ..AgentChange::default()
                        };
                        match root.configure(&change, cancel) {
                            Err(error) => failure(id, kind, error.to_string()),
                            Ok(()) => {
                                if let Ok(mut state) = state.lock() {
                                    state.thinking = level;
                                }
                                success(id, kind, None)
                            }
                        }
                    }
                }
            }
            _ => failure(
                id,
                kind,
                format!("Invalid thinking level: {}", command["level"]),
            ),
        },

        "get_available_thinking_levels" => {
            let reasoning = state
                .lock()
                .map(|state| state.model.reasoning)
                .unwrap_or(false);
            let levels: Vec<&str> = if reasoning {
                VALID_THINKING_LEVELS.to_vec()
            } else {
                vec!["off"]
            };
            success(id, kind, Some(json!({ "levels": levels })))
        }

        "compact" => {
            let instructions = command["customInstructions"].as_str();
            match root.compact(instructions, cancel) {
                Ok(task) => success(id, kind, Some(json!({ "taskId": value(&task) }))),
                Err(error) => failure(id, kind, error.to_string()),
            }
        }

        "get_messages" => match transcript(root, cancel) {
            Ok(messages) => success(id, kind, Some(json!({ "messages": messages }))),
            Err(error) => failure(id, kind, error),
        },

        "get_last_assistant_text" => match transcript(root, cancel) {
            Ok(messages) => {
                let text = messages.iter().rev().find_map(|message| match message {
                    Message::Assistant(assistant) => Some(
                        assistant
                            .content
                            .iter()
                            .filter_map(|block| match block {
                                AssistantBlock::Text(text) => Some(text.text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<String>>()
                            .join(""),
                    ),
                    _ => None,
                });
                match text {
                    Some(text) => success(id, kind, Some(json!({ "text": text }))),
                    None => success(id, kind, Some(json!({}))),
                }
            }
            Err(error) => failure(id, kind, error),
        },

        "get_entries" => {
            let query = EntryQuery {
                conversation_id: root.id(),
                min_entry_id: None,
                max_entry_id: None,
                order: Some(ScanOrder::Ascending),
            };
            match root.entries(query, 10_000, None, cancel) {
                Err(error) => failure(id, kind, error.to_string()),
                Ok(page) => {
                    let mut entries = page.items;
                    let leaf = entries.last().map(|entry| entry.id.0);
                    if let Some(since) = command["since"].as_str() {
                        match since.parse::<u64>() {
                            Err(_) => {
                                return Some(failure(
                                    id,
                                    kind,
                                    format!("Entry not found: {since}"),
                                ));
                            }
                            Ok(since) => match entries.iter().position(|entry| entry.id.0 == since)
                            {
                                None => {
                                    return Some(failure(
                                        id,
                                        kind,
                                        format!("Entry not found: {since}"),
                                    ));
                                }
                                Some(index) => {
                                    entries.drain(..=index);
                                }
                            },
                        }
                    }
                    success(
                        id,
                        kind,
                        Some(json!({ "entries": entries, "leafId": leaf })),
                    )
                }
            }
        }

        "get_commands" => success(id, kind, Some(commands(options))),

        "set_session_name" => {
            let name = command["name"].as_str().unwrap_or_default().trim();
            if name.is_empty() {
                failure(id, kind, "Session name cannot be empty")
            } else {
                if let Ok(mut state) = state.lock() {
                    state.session_name = Some(name.to_owned());
                }
                success(id, kind, None)
            }
        }

        "cycle_model"
        | "cycle_thinking_level"
        | "clear_queue"
        | "switch_session"
        | "fork"
        | "clone"
        | "get_fork_messages"
        | "get_tree"
        | "set_steering_mode"
        | "set_follow_up_mode"
        | "set_auto_compaction"
        | "set_auto_retry"
        | "abort_retry"
        | "bash"
        | "abort_bash"
        | "get_session_stats"
        | "export_html" => failure(
            id,
            kind,
            format!("pi-rs: {kind} is not implemented in RPC mode"),
        ),

        other => failure(id, other, format!("Unknown command: {other}")),
    };
    Some(response)
}
