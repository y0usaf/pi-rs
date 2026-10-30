//! `modes/json-event.ts` for the durable harness's events: the cumulative assistant snapshots are
//! never on the wire (durable streams deltas and a whole message only as a fallback), usage rides
//! on every `message_update`, and `toolcall_start` carries the call's id and tool name.
//!
//! The TypeScript session's `agent_end` and `turn_end` carry the messages the harness already
//! streamed, so the mapper keeps them as it goes. Events pi-rs has no TypeScript counterpart for
//! keep the durable name and fields.

use std::collections::HashMap;

use pi_durable::ai::types::{AssistantBlock, Message, StopReason, ToolResultMessage};
use pi_durable::entries::{compaction_entry, system_entry};
use pi_durable::harness::events::{AgentEvent, MessageChange, SnapshotEvent};
use pi_durable::harness::tool::{ToolResultData, tool_result_data};
use pi_durable::types::EntryRecord;
use serde_json::{Value, json};

use super::value;
use crate::sessions::export;

#[derive(Default)]
pub struct Mapper {
    /// The messages of the current agent run, oldest first.
    messages: Vec<Message>,
    /// The assistant message of the current turn, once its entry has arrived.
    turn: Option<Message>,
    /// The tool results of the current turn.
    results: Vec<Message>,
    /// Whether the run's last assistant message was aborted.
    aborted: bool,
    /// Whether an agent run is between its start and its end.
    running: bool,
    /// Messages a submission appended before its run started; upstream announces them inside the
    /// turn, so their lines and messages are held until the run claims them.
    held: Vec<Value>,
    held_messages: Vec<Message>,
    started: HashMap<String, Value>,
    tip: Option<String>,
    starting: Option<Value>,
    restating: bool,
}

fn message_of(entry: &pi_durable::types::EntryRecord) -> Option<Message> {
    entry
        .model
        .as_ref()
        .and_then(|model| model.first())
        .cloned()
}

pub(crate) fn tool_result(message: &ToolResultMessage, data: &ToolResultData) -> Value {
    let mut result = json!({ "content": value(&message.content) });
    if let Some(details) = &message.details {
        result["details"] = details.clone();
    }
    if let Some(structured) = &data.structured_content {
        result["structuredContent"] = structured.clone();
    }
    if let Some(is_error) = data.result_is_error {
        result["isError"] = json!(is_error);
    }
    if let Some(usage) = &message.usage {
        result["usage"] = value(usage);
    }
    if data.terminate {
        result["terminate"] = json!(true);
    }
    result
}

impl Mapper {
    pub fn seed(&mut self, snapshot: &SnapshotEvent) {
        self.tip = snapshot.entries.iter().rev().find_map(export::tip);
    }

    fn follow(&mut self, entry: &EntryRecord) {
        if let Some(id) = export::tip(entry) {
            self.tip = Some(id);
        }
    }

    fn call_args(&self, id: &str) -> Option<Value> {
        let Some(Message::Assistant(message)) = &self.turn else {
            return None;
        };
        message.content.iter().find_map(|block| match block {
            AssistantBlock::ToolCall(call) if call.id == id => {
                Some(Value::Object(call.arguments.clone()))
            }
            _ => None,
        })
    }

    fn tool_start(&mut self, id: &str, name: &str, args: Value) -> Value {
        let args = self.call_args(id).unwrap_or(args);
        self.started.insert(id.to_owned(), args.clone());
        json!({
            "type": "tool_execution_start",
            "toolCallId": id,
            "toolName": name,
            "args": args,
        })
    }

