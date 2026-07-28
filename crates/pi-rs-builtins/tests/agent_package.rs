//! Deterministic acceptance for the shipped Lua agent package.
//!
//! Every scenario drives the ordinary file-backed agent package through the
//! public kernel transaction with a registered fixture provider. No private
//! host API, no product code in Rust: the agent's transition policy is the
//! Lua under `crates/pi-rs-builtins/agent/`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_rs_ai::protocols::ProtocolError;
use pi_rs_ai::registry::{ApiProvider, register_api_provider, unregister_api_providers};
use pi_rs_ai::transport::{AssistantMessageEventStream, create_assistant_message_event_stream};
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Message,
    Model, StopReason, TextContent, ToolCall, ToolCallType, Usage, now_ms,
};
use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Fixture provider
// ---------------------------------------------------------------------------

fn assistant(model: &Model, content: Vec<AssistantContent>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRole::Assistant,
        content,
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_model: None,
        response_id: None,
        diagnostics: None,
        usage: Usage::default(),
        stop_reason: stop,
        error_message: None,
        timestamp: now_ms(),
    }
}

fn text_stream(model: &Model, chunks: &[&str]) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let joined = chunks.concat();
    let partial = assistant(
        model,
        vec![AssistantContent::Text(TextContent::new(&joined))],
        StopReason::Stop,
    );
    stream.push(AssistantMessageEvent::Start {
        partial: partial.clone(),
    });
    for (index, chunk) in chunks.iter().enumerate() {
        stream.push(AssistantMessageEvent::TextDelta {
            content_index: index,
            delta: (*chunk).to_owned(),
            partial: partial.clone(),
        });
    }
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message: partial,
    });
    stream.end();
    stream
}

fn tool_call(id: &str, name: &str, argument: &str) -> ToolCall {
    let mut arguments = serde_json::Map::new();
    arguments.insert("input".to_owned(), Value::String(argument.to_owned()));
    ToolCall {
        r#type: ToolCallType::ToolCall,
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
        thought_signature: None,
    }
}

fn tool_stream(model: &Model, calls: Vec<ToolCall>) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let message = assistant(
        model,
        calls
            .iter()
            .cloned()
            .map(AssistantContent::ToolCall)
            .collect(),
        StopReason::ToolUse,
    );
    stream.push(AssistantMessageEvent::Start {
        partial: message.clone(),
    });
    for (index, call) in calls.into_iter().enumerate() {
        stream.push(AssistantMessageEvent::ToolCallEnd {
            content_index: index,
            tool_call: call,
            partial: message.clone(),
        });
    }
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::ToolUse,
        message,
    });
    stream.end();
    stream
}

fn user_texts(context: &Context) -> Vec<String> {
    context
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::User(user) => Some(format!("{:?}", user.content)),
            _ => None,
        })
        .collect()
}

fn has_tool_results(context: &Context) -> bool {
    context
        .messages
        .iter()
        .any(|message| matches!(message, Message::ToolResult(_)))
}

/// Registers a fixture api family for the duration of one test and removes it
/// on drop, so scenarios stay independent and deterministic.
struct Fixture {
    owner: String,
}

impl Fixture {
    fn install(api: &str, attempts: Arc<AtomicUsize>) -> Self {
        let owner = format!("pi-rs-builtins-agent-{api}");
        let stream_attempts = Arc::clone(&attempts);
        let stream = move |model: &Model, context: &Context, _options: Option<_>| {
            fixture_stream(model, context, &stream_attempts)
        };
        let simple_attempts = Arc::clone(&attempts);
        register_api_provider(
            ApiProvider {
                api: api.to_owned(),
                stream: Arc::new(move |model, context, _| {
                    fixture_stream(model, context, &simple_attempts)
                }),
                stream_simple: Arc::new(move |model, context, options| {
                    stream(model, context, options)
                }),
            },
            Some(&owner),
        );
        Self { owner }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        unregister_api_providers(&self.owner);
    }
}

