//! Deterministic acceptance for the shipped Lua application/frontend package.
//!
//! Every journey below drives the ordinary file-backed packages under
//! `crates/pi-rs-builtins/frontend/` through the public application root with
//! a registered fixture provider. Rust holds no presentation policy: the
//! frames asserted here are produced by Lua submitting retained display trees.

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

fn has_tool_results(context: &Context) -> bool {
    context
        .messages
        .iter()
        .any(|message| matches!(message, Message::ToolResult(_)))
}

fn fixture_stream(
    model: &Model,
    context: &Context,
    attempts: &Arc<AtomicUsize>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    attempts.fetch_add(1, Ordering::SeqCst);
    match model.id.as_str() {
        "text" | "cancel" => Ok(text_stream(model, &["Hel", "lo", " world"])),
        "tools" => {
            if has_tool_results(context) {
                Ok(text_stream(model, &["settled"]))
            } else {
                Ok(tool_stream(
                    model,
                    vec![tool_call("call-1", "read_note", "alpha")],
                ))
            }
        }
        "unauthorized" => Err(ProtocolError("401 unauthorized: invalid api key".into())),
        other => Err(ProtocolError(format!("unknown fixture {other}"))),
    }
}

/// Registers a fixture api family for one test and removes it on drop.
struct Fixture {
    owner: String,
}

