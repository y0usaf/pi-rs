use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pi_durable::ai::types::{Message, ToolCall, ToolResultMessage, Usage, UserBlock};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::harness::Conversation;
use pi_durable::harness::tool::{
    BeforeToolDecision, TOOL_HOOK, ToolHooks, ToolRound, ToolRuntime, blocked, tool_result_message,
};
use pi_durable::harness::types::{
    HookApi, HookRegistration, ToolControl, ToolErrorStyle, ToolExecutionResult, ToolGate,
};
use pi_durable::ids::ConversationId;
use pi_durable::types::{EntryRecord, JsonObject};
use pi_extensions::{Call, Peer};
use serde_json::{Value, json};

use super::{Live, lock, now_ms};

#[derive(Default)]
pub(super) struct Round {
    open: bool,
    sequential: bool,
    calls: Vec<ToolCall>,
    tasked: HashSet<String>,
    next: usize,
    told: HashSet<String>,
    started: HashSet<String>,
    ended: HashSet<String>,
    gates: HashMap<String, Gate>,
    inputs: HashMap<String, Value>,
    relayed: HashSet<String>,
}

pub(super) struct Ahead {
    calls: Vec<(ToolCall, ToolGate)>,
    style: ToolErrorStyle,
}

enum Gate {
    Run(JsonObject),
    Block { reason: String, terminate: bool },
}