/// Deterministic provider behaviour keyed by model id. Every scenario the
/// agent must survive — text, tool use, retryable failure, malformed events —
/// is a fixed reply here, never a live provider.
fn fixture_stream(
    model: &Model,
    context: &Context,
    attempts: &Arc<AtomicUsize>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    match model.id.as_str() {
        "text" => Ok(text_stream(model, &["Hel", "lo", " world"])),
        "echo-context" => {
            let summary = user_texts(context).join("|");
            Ok(text_stream(model, &[summary.as_str()]))
        }
        "tools" => {
            if has_tool_results(context) {
                Ok(text_stream(model, &["settled"]))
            } else {
                Ok(tool_stream(
                    model,
                    vec![
                        tool_call("call-1", "read_note", "alpha"),
                        tool_call("call-2", "read_note", "beta"),
                        tool_call("call-3", "write_note", "gamma"),
                    ],
                ))
            }
        }
        "flaky" => {
            let seen = attempts.fetch_add(1, Ordering::SeqCst);
            if seen < 2 {
                Err(ProtocolError("fixture transport failure".into()))
            } else {
                Ok(text_stream(model, &["recovered"]))
            }
        }
        "broken" => {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(ProtocolError("fixture permanent failure".into()))
        }
        "malformed" => {
            if has_tool_results(context) {
                // Second malformed shape: a tool-use stop with no tool calls.
                let message = assistant(model, Vec::new(), StopReason::ToolUse);
                let stream = create_assistant_message_event_stream();
                stream.push(AssistantMessageEvent::TextDelta {
                    content_index: 0,
                    delta: String::new(),
                    partial: message.clone(),
                });
                stream.push(AssistantMessageEvent::Done {
                    reason: StopReason::ToolUse,
                    message,
                });
                stream.end();
                Ok(stream)
            } else {
                // First malformed shape: a call naming a tool nobody declared.
                Ok(tool_stream(model, vec![tool_call("ghost-1", "ghost", "x")]))
            }
        }
        "explode" => {
            if has_tool_results(context) {
                Ok(text_stream(model, &["continued after tool failure"]))
            } else {
                Ok(tool_stream(
                    model,
                    vec![tool_call("call-boom", "explode", "now")],
                ))
            }
        }
        "cancel" => Ok(text_stream(model, &["never", "rendered"])),
        other => Err(ProtocolError(format!("unknown fixture {other}"))),
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn agent_package_files() -> Vec<std::path::PathBuf> {
    let agent = pi_rs_builtins::package_root().join("agent");
    ["queue.lua", "tools.lua", "turn.lua", "init.lua"]
        .into_iter()
        .map(|file| agent.join(file))
        .collect()
}

const TOOL_PACKAGE: &str = r#"
local pi = ...
local tools = pi.kernel.v1.module.require("pi.agent.tools", "1")

tools.register({
  name = "read_note",
  description = "read one note",
  owner = "test-tools",
  execute = function(call)
    return { output = "read:" .. tostring(call.arguments.input) }
  end,
})

tools.register({
  name = "write_note",
  description = "write one note",
  owner = "test-tools",
  serialize = true,
  execute = function(call)
    return { output = "wrote:" .. tostring(call.arguments.input) }
  end,
})

tools.register({
  name = "explode",
  description = "always fails",
  owner = "test-tools",
  execute = function()
    error("tool failed on purpose", 0)
  end,
})
"#;

struct Harness {
    host: Host,
    _directory: tempfile::TempDir,
}

impl Harness {
    fn new(extra_packages: &[(&str, &str)]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host = Host::new(HostConfig {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..HostConfig::default()
        })
        .unwrap();
        for path in agent_package_files() {
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
        }
        for (name, source) in extra_packages {
            let path = directory.path().join(name);
            std::fs::write(&path, source).unwrap();
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {name}: {error}"));
        }
        Self {
            host,
            _directory: directory,
        }
    }

    fn with_tools() -> Self {
        Self::new(&[("tools_package.lua", TOOL_PACKAGE)])
    }

    fn dispatch(&self, event: Value) -> DispatchBatch {
        self.host
            .dispatch(DispatchRequest::new(RootKind::Agent, event, Value::Null))
            .unwrap_or_else(|error| panic!("agent dispatch failed: {error}"))
    }
}

fn model(api: &str, id: &str) -> Value {
    json!({
        "id": id,
        "name": id,
        "api": api,
        "provider": "fixture",
        "baseUrl": "http://127.0.0.1:1",
        "reasoning": false,
        "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 4096,
        "maxTokens": 128,
    })
}

fn kinds(batch: &DispatchBatch) -> Vec<String> {
    batch
        .actions
        .iter()
        .map(|action| action.kind.clone())
        .collect()
}

fn first(batch: &DispatchBatch, kind: &str) -> Value {
    batch
        .actions
        .iter()
        .find(|action| action.kind == kind)
        .unwrap_or_else(|| panic!("missing {kind} in {:?}", kinds(batch)))
        .payload
        .clone()
}

fn count(batch: &DispatchBatch, kind: &str) -> usize {
    batch
        .actions
        .iter()
        .filter(|action| action.kind == kind)
        .count()
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[test]
fn text_turn_streams_deltas_and_settles_idle() {
    let _fixture = Fixture::install("agent-text", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "hello",
        "model": model("agent-text", "text"),
    }));

    assert_eq!(
        kinds(&batch),
        vec![
            "agent_turn_start",
            "agent_status",
            "agent_text_delta",
            "agent_text_delta",
            "agent_text_delta",
            "agent_message",
            "agent_status",
        ]
    );
    assert_eq!(first(&batch, "agent_message")["text"], "Hello world");
    assert_eq!(first(&batch, "agent_message")["stop_reason"], "stop");
    let status = batch.actions.last().unwrap();
    assert_eq!(status.payload["state"], "idle");
    assert_eq!(status.payload["messages"], 2);
}

#[test]
fn tool_calls_settle_in_parallel_groups_and_serialized_order() {
    let _fixture = Fixture::install("agent-tools", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "use tools",
        "model": model("agent-tools", "tools"),
    }));

    let groups: Vec<Value> = batch
        .actions
        .iter()
        .filter(|action| action.kind == "agent_tool_group")
        .map(|action| action.payload.clone())
        .collect();
    assert_eq!(
        groups,
        vec![
            json!({"index": 1, "mode": "parallel", "calls": 2}),
            json!({"index": 2, "mode": "serial", "calls": 1}),
        ]
    );

    let results: Vec<Value> = batch
        .actions
        .iter()
        .filter(|action| action.kind == "agent_tool_result")
        .map(|action| {
            json!({
                "id": action.payload["id"],
                "group": action.payload["group"],
                "mode": action.payload["mode"],
                "ok": action.payload["ok"],
                "output": action.payload["output"],
            })
        })
        .collect();
    assert_eq!(
        results,
        vec![
            json!({"id": "call-1", "group": 1, "mode": "parallel", "ok": true, "output": "read:alpha"}),
            json!({"id": "call-2", "group": 1, "mode": "parallel", "ok": true, "output": "read:beta"}),
            json!({"id": "call-3", "group": 2, "mode": "serial", "ok": true, "output": "wrote:gamma"}),
        ]
    );

    // The follow-up request sees the settled tool results and finishes the turn.
    let messages: Vec<Value> = batch
        .actions
        .iter()
        .filter(|action| action.kind == "agent_message")
        .map(|action| action.payload["stop_reason"].clone())
        .collect();
    assert_eq!(messages, vec![json!("toolUse"), json!("stop")]);
    assert_eq!(
        batch
            .actions
            .last()
            .map(|action| action.payload["state"].clone()),
        Some(json!("idle"))
    );
}