impl Fixture {
    fn install(api: &str) -> Self {
        let owner = format!("pi-rs-builtins-frontend-{api}");
        let attempts = Arc::new(AtomicUsize::new(0));
        let stream_attempts = Arc::clone(&attempts);
        register_api_provider(
            ApiProvider {
                api: api.to_owned(),
                stream: Arc::new(move |model, context, _| {
                    fixture_stream(model, context, &attempts)
                }),
                stream_simple: Arc::new(move |model, context, _| {
                    fixture_stream(model, context, &stream_attempts)
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

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn package_files() -> Vec<std::path::PathBuf> {
    let root = pi_rs_builtins::package_root();
    let agent = root.join("agent");
    let frontend = root.join("frontend");
    let mut files: Vec<std::path::PathBuf> = ["queue.lua", "tools.lua", "turn.lua", "init.lua"]
        .into_iter()
        .map(|file| agent.join(file))
        .collect();
    files.extend(
        [
            "keys.lua",
            "editor.lua",
            "transcript.lua",
            "chrome.lua",
            "view.lua",
            "init.lua",
            "application.lua",
        ]
        .into_iter()
        .map(|file| frontend.join(file)),
    );
    files
}

const TOOL_PACKAGE: &str = r#"
local pi = ...
local tools = pi.kernel.v1.module.require("pi.agent.tools", "1")

tools.register({
  name = "read_note",
  description = "read one note",
  owner = "test-tools",
  execute = function(call)
    return { output = "note:" .. tostring(call.arguments.input) }
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
        for path in package_files() {
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
            .dispatch(DispatchRequest::new(
                RootKind::Application,
                event,
                Value::Null,
            ))
            .unwrap_or_else(|error| panic!("application dispatch failed: {error}"))
    }

    fn frontend(&self, event: Value) -> DispatchBatch {
        self.host
            .dispatch(DispatchRequest::new(RootKind::Frontend, event, Value::Null))
            .unwrap_or_else(|error| panic!("frontend dispatch failed: {error}"))
    }

    fn type_keys(&self, data: &str) -> DispatchBatch {
        self.dispatch(json!({"kind": "input", "data": data}))
    }

    /// A full repaint of the current retained frame, as readable text.
    ///
    /// Incremental frames only paint changed cells, so the readable screen is
    /// recovered by forcing one repaint through the ordinary resize path.
    /// Styling is dropped here because these journeys assert what the frame
    /// says; `crates/pi-rs-app/tests/transcript_presentation.rs` owns what the
    /// cells look like.
    fn screen(&self) -> String {
        let batch = self.dispatch(json!({"kind": "resize", "columns": 80, "rows": 24}));
        strip_ansi(&ansi(&batch))
    }

    /// The frontend's own status word, as it reports it.
    fn status_word(&self) -> String {
        let batch = self.frontend(json!({"kind": "status"}));
        batch
            .actions
            .iter()
            .find(|action| action.kind == "frontend_status")
            .and_then(|action| action.payload.get("status").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned()
    }

    fn start(&self, model_id: Option<&str>, api: &str) {
        if let Some(id) = model_id {
            self.dispatch(json!({"kind": "configure", "model": model(api, id)}));
        }
        self.dispatch(json!({"kind": "startup"}));
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

/// Concatenated ANSI payloads of one batch.
fn ansi(batch: &DispatchBatch) -> String {
    batch
        .actions
        .iter()
        .filter(|action| action.kind == "ansi")
        .filter_map(|action| action.payload.get("data").and_then(Value::as_str))
        .collect()
}

/// The same bytes with every escape sequence removed.
///
/// A styled run is split by SGR sequences, so a phrase that reads as one
/// string on screen is not one substring of the raw stream.
fn strip_ansi(data: &str) -> String {
    let mut out = String::with_capacity(data.len());
    let mut bytes = data.chars().peekable();
    while let Some(character) = bytes.next() {
        if character != '\u{1b}' {
            out.push(character);
            continue;
        }
        match bytes.next() {
            // CSI: parameters, then intermediates, then one final byte.
            Some('[') => {
                for byte in bytes.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&byte) {
                        break;
                    }
                }
            }
            // OSC: terminated by BEL or ST.
            Some(']') => {
                while let Some(byte) = bytes.next() {
                    if byte == '\u{7}' {
                        break;
                    }
                    if byte == '\u{1b}' && bytes.peek() == Some(&'\\') {
                        bytes.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn count(batch: &DispatchBatch, kind: &str) -> usize {
    batch
        .actions
        .iter()
        .filter(|action| action.kind == kind)
        .count()
}

// ---------------------------------------------------------------------------
// Journeys
// ---------------------------------------------------------------------------

#[test]
fn startup_reaches_an_input_ready_frame() {
    let _fixture = Fixture::install("frontend-start");
    let harness = Harness::with_tools();

    harness.dispatch(json!({"kind": "configure", "model": model("frontend-start", "text")}));
    let batch = harness.dispatch(json!({"kind": "startup"}));

    let frame = ansi(&batch);
    assert!(
        frame.contains("pi · text · idle"),
        "header missing: {frame}"
    );
    assert!(frame.contains('>'), "prompt marker missing: {frame}");
    assert!(frame.contains("enter send"), "footer missing: {frame}");
    // Only mechanism actions reach Rust; product intents stay inside Lua.
    for kind in kinds(&batch) {
        assert!(
            kind == "ansi" || kind == "shutdown",
            "unexpected host action {kind}"
        );
    }
}

#[test]
fn typed_prompt_streams_assistant_output_incrementally() {
    let _fixture = Fixture::install("frontend-text");
    let harness = Harness::with_tools();
    harness.start(Some("text"), "frontend-text");

    harness.type_keys("hi");
    let submit = harness.type_keys("\r");

    // One frame per streamed delta, not one frame at turn end.
    assert!(
        count(&submit, "ansi") >= 4,
        "expected incremental frames, got {:?}",
        kinds(&submit)
    );

    let screen = harness.screen();
    // Rows carry no author prefix: a block's colors say who spoke.
    assert!(screen.contains(" hi"), "user row missing: {screen}");
    assert!(
        screen.contains(" Hello world"),
        "assistant row missing: {screen}"
    );
    assert!(screen.contains("idle"), "status not settled: {screen}");
}

#[test]
fn tool_calls_render_start_and_result_rows() {
    let _fixture = Fixture::install("frontend-tools");
    let harness = Harness::with_tools();
    harness.start(Some("tools"), "frontend-tools");

    harness.type_keys("check\r");

    let screen = harness.screen();
    // A settled call collapses to what ran; the shipped tool block shows the
    // tool name and its arguments rather than the returned output.
    assert!(
        screen.contains("read_note") && screen.contains("alpha"),
        "tool block missing: {screen}"
    );
    assert!(
        screen.contains(" settled"),
        "assistant follow-up missing: {screen}"
    );
}

#[test]
fn interrupt_cancels_the_next_turn_and_reports_it() {
    let _fixture = Fixture::install("frontend-cancel");
    let harness = Harness::with_tools();
    harness.start(Some("cancel"), "frontend-cancel");

    harness.type_keys("\u{3}");
    assert!(
        harness.screen().contains("interrupt"),
        "interrupt notice missing"
    );

    harness.type_keys("go\r");
    let screen = harness.screen();
    assert!(
        screen.contains("Operation aborted"),
        "cancel row missing: {screen}"
    );
    assert!(
        harness.status_word().contains("cancelled"),
        "status not cancelled"
    );
}

#[test]
fn ctrl_d_on_an_empty_prompt_shuts_down() {
    let _fixture = Fixture::install("frontend-exit");
    let harness = Harness::with_tools();
    harness.start(Some("text"), "frontend-exit");

    let batch = harness.type_keys("\u{4}");

    assert_eq!(count(&batch, "shutdown"), 1, "{:?}", kinds(&batch));
    assert!(
        ansi(&batch).contains("session closed"),
        "no farewell frame: {}",
        ansi(&batch)
    );
}

#[test]
fn resize_repaints_the_whole_frame_at_the_new_size() {
    let _fixture = Fixture::install("frontend-resize");
    let harness = Harness::with_tools();
    harness.start(Some("text"), "frontend-resize");

    let batch = harness.dispatch(json!({"kind": "resize", "columns": 40, "rows": 10}));
    let frame = ansi(&batch);
    assert!(frame.contains("pi · text"), "header missing: {frame}");

    let status = harness.frontend(json!({"kind": "status"}));
    let payload = status
        .actions
        .iter()
        .find(|action| action.kind == "frontend_status")
        .expect("status action")
        .payload
        .clone();
    assert_eq!(payload.get("columns").and_then(Value::as_i64), Some(40));
    assert_eq!(payload.get("rows").and_then(Value::as_i64), Some(10));
}

#[test]
fn missing_model_produces_actionable_guidance() {
    let _fixture = Fixture::install("frontend-nomodel");
    let harness = Harness::with_tools();
    harness.dispatch(json!({"kind": "startup"}));

    harness.type_keys("hello\r");

    let screen = harness.screen();
    assert!(
        screen.contains("no model selected"),
        "missing-model guidance absent: {screen}"
    );
}

#[test]
fn rejected_credentials_produce_actionable_guidance() {
    let _fixture = Fixture::install("frontend-auth");
    let harness = Harness::with_tools();
    harness.start(Some("unauthorized"), "frontend-auth");

    harness.type_keys("hello\r");

    let screen = harness.screen();
    assert!(
        screen.contains("provider credentials missing or rejected"),
        "auth guidance absent: {screen}"
    );
    assert!(
        screen.contains("retrying"),
        "bounded retry not reported: {screen}"
    );
}

#[test]
fn multiline_editing_keeps_both_lines_before_submitting() {
    let _fixture = Fixture::install("frontend-editor");
    let harness = Harness::with_tools();
    harness.start(Some("text"), "frontend-editor");

    // alt+enter inserts a line; backspace edits the current one.
    harness.type_keys("firstX");
    harness.type_keys("\u{7f}");
    harness.type_keys("\u{1b}\r");
    harness.type_keys("second");

    let status = harness.frontend(json!({"kind": "status"}));
    let payload = status
        .actions
        .iter()
        .find(|action| action.kind == "frontend_status")
        .expect("status action")
        .payload
        .clone();
    assert_eq!(
        payload.get("input").and_then(Value::as_str),
        Some("first\nsecond")
    );

    let screen = harness.screen();
    assert!(screen.contains("> first"), "first line missing: {screen}");
    assert!(screen.contains("  second"), "second line missing: {screen}");
}

const REPLACEMENT_FRONTEND: &str = r#"
local pi = ...
local roots = pi.roots.v1

roots.register({
  kind = "frontend",
  id = "test.replacement-frontend",
  priority = 10,
  dispatch = function(snapshot)
    roots.action("ansi", { data = "[replacement:" .. tostring(snapshot.event.kind) .. "]" })
  end,
})
"#;

#[test]
fn a_file_backed_frontend_root_replaces_the_shipped_presentation() {
    let _fixture = Fixture::install("frontend-replaced");
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("replacement.lua", REPLACEMENT_FRONTEND),
    ]);

    let batch = harness.dispatch(json!({"kind": "startup"}));
    let frame = ansi(&batch);

    assert_eq!(frame, "[replacement:startup]");
    assert!(!frame.contains("pi · "), "shipped frontend still rendering");
}

const REPLACEMENT_APPLICATION: &str = r#"
local pi = ...
local roots = pi.roots.v1

roots.register({
  kind = "application",
  id = "test.replacement-application",
  priority = 10,
  dispatch = function(snapshot)
    -- A replacement coordinator drives the shipped frontend directly.
    local batch = roots.dispatch("frontend", { kind = "notice", level = "info", text = "hello from a replacement" })
    for _, action in ipairs(batch.actions) do
      if action.kind == "ansi" then
        roots.action("ansi", { data = action.payload.data })
      end
    end
    roots.action("shutdown", { reason = "replacement done" })
  end,
})
"#;

#[test]
fn a_file_backed_application_root_replaces_the_shipped_coordinator() {
    let _fixture = Fixture::install("frontend-app-replaced");
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("replacement.lua", REPLACEMENT_APPLICATION),
    ]);

    let batch = harness.dispatch(json!({"kind": "startup"}));

    assert_eq!(count(&batch, "shutdown"), 1, "{:?}", kinds(&batch));
    assert!(
        ansi(&batch).contains("hello from a replacement"),
        "shipped frontend not driven by the replacement: {}",
        ansi(&batch)
    );
}

const RENDER_MIDDLEWARE: &str = r#"
local pi = ...
local middleware = pi.roots.v1.middleware

middleware.register({
  kind = "frontend",
  phase = "render",
  id = "test.frame-marker",
  handler = function(snapshot)
    local next_actions = {}
    local rendered = false
    for _, action in ipairs(snapshot.actions) do
      if action.kind == "ansi" then
        rendered = true
      end
      local payload = {}
      for key, value in pairs(action.payload) do
        payload[key] = value
      end
      next_actions[#next_actions + 1] = { kind = action.kind, payload = payload }
    end
    if not rendered then
      return nil
    end
    next_actions[#next_actions + 1] = { kind = "ansi", payload = { data = "[wrapped]" } }
    return { actions = next_actions }
  end,
})
"#;

#[test]
fn file_backed_middleware_wraps_the_shipped_render_stage() {
    let _fixture = Fixture::install("frontend-middleware");
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("middleware.lua", RENDER_MIDDLEWARE),
    ]);

    let batch = harness.dispatch(json!({"kind": "startup"}));
    let frame = ansi(&batch);

    assert!(frame.contains("pi · "), "shipped frame missing: {frame}");
    assert!(frame.ends_with("[wrapped]"), "middleware not applied");
}

/// Claims the shipped `user` block through the one generic declaration path.
///
/// Nothing here is frontend-private: `pi.kernel.v1.declare` is the same call
/// a theme, a provider, or a tool declaration uses, and `context.line` is the
/// same primitive the shipped renderers build their rows from.
const REPLACEMENT_USER_BLOCK: &str = r#"
local pi = ...
local kernel = pi.kernel.v1

kernel.declare("renderer", {
  id = "test.user-block",
  surface = "transcript.block",
  entry = "user",
  order = 10,
  render = function(entry, context)
    return { context.line({ { text = "<mine> " .. tostring(entry.text) } }) }
  end,
})
"#;

#[test]
fn a_file_backed_renderer_replaces_one_transcript_block() {
    let _fixture = Fixture::install("frontend-renderer");
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("renderer.lua", REPLACEMENT_USER_BLOCK),
    ]);
    harness.start(Some("text"), "frontend-renderer");

    harness.type_keys("hi\r");
    let screen = harness.screen();

    // The claimed block is the replacement's.
    assert!(
        screen.contains("<mine> hi"),
        "file-backed user block not used: {screen}"
    );
    // Every unclaimed block is still the shipped one, and the rest of the
    // frontend root is untouched: this is a block replacement, not a fork.
    assert!(
        screen.contains(" Hello world"),
        "shipped assistant block lost: {screen}"
    );
    assert!(
        screen.contains("pi · text · idle"),
        "shipped chrome lost: {screen}"
    );
}

const LOW_ORDER_NOTICE_BLOCK: &str = r#"
local pi = ...
pi.kernel.v1.declare("renderer", {
  id = "test.notice-low",
  surface = "transcript.block",
  entry = "notice",
  order = 5,
  render = function(entry, context)
    return { context.line({ { text = "[low] " .. tostring(entry.text) } }) }
  end,
})
"#;

const HIGH_ORDER_NOTICE_BLOCK: &str = r#"
local pi = ...
pi.kernel.v1.declare("renderer", {
  id = "test.notice-high",
  surface = "transcript.block",
  entry = "notice",
  order = 20,
  render = function(entry, context)
    return { context.line({ { text = "[high] " .. tostring(entry.text) } }) }
  end,
})
"#;

#[test]
fn renderer_order_decides_which_block_wins_not_load_order() {
    let _fixture = Fixture::install("frontend-renderer-order");
    // The higher `order` is loaded first, so winning cannot be "last loaded".
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("notice_high.lua", HIGH_ORDER_NOTICE_BLOCK),
        ("notice_low.lua", LOW_ORDER_NOTICE_BLOCK),
    ]);
    harness.start(Some("text"), "frontend-renderer-order");

    harness.frontend(json!({"kind": "notice", "level": "info", "text": "ping"}));
    let screen = harness.screen();

    assert!(
        screen.contains("[high] ping"),
        "highest declared order did not win: {screen}"
    );
    assert!(
        !screen.contains("[low] ping"),
        "losing renderer still painted: {screen}"
    );
}

/// The action the shipped agent emits when a provider request starts.
fn streaming() -> Value {
    json!({"kind": "agent", "actions": [
        {"kind": "agent_status", "payload": {"state": "streaming"}}
    ]})
}

#[test]
fn lines_typed_during_a_turn_are_queued_by_the_agent_and_shown_as_pending() {
    let _fixture = Fixture::install("frontend-queues");
    let harness = Harness::with_tools();
    harness.start(Some("text"), "frontend-queues");
    harness.frontend(streaming());

    // enter steers the running turn, alt+enter queues a follow-up. Both rows
    // are painted only because the agent answered `agent_queued`: the typed
    // line went frontend intent -> application -> agent -> frontend.
    harness.type_keys("also check tests\r");
    harness.type_keys("then run lint\u{1b}\r");

    let screen = harness.screen();
    assert!(
        screen.contains("Steering: also check tests"),
        "steering row missing: {screen}"
    );
    assert!(
        screen.contains("Follow-up: then run lint"),
        "follow-up row missing: {screen}"
    );
    assert!(
        screen.contains("Alt+Up to edit all queued messages"),
        "dequeue hint missing: {screen}"
    );
    // Neither line became a user block: it has not been sent yet.
    assert!(
        !screen.contains("> also check tests"),
        "a queued line must not enter the transcript as a turn: {screen}"
    );

    let status = harness.frontend(json!({"kind": "status"}));
    let queued = status
        .actions
        .iter()
        .find(|action| action.kind == "frontend_status")
        .and_then(|action| action.payload.get("queued").cloned())
        .expect("status must report the pending queue");
    assert_eq!(queued["steer"][0], "also check tests");
    assert_eq!(queued["follow_up"][0], "then run lint");
}

/// Claims the pending-queue block through the same declaration path the
/// shipped transcript blocks use.
const REPLACEMENT_QUEUE_BLOCK: &str = r#"
local pi = ...
pi.kernel.v1.declare("renderer", {
  id = "test.queue-block",
  surface = "transcript.block",
  entry = "queue",
  order = 10,
  render = function(entry, context)
    return { context.line({ { text = "<pending " .. #entry.steering .. ">" } }) }
  end,
})
"#;

#[test]
fn a_file_backed_renderer_replaces_the_pending_queue_block() {
    let _fixture = Fixture::install("frontend-queue-renderer");
    let harness = Harness::new(&[
        ("tools_package.lua", TOOL_PACKAGE),
        ("queue_block.lua", REPLACEMENT_QUEUE_BLOCK),
    ]);
    harness.start(Some("text"), "frontend-queue-renderer");
    harness.frontend(streaming());

    harness.type_keys("also check tests\r");
    let screen = harness.screen();

    assert!(
        screen.contains("<pending 1>"),
        "file-backed queue block not used: {screen}"
    );
    assert!(
        !screen.contains("Steering: also check tests"),
        "shipped queue block still painted: {screen}"
    );
}