#[derive(serde::Deserialize)]
struct Called {
    #[serde(default)]
    block: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    terminate: bool,
    #[serde(default)]
    input: Option<JsonObject>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rewritten {
    content: Option<Vec<UserBlock>>,
    details: Option<Value>,
    is_error: Option<bool>,
    usage: Option<Usage>,
}

fn terminates(control: Option<&ToolControl>) -> bool {
    control.is_some_and(|control| control.terminate)
}

fn call_id(record: &EntryRecord) -> Option<&str> {
    match record.model.as_deref()?.first()? {
        Message::ToolResult(result) => Some(&result.tool_call_id),
        _ => None,
    }
}

impl Live {
    pub(super) fn tool_hooks(&self) -> HookRegistration {
        let starting = self.clone();
        let gating = self.clone();
        let updating = self.clone();
        let rewriting = self.clone();
        let ending = self.clone();
        HookRegistration::new(
            &TOOL_HOOK,
            ToolHooks {
                on_start: Some(Arc::new(move |call: &ToolCall, rt: &ToolRuntime<'_>| {
                    if starting.ours(rt.conversation_id()) {
                        starting.go_ahead(Some(call));
                        starting.task_started(call);
                    }
                })),
                before_tool: Some(Arc::new(
                    move |call: &ToolCall, rt: &ToolRuntime<'_>, _cancel: &CancelToken| {
                        if !gating.ours(rt.conversation_id()) {
                            return Ok(None);
                        }
                        gating.go_ahead(Some(call));
                        Ok(Some(gating.gated(call)))
                    },
                )),
                on_update: Some(Arc::new(
                    move |call: &ToolCall, partial: &ToolExecutionResult, rt: &ToolRuntime<'_>| {
                        if updating.ours(rt.conversation_id())
                            && !lock(&updating.0.round).relayed.contains(&call.id)
                        {
                            updating.updated(call, partial);
                        }
                    },
                )),
                after_tool: Some(Arc::new(
                    move |call: &ToolCall,
                          result: ToolExecutionResult,
                          rt: &ToolRuntime<'_>,
                          _cancel: &CancelToken| {
                        if !rewriting.ours(rt.conversation_id()) {
                            return Ok(None);
                        }
                        Ok(rewriting.tool_result(call, result))
                    },
                )),
                on_end: Some(Arc::new(
                    move |call: &ToolCall,
                          message: &ToolResultMessage,
                          control: Option<&ToolControl>,
                          rt: &ToolRuntime<'_>| {
                        if ending.ours(rt.conversation_id())
                            && !lock(&ending.0.round).ended.remove(&call.id)
                        {
                            ending.ended(call, message, terminates(control));
                        }
                    },
                )),
            },
        )
    }

    pub(super) fn tools_ahead(&self, api: &HookApi, round: &ToolRound<'_>) {
        if self.root_of(api).is_none() {
            return;
        }
        *lock(&self.0.round) = Round {
            open: true,
            sequential: round.sequential,
            calls: round.calls.to_vec(),
            tasked: round
                .calls
                .iter()
                .filter(|call| round.offered.contains(&call.name))
                .map(|call| call.id.clone())
                .collect(),
            ..Round::default()
        };
        *lock(&self.0.ahead) = (!round.sequential).then(|| Ahead {
            calls: round
                .calls
                .iter()
                .map(|call| (call.clone(), round.gate(call)))
                .collect(),
            style: round.style,
        });
    }

    fn go_ahead(&self, call: Option<&ToolCall>) {
        if let Some(call) = call
            && !lock(&self.0.round)
                .calls
                .iter()
                .any(|each| each.id == call.id)
        {
            return;
        }
        let mut ahead = pi_durable::turn::pause(|| lock(&self.0.ahead));
        let Some(Ahead { calls, style }) = ahead.take() else {
            return;
        };
        for (call, gate) in calls {
            let call = &call;
            self.started(call);
            let settled = match gate {
                ToolGate::Blocked(result) => *result,
                ToolGate::Run(args) => match self.tool_call(call, args) {
                    Gate::Block { reason, terminate } => {
                        let result = blocked(style, &reason, terminate);
                        lock(&self.0.round)
                            .gates
                            .insert(call.id.clone(), Gate::Block { reason, terminate });
                        result
                    }
                    run => {
                        lock(&self.0.round).gates.insert(call.id.clone(), run);
                        continue;
                    }
                },
            };
            self.ended(
                call,
                &tool_result_message(call, &settled, now_ms(), None),
                terminates(settled.control.as_ref()),
            );
            lock(&self.0.round).ended.insert(call.id.clone());
        }
    }

    pub(super) fn tools_done(&self, root: &Conversation) {
        self.go_ahead(None);
        let (sequential, all) = {
            let round = lock(&self.0.round);
            (round.sequential, round.calls.len())
        };
        if sequential {
            self.catch_up(all);
        }
        self.announce(root);
        self.close_round();
    }

    pub(super) fn close_round(&self) {
        lock(&self.0.round).open = false;
    }

    pub(super) fn unstored(&self, entries: &mut Vec<Value>) {
        let before = entries.len();
        {
            let round = lock(&self.0.round);
            if !round.open {
                return;
            }
            entries.retain(|entry| {
                let message = &entry["message"];
                let Some(id) = message["toolCallId"].as_str() else {
                    return true;
                };
                message["role"] != "toolResult"
                    || round.told.contains(id)
                    || !round.calls.iter().any(|call| call.id == id)
            });
        }
        if entries.len() == before {
            return;
        }
        for at in 1..entries.len() {
            entries[at]["parentId"] = entries[at - 1]["id"].clone();
        }
    }

    pub(super) fn in_call_order(&self, records: &mut Vec<EntryRecord>) {
        let round = lock(&self.0.round);
        records.retain(|record| call_id(record).is_none_or(|id| !round.told.contains(id)));
        let rank = |record: &EntryRecord| {
            call_id(record).and_then(|id| round.calls.iter().position(|call| call.id == id))
        };
        let slots: Vec<usize> = (0..records.len())
            .filter(|&at| rank(&records[at]).is_some())
            .collect();
        let mut ranked: Vec<EntryRecord> = slots.iter().map(|&at| records[at].clone()).collect();
        ranked.sort_by_key(|record| rank(record));
        for (at, record) in slots.into_iter().zip(ranked) {
            records[at] = record;
        }
    }

    fn with_parent(&self, call: &ToolCall, mut event: Value) -> Value {
        let issued = lock(&self.0.round)
            .calls
            .iter()
            .any(|each| each.id == call.id);
        if !issued && let Some((parent, _)) = call.id.rsplit_once('/') {
            event["parentToolCallId"] = json!(parent);
        }
        event
    }

    fn ours(&self, conversation: ConversationId) -> bool {
        self.root().is_some_and(|root| root.id() == conversation)
    }

    fn task_started(&self, call: &ToolCall) {
        let (known, sequential) = {
            let round = lock(&self.0.round);
            (round.started.contains(&call.id), round.sequential)
        };
        if known {
            return;
        }
        if sequential {
            let at = lock(&self.0.round)
                .calls
                .iter()
                .position(|each| each.id == call.id);
            if let Some(at) = at {
                self.catch_up(at);
            }
        }
        self.started(call);
    }

    fn catch_up(&self, until: usize) {
        let Some(root) = self.root() else {
            return;
        };
        loop {
            let (call, tasked) = {
                let mut round = lock(&self.0.round);
                let Some(call) = round.calls.get(round.next).filter(|_| round.next < until) else {
                    return;
                };
                let call = call.clone();
                round.next += 1;
                let tasked = round.tasked.contains(&call.id);
                (call, tasked)
            };
            let Some(record) = self.result_of(&root, &call.id) else {
                continue;
            };
            if !tasked
                && let Some(Message::ToolResult(result)) =
                    record.model.as_deref().and_then(<[Message]>::first)
            {
                self.started(&call);
                self.ended(&call, result, false);
            }
            lock(&self.0.round).told.insert(call.id.clone());
            let thinking = lock(&self.0.thinking).clone().unwrap_or_default();
            match crate::sessions::export::run_messages(&[record], &thinking) {
                Ok(messages) => self.tell(messages),
                Err(error) => self.0.sink.call(Call::Notify {
                    text: format!(
                        "Warning: the extensions were not told of a tool result: {error}"
                    ),
                    level: "warning".to_owned(),
                }),
            }
        }
    }

    fn result_of(&self, root: &Conversation, id: &str) -> Option<EntryRecord> {
        let after = *lock(&self.0.announced);
        crate::sessions::export::entries_after(root, after, &CancelToken::new())
            .ok()?
            .into_iter()
            .find(|record| call_id(record) == Some(id))
    }

    fn started(&self, call: &ToolCall) {
        lock(&self.0.round).started.insert(call.id.clone());
        if self.handles("tool_execution_start") {
            let event = self.with_parent(
                call,
                json!({
                    "toolCallId": call.id,
                    "toolName": call.name,
                    "args": call.arguments,
                }),
            );
            self.event("tool_execution_start", event);
        }
    }

    pub(super) fn relays(
        &self,
        conversation: ConversationId,
        id: &str,
        name: &str,
    ) -> Option<Value> {
        if !self.ours(conversation) {
            return None;
        }
        let mut round = lock(&self.0.round);
        let args = round
            .calls
            .iter()
            .find(|call| call.id == id && call.name == name)
            .map(|call| Value::Object(call.arguments.clone()))?;
        round.relayed.insert(id.to_owned());
        Some(args)
    }

    fn updated(&self, call: &ToolCall, partial: &ToolExecutionResult) {
        if !self.handles("tool_execution_update") {
            return;
        }
        let mut result = json!({ "content": partial.content.clone().unwrap_or_default() });
        if let Some(details) = &partial.details {
            result["details"] = details.clone();
        }
        let event = self.with_parent(
            call,
            json!({
                "toolCallId": call.id,
                "toolName": call.name,
                "args": call.arguments,
                "partialResult": result,
            }),
        );
        self.event("tool_execution_update", event);
    }

    fn ended(&self, call: &ToolCall, message: &ToolResultMessage, terminate: bool) {
        if !self.handles("tool_execution_end") {
            return;
        }
        let mut result = json!({ "content": message.content });
        if let Some(details) = &message.details {
            result["details"] = details.clone();
        }
        if let Some(usage) = &message.usage {
            result["usage"] = json!(usage);
        }
        if terminate {
            result["terminate"] = json!(true);
        }
        let mut event = json!({
            "toolCallId": call.id,
            "toolName": call.name,
            "result": result,
            "isError": message.is_error,
        });
        if let Some(duration) = message.duration_ms {
            event["durationMs"] = json!(duration);
        }
        let event = self.with_parent(call, event);
        self.event("tool_execution_end", event);
    }

    fn gated(&self, call: &ToolCall) -> BeforeToolDecision {
        let decided = lock(&self.0.round).gates.remove(&call.id);
        let gate = decided.unwrap_or_else(|| self.tool_call(call, call.arguments.clone()));
        match gate {
            Gate::Run(input) => {
                lock(&self.0.round)
                    .inputs
                    .insert(call.id.clone(), Value::Object(input.clone()));
                BeforeToolDecision {
                    arguments: Some(input),
                    block: None,
                    terminate: false,
                }
            }
            Gate::Block { reason, terminate } => BeforeToolDecision {
                arguments: None,
                block: Some(reason),
                terminate,
            },
        }
    }

    fn tool_call(&self, call: &ToolCall, args: JsonObject) -> Gate {
        if !self.handles("tool_call") {
            return Gate::Run(args);
        }
        self.refresh_session(&CancelToken::new());
        let event = self.with_parent(
            call,
            json!({
                "toolName": call.name,
                "toolCallId": call.id,
                "input": args,
            }),
        );
        let answer = match self.ask("tool_call", event) {
            Ok(answer) => answer,
            Err(reason) => {
                return Gate::Block {
                    reason,
                    terminate: false,
                };
            }
        };
        match serde_json::from_value::<Called>(answer) {
            Ok(called) if called.block => Gate::Block {
                reason: called.reason,
                terminate: called.terminate,
            },
            Ok(called) => Gate::Run(called.input.unwrap_or(args)),
            Err(e) => Gate::Block {
                reason: format!("the extension host answered tool_call with {e}"),
                terminate: false,
            },
        }
    }

    fn tool_result(
        &self,
        call: &ToolCall,
        result: ToolExecutionResult,
    ) -> Option<ToolExecutionResult> {
        if !self.handles("tool_result") {
            return None;
        }
        self.refresh_session(&CancelToken::new());
        let input = lock(&self.0.round)
            .inputs
            .get(&call.id)
            .cloned()
            .unwrap_or_else(|| Value::Object(call.arguments.clone()));
        let mut event = json!({
            "toolName": call.name,
            "toolCallId": call.id,
            "input": input,
            "content": result.content.clone().unwrap_or_default(),
            "isError": result.is_error.unwrap_or(false),
        });
        if let Some(details) = &result.details {
            event["details"] = details.clone();
        }
        if let Some(usage) = &result.usage {
            event["usage"] = json!(usage);
        }
        let event = self.with_parent(call, event);
        let answer = self.ask("tool_result", event).ok()?;
        let rewritten = match serde_json::from_value::<Option<Rewritten>>(answer) {
            Ok(rewritten) => rewritten?,
            Err(e) => {
                self.0.sink.call(Call::Notify {
                    text: format!(
                        "Warning: the extension host answered tool_result with {e}; the result stands"
                    ),
                    level: "warning".to_owned(),
                });
                return None;
            }
        };
        Some(ToolExecutionResult {
            content: rewritten.content.or(result.content),
            details: rewritten.details.or(result.details),
            is_error: rewritten.is_error.or(result.is_error),
            usage: rewritten.usage.or(result.usage),
            ..result
        })
    }
}