#[test]
fn failing_tools_report_errors_without_ending_the_turn() {
    let _fixture = Fixture::install("agent-explode", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "boom",
        "model": model("agent-explode", "explode"),
    }));

    // A tool that raises settles as an error result the model can read; the
    // turn continues to the next request instead of failing the dispatch.
    let result = first(&batch, "agent_tool_result");
    assert_eq!(result["ok"], false);
    assert!(
        result["output"]
            .as_str()
            .unwrap()
            .contains("tool failed on purpose"),
        "{result}"
    );
    assert_eq!(count(&batch, "agent_error"), 0);
    assert_eq!(
        batch
            .actions
            .iter()
            .filter(|action| action.kind == "agent_message")
            .last()
            .map(|action| action.payload["text"].clone()),
        Some(json!("continued after tool failure"))
    );
}

#[test]
fn a_missing_model_is_reported_once_without_retrying() {
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({"kind": "prompt", "text": "no model yet"}));

    assert_eq!(count(&batch, "agent_retry"), 0);
    assert_eq!(first(&batch, "agent_error")["reason"], "missing_model");
    assert_eq!(first(&batch, "agent_error")["attempts"], 1);
}

#[test]
fn an_unresolvable_provider_stops_at_the_retry_bound() {
    let _fixture = Fixture::install("agent-unknown", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "boom",
        "model": model("agent-unknown", "unknown-model"),
    }));

    assert_eq!(count(&batch, "agent_retry"), 2);
    let error = first(&batch, "agent_error");
    assert_eq!(error["attempts"], 3);
    assert!(
        error["reason"]
            .as_str()
            .unwrap()
            .contains("unknown fixture"),
        "{error}"
    );
}

#[test]
fn retryable_stream_failures_recover_within_the_retry_bound() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let _fixture = Fixture::install("agent-flaky", Arc::clone(&attempts));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "retry",
        "model": model("agent-flaky", "flaky"),
    }));

    assert_eq!(count(&batch, "agent_retry"), 2);
    assert_eq!(count(&batch, "agent_error"), 0);
    assert_eq!(first(&batch, "agent_message")["text"], "recovered");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[test]
fn permanent_stream_failures_stop_at_the_retry_bound() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let _fixture = Fixture::install("agent-broken", Arc::clone(&attempts));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "never works",
        "model": model("agent-broken", "broken"),
    }));

    assert_eq!(count(&batch, "agent_retry"), 2);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(count(&batch, "agent_message"), 0);
    assert_eq!(first(&batch, "agent_error")["attempts"], 3);
}