    /// The event as JSON lines: one per assistant delta, one for everything else.
    pub fn lines(&mut self, event: &AgentEvent) -> Vec<Value> {
        match event {
            // The current state, not a change; clients start from the commands they send.
            AgentEvent::Snapshot(snapshot) => {
                self.seed(snapshot);
                Vec::new()
            }

            AgentEvent::RunStart { .. } => {
                self.messages.clear();
                self.turn = None;
                self.results.clear();
                self.aborted = false;
                self.running = true;
                vec![json!({ "type": "agent_start" })]
            }
            AgentEvent::RunEnd { .. } => {
                self.running = false;
                self.messages.append(&mut self.held_messages);
                let mut lines = std::mem::take(&mut self.held);
                lines.extend([
                    json!({
                        "type": "agent_end",
                        "messages": self.messages.iter().map(value).collect::<Vec<Value>>(),
                        "willRetry": false,
                    }),
                    if self.aborted {
                        json!({ "type": "agent_settled", "aborted": true })
                    } else {
                        json!({ "type": "agent_settled" })
                    },
                ]);
                lines
            }
            AgentEvent::TurnStart => {
                self.turn = None;
                self.results.clear();
                vec![json!({ "type": "turn_start" })]
            }
            AgentEvent::TurnEnd => {
                let mut line = json!({ "type": "turn_end" });
                if let Some(turn) = &self.turn {
                    line["message"] = value(turn);
                }
                line["toolResults"] = self.results.iter().map(value).collect();
                vec![line]
            }

            AgentEvent::MessageStart { message } => {
                let line = json!({ "type": "message_start", "message": value(message) });
                if matches!(message, Message::Assistant(_)) {
                    self.messages.append(&mut self.held_messages);
                    let mut lines = std::mem::take(&mut self.held);
                    lines.push(line);
                    lines
                } else {
                    self.starting = Some(line);
                    Vec::new()
                }
            }
            AgentEvent::MessageUpdate { usage, changes } => changes
                .iter()
                .filter_map(|change| {
                    Some(json!({
                        "type": "message_update",
                        "usage": value(usage),
                        "assistantMessageEvent": assistant_event(change)?,
                    }))
                })
                .collect(),
            AgentEvent::MessageEnd { entry } => {
                self.follow(entry);
                let start = self.starting.take();
                let compaction = entry.kind == compaction_entry().kind();
                if compaction || (self.restating && entry.kind == system_entry().kind()) {
                    self.restating = compaction;
                    return Vec::new();
                }
                let Some(message) = message_of(entry) else {
                    return Vec::new();
                };
                match &message {
                    Message::Assistant(assistant) => {
                        if matches!(assistant.stop_reason, StopReason::Aborted) {
                            self.aborted = true;
                        }
                        self.turn = Some(message.clone());
                    }
                    Message::ToolResult(_) => self.results.push(message.clone()),
                    _ => {}
                }
                let line = json!({ "type": "message_end", "message": value(&message) });
                let lines: Vec<Value> = start.into_iter().chain([line]).collect();
                if self.running || matches!(message, Message::Assistant(_)) {
                    self.messages.push(message);
                    lines
                } else {
                    self.held.extend(lines);
                    self.held_messages.push(message);
                    Vec::new()
                }
            }

            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => vec![self.tool_start(tool_call_id, tool_name, Value::Object(args.clone()))],
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                partial: Some(partial),
                ..
            } => vec![json!({
                "type": "tool_execution_update",
                "toolCallId": tool_call_id,
                "toolName": tool_name,
                "args": self
                    .started
                    .get(tool_call_id)
                    .cloned()
                    .or_else(|| self.call_args(tool_call_id))
                    .unwrap_or_else(|| json!({})),
                "partialResult": partial,
            })],
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                output,
                details,
                diagnostics,
                partial: None,
            } => {
                let mut partial = serde_json::Map::new();
                if let Some(output) = output {
                    partial.insert("content".into(), value(output));
                }
                if let Some(details) = details {
                    partial.insert("details".into(), details.clone());
                }
                if let Some(diagnostics) = diagnostics {
                    partial.insert("diagnostics".into(), value(diagnostics));
                }
                vec![json!({
                    "type": "tool_execution_update",
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                    "partialResult": partial,
                })]
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                entry,
            } => {
                let mut lines = Vec::new();
                if self.started.remove(tool_call_id).is_none() {
                    lines.push(self.tool_start(tool_call_id, tool_name, json!({})));
                    self.started.remove(tool_call_id);
                }
                let mut line = json!({
                    "type": "tool_execution_end",
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                });
                if let Some(entry) = entry
                    && let Some(Message::ToolResult(result)) = message_of(entry)
                {
                    line["result"] = tool_result(&result, &tool_result_data(entry));
                    line["isError"] = json!(result.is_error);
                    if let Some(duration) = result.duration_ms {
                        line["durationMs"] = json!(duration);
                    }
                }
                lines.push(line);
                lines
            }

