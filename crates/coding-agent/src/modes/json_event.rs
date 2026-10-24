//! `modes/json-event.ts` for the durable harness's events: the cumulative assistant snapshots are
//! never on the wire (durable streams deltas and a whole message only as a fallback), usage rides
//! on every `message_update`, and `toolcall_start` carries the call's id and tool name.
//!
//! The TypeScript session's `agent_end` and `turn_end` carry the messages the harness already
//! streamed, so the mapper keeps them as it goes. Events pi-rs has no TypeScript counterpart for
//! keep the durable name and fields.

use pi_durable::ai::types::{AssistantBlock, Message, StopReason};
use pi_durable::harness::events::{AgentEvent, MessageChange};
use serde_json::{Value, json};

use super::value;

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
    /// turn, so they are held until `turn_start`.
    held: Vec<Value>,
}

fn message_of(entry: &pi_durable::types::EntryRecord) -> Option<Message> {
    entry
        .model
        .as_ref()
        .and_then(|model| model.first())
        .cloned()
}

impl Mapper {
    /// The event as JSON lines: one per assistant delta, one for everything else.
    pub fn lines(&mut self, event: &AgentEvent) -> Vec<Value> {
        match event {
            // The current state, not a change; clients start from the commands they send.
            AgentEvent::Snapshot(_) => Vec::new(),

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
                let mut lines = std::mem::take(&mut self.held);
                lines.extend([
                    json!({
                        "type": "agent_end",
                        "messages": self.messages.iter().map(value).collect::<Vec<Value>>(),
                        "willRetry": false,
                    }),
                    json!({ "type": "agent_settled", "aborted": self.aborted }),
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
                if !self.results.is_empty() {
                    line["toolResults"] = self.results.iter().map(value).collect();
                }
                vec![line]
            }

            AgentEvent::MessageStart { message } => {
                let line = json!({ "type": "message_start", "message": value(message) });
                if matches!(message, Message::Assistant(_)) {
                    let mut lines = std::mem::take(&mut self.held);
                    lines.push(line);
                    lines
                } else if self.running {
                    vec![line]
                } else {
                    self.held.push(line);
                    Vec::new()
                }
            }
            AgentEvent::MessageUpdate { usage, changes } => changes
                .iter()
                .map(|change| {
                    json!({
                        "type": "message_update",
                        "usage": value(usage),
                        "assistantMessageEvent": assistant_event(change),
                    })
                })
                .collect(),
            AgentEvent::MessageEnd { entry } => {
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
                self.messages.push(message.clone());
                let line = json!({ "type": "message_end", "message": value(&message) });
                if self.running || matches!(message, Message::Assistant(_)) {
                    vec![line]
                } else {
                    self.held.push(line);
                    Vec::new()
                }
            }

            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => vec![json!({
                "type": "tool_execution_start",
                "toolCallId": tool_call_id,
                "toolName": tool_name,
                "args": args,
            })],
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                output,
                details,
                diagnostics,
            } => {
                let mut line = json!({
                    "type": "tool_execution_update",
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                });
                if let Some(output) = output {
                    line["output"] = value(output);
                }
                if let Some(details) = details {
                    line["details"] = details.clone();
                }
                if let Some(diagnostics) = diagnostics {
                    line["diagnostics"] = value(diagnostics);
                }
                vec![line]
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                entry,
            } => {
                let mut line = json!({
                    "type": "tool_execution_end",
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                });
                if let Some(entry) = entry {
                    if let Some(Message::ToolResult(result)) = message_of(entry) {
                        let mut result_line = json!({ "content": value(&result.content) });
                        if let Some(details) = &result.details {
                            result_line["details"] = details.clone();
                        }
                        line["result"] = result_line;
                        line["isError"] = json!(result.is_error);
                    }
                    line["entry"] = value(entry);
                }
                vec![line]
            }

            AgentEvent::InboxUpdate { items } => vec![json!({
                "type": "inbox_update",
                "items": value(items),
            })],
            AgentEvent::Submission { record } => vec![json!({
                "type": "submission",
                "record": value(record),
            })],
            AgentEvent::EntryAppended { entry } => vec![json!({
                "type": "entry_appended",
                "entry": value(entry),
            })],
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
fn assistant_event(change: &MessageChange) -> Value {
    match change {
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
        // The fallback change: the whole message, as the stream's last word on it.
        MessageChange::Message { message } => json!({
            "type": "done",
            "reason": value(&message.stop_reason),
            "message": value(message),
        }),
    }
}