#[test]
fn queued_interrupt_cancels_the_turn_and_leaves_the_agent_usable() {
    let _fixture = Fixture::install("agent-cancel", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let queued = harness.dispatch(json!({"kind": "interrupt"}));
    assert_eq!(first(&queued, "agent_queued")["queue"], "interrupt");

    let cancelled = harness.dispatch(json!({
        "kind": "prompt",
        "text": "long answer",
        "model": model("agent-cancel", "cancel"),
    }));
    assert_eq!(count(&cancelled, "agent_message"), 0);
    assert_eq!(count(&cancelled, "agent_text_delta"), 0);
    assert_eq!(first(&cancelled, "agent_cancelled")["reason"], "interrupt");

    // The interrupt is consumed by the cancelled turn: the next prompt runs.
    let resumed = harness.dispatch(json!({
        "kind": "prompt",
        "text": "again",
        "model": model("agent-cancel", "cancel"),
    }));
    assert_eq!(first(&resumed, "agent_message")["text"], "neverrendered");
}

#[test]
fn malformed_provider_events_are_diagnosed_without_breaking_the_turn() {
    let _fixture = Fixture::install("agent-malformed", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "confuse me",
        "model": model("agent-malformed", "malformed"),
    }));

    // An undeclared tool settles as a serialized error result, not a crash.
    let result = first(&batch, "agent_tool_result");
    assert_eq!(result["mode"], "serial");
    assert_eq!(result["ok"], false);
    assert!(
        result["output"]
            .as_str()
            .unwrap()
            .contains("unknown tool: ghost"),
        "{result}"
    );

    // An empty delta renders nothing; a tool-use stop with no calls is a
    // diagnosed dead end that still returns the agent to idle.
    assert_eq!(count(&batch, "agent_text_delta"), 0);
    let diagnostics: Vec<Value> = batch
        .actions
        .iter()
        .filter(|action| action.kind == "agent_diagnostic")
        .map(|action| action.payload["reason"].clone())
        .collect();
    assert_eq!(diagnostics, vec![json!("tool_use_without_calls")]);
    assert_eq!(
        batch
            .actions
            .last()
            .map(|action| action.payload["state"].clone()),
        Some(json!("idle"))
    );
}

#[test]
fn steering_joins_the_active_turn_and_follow_ups_run_after_it() {
    let _fixture = Fixture::install("agent-queues", Arc::new(AtomicUsize::new(0)));
    let harness = Harness::with_tools();

    let steered = harness.dispatch(json!({"kind": "steer", "text": "prefer tests"}));
    assert_eq!(first(&steered, "agent_queued")["depth"], 1);
    let queued = harness.dispatch(json!({"kind": "follow_up", "text": "then document it"}));
    assert_eq!(first(&queued, "agent_queued")["queue"], "follow_up");

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "start",
        "model": model("agent-queues", "echo-context"),
    }));

    assert_eq!(count(&batch, "agent_turn_start"), 2);
    assert_eq!(count(&batch, "agent_steered"), 1);
    assert_eq!(first(&batch, "agent_follow_up")["text"], "then document it");

    let answers: Vec<String> = batch
        .actions
        .iter()
        .filter(|action| action.kind == "agent_message")
        .map(|action| action.payload["text"].as_str().unwrap_or_default().into())
        .collect();
    assert_eq!(answers.len(), 2);
    assert!(answers[0].contains("start"), "{:?}", answers);
    assert!(answers[0].contains("prefer tests"), "{:?}", answers);
    assert!(answers[1].contains("then document it"), "{:?}", answers);

    let status = harness.dispatch(json!({"kind": "status"}));
    let payload = first(&status, "agent_status");
    assert_eq!(payload["steering"], 0);
    assert_eq!(payload["follow_ups"], 0);
    assert_eq!(payload["tools"], 3);
}

#[test]
fn a_replacement_agent_root_changes_transition_policy() {
    let _fixture = Fixture::install("agent-replaced", Arc::new(AtomicUsize::new(0)));
    let replacement = r#"
local pi = ...
local roots = pi.roots.v1
roots.register({
  kind = "agent",
  id = "test.replacement-agent",
  priority = 10,
  dispatch = function(snapshot)
    roots.action("replacement_transition", { kind = snapshot.event.kind })
  end,
})
"#;
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("replacement.lua", replacement),
    ]);

    let batch = harness.dispatch(json!({
        "kind": "prompt",
        "text": "hello",
        "model": model("agent-replaced", "text"),
    }));

    assert_eq!(kinds(&batch), vec!["replacement_transition"]);
    assert_eq!(first(&batch, "replacement_transition")["kind"], "prompt");
    assert!(
        batch.source.ends_with("replacement.lua"),
        "{}",
        batch.source
    );
}