            AgentEvent::InboxUpdate { items } => vec![json!({
                "type": "inbox_update",
                "items": value(items),
            })],
            AgentEvent::Submission { record } => vec![json!({
                "type": "submission",
                "record": value(record),
            })],
            AgentEvent::EntryAppended { entry } if entry.kind == crate::session_info::KIND => {
                self.follow(entry);
                let mut line = json!({ "type": "session_info_changed" });
                if let Some(name) = entry
                    .data
                    .as_ref()
                    .and_then(|data| data.get("name"))
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                {
                    line["name"] = name.into();
                }
                vec![line]
            }
            AgentEvent::EntryAppended { entry } => {
                let line = json!({
                    "type": "entry_appended",
                    "entry": export::appended(entry, self.tip.as_deref())
                        .unwrap_or_else(|| value(entry)),
                });
                self.follow(entry);
                vec![line]
            }
            AgentEvent::AgentChanged { agent } => vec![json!({
                "type": "agent_changed",
                "agent": value(agent),
            })],
            AgentEvent::UsageChanged { usage } => vec![json!({
                "type": "usage_changed",
                "usage": value(usage),
            })],
            AgentEvent::TaskFailed {
                task_id,
                kind,
                message,
            } => vec![json!({
                "type": "task_failed",
                "taskId": value(task_id),
                "kind": kind,
                "message": message,
            })],

            AgentEvent::AutoRetryStart {
                attempt,
                at,
                error_message,
            } => vec![json!({
                "type": "auto_retry_start",
                "attempt": attempt,
                "at": at,
                "errorMessage": error_message,
            })],
            AgentEvent::AutoRetryEnd { attempt } => vec![json!({
                "type": "auto_retry_end",
                "attempt": attempt,
            })],
            AgentEvent::DeferredPoll { poll_at } => vec![json!({
                "type": "deferred_poll",
                "pollAt": poll_at,
            })],

            AgentEvent::CompactionStart {
                task_id,
                reason,
                blocking,
            } => vec![json!({
                "type": "compaction_start",
                "taskId": value(task_id),
                "reason": value(reason),
                "blocking": blocking,
            })],
            AgentEvent::CompactionEnd { task_id, reason } => vec![json!({
                "type": "compaction_end",
                "taskId": value(task_id),
                "reason": value(reason),
            })],
        }
    }
}

/// One durable assistant delta as the `AssistantMessageEvent` upstream streams, without the
/// cumulative `partial` upstream's own JSON events strip.
fn assistant_event(change: &MessageChange) -> Option<Value> {
    Some(match change {
        MessageChange::TextStart { content_index, .. } => json!({
            "type": "text_start",
            "contentIndex": content_index,
        }),
        MessageChange::TextDelta {
            content_index,
            delta,
        } => json!({
            "type": "text_delta",
            "contentIndex": content_index,
            "delta": delta,
        }),
        MessageChange::ThinkingStart { content_index, .. } => json!({
            "type": "thinking_start",
            "contentIndex": content_index,
        }),
        MessageChange::ThinkingDelta {
            content_index,
            delta,
        } => json!({
            "type": "thinking_delta",
            "contentIndex": content_index,
            "delta": delta,
        }),
        MessageChange::ToolcallStart {
            content_index,
            block,
        } => match block.as_ref() {
            AssistantBlock::ToolCall(call) => json!({
                "type": "toolcall_start",
                "contentIndex": content_index,
                "id": call.id,
                "toolName": call.name,
            }),
            _ => json!({
                "type": "toolcall_start",
                "contentIndex": content_index,
            }),
        },
        MessageChange::ToolcallDelta {
            content_index,
            delta,
            ..
        } => json!({
            "type": "toolcall_delta",
            "contentIndex": content_index,
            "delta": delta,
        }),
        MessageChange::Block {
            content_index,
            block,
        } => match block.as_ref() {
            AssistantBlock::Text(text) => json!({
                "type": "text_end",
                "contentIndex": content_index,
                "content": text.text,
            }),
            AssistantBlock::Thinking(thinking) => json!({
                "type": "thinking_end",
                "contentIndex": content_index,
                "content": thinking.thinking,
            }),
            AssistantBlock::ToolCall(call) => json!({
                "type": "toolcall_end",
                "contentIndex": content_index,
                "toolCall": value(call),
            }),
        },
        // The whole-message fallback: `message_end` carries that message authoritatively, and
        // upstream's streamed events never carry it.
        MessageChange::Message { .. } => return None,
    })
}
