//! Acceptance for the default distribution: the shipped manifest, the
//! installed launcher journey, and the offline default coding journey.
//!
//! The distribution is one declarative manifest over ordinary file-backed Lua
//! packages. Nothing here is embedded, concatenated, or privileged: the same
//! files copied anywhere on disk must behave identically, and the raw
//! zero-package launcher must stay reachable.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
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
use pi_rs_host::{Host, HostConfig, HostError, PackageSource};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// The shipped package files, resolved from the shipped manifest in load
/// order. Reading the manifest here is deliberate: the test cannot pass with
/// a manifest that forgets a file.
fn manifest_packages(manifest: &Path) -> Vec<PathBuf> {
    let document: Value =
        serde_json::from_slice(&std::fs::read(manifest).expect("read manifest")).expect("manifest");
    assert_eq!(document.get("version").and_then(Value::as_u64), Some(1));
    let base = manifest.parent().expect("manifest directory");
    document
        .get("packages")
        .and_then(Value::as_array)
        .expect("packages")
        .iter()
        .map(|entry| base.join(entry.as_str().expect("package path")))
        .collect()
}

fn shipped_lua_files() -> Vec<PathBuf> {
    let root = pi_rs_builtins::package_root();
    let mut found = Vec::new();
    for tree in [
        "config", "agent", "tools", "frontend", "session", "defaults",
    ] {
        let directory = root.join(tree);
        for entry in std::fs::read_dir(&directory).expect("read package tree") {
            let path = entry.expect("directory entry").path();
            if path.extension().is_some_and(|value| value == "lua") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_manifest_indexes_every_shipped_package_exactly_once() {
    let selected = manifest_packages(&pi_rs_builtins::manifest_path());

    for path in &selected {
        assert!(path.is_file(), "manifest selects a missing file: {path:?}");
    }
    let mut sorted = selected.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        selected.len(),
        "manifest selects a package twice"
    );
    assert_eq!(
        sorted,
        shipped_lua_files(),
        "manifest and shipped package trees disagree"
    );
}

// ---------------------------------------------------------------------------
// Private storage roots
// ---------------------------------------------------------------------------

/// One scenario's private HOME, XDG roots, and workspace.
///
/// The shipped distribution now carries the configuration and session
/// packages, so a run reads `<config>/pi/config.lua` and may write session
/// records under `<state>/pi/sessions`. A test that inherited the developer's
/// real environment would read their configuration and write their session
/// directory, so every root is pinned here and nothing outside the temporary
/// directory is reachable.
struct Sandbox {
    directory: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("sandbox");
        for entry in ["home", "config", "data", "state", "cache", "workspace"] {
            std::fs::create_dir_all(directory.path().join(entry)).expect("sandbox directory");
        }
        Self { directory }
    }

    fn path(&self, entry: &str) -> PathBuf {
        self.directory.path().join(entry)
    }

    fn workspace(&self) -> PathBuf {
        self.path("workspace")
    }

    /// Canonical session destination: `$XDG_STATE_HOME/pi/sessions`.
    fn sessions(&self) -> PathBuf {
        self.path("state").join("pi").join("sessions")
    }

    /// Canonical user configuration: `$XDG_CONFIG_HOME/pi/config.lua`.
    fn write_configuration(&self, text: &str) {
        let directory = self.path("config").join("pi");
        std::fs::create_dir_all(&directory).expect("configuration directory");
        std::fs::write(directory.join("config.lua"), text).expect("configuration file");
    }

    /// A file-backed package under the canonical packages resource,
    /// `$XDG_DATA_HOME/pi/packages`. This is exactly where a configuration's
    /// `packages` entries resolve from, so nothing here is a test-only
    /// loading path.
    fn write_package(&self, name: &str, text: &str) -> PathBuf {
        let directory = self.path("data").join("pi").join("packages");
        std::fs::create_dir_all(&directory).expect("packages directory");
        let path = directory.join(name);
        std::fs::write(&path, text).expect("package file");
        path
    }

    /// Exactly the variables `pi.config.paths@1` reads, and nothing else.
    fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("HOME".to_owned(), text(&self.path("home"))),
            ("XDG_CONFIG_HOME".to_owned(), text(&self.path("config"))),
            ("XDG_DATA_HOME".to_owned(), text(&self.path("data"))),
            ("XDG_STATE_HOME".to_owned(), text(&self.path("state"))),
            ("XDG_CACHE_HOME".to_owned(), text(&self.path("cache"))),
        ])
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Every `.jsonl` session log written under the sandbox's state root.
fn session_logs(sandbox: &Sandbox) -> Vec<PathBuf> {
    let directory = sandbox.sessions();
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut found = entries
        .map(|entry| entry.expect("session entry").path())
        .filter(|path| path.extension().is_some_and(|value| value == "jsonl"))
        .collect::<Vec<_>>();
    found.sort();
    found
}

// ---------------------------------------------------------------------------
// Installed launcher journey
// ---------------------------------------------------------------------------

/// Runs the built `pi` binary with an explicit manifest inside a sandbox.
/// stdin is not a terminal here, so the launcher serializes the startup batch
/// — the same batch an interactive session presents as its first frame.
fn run_launcher(sandbox: &Sandbox, manifest: &Path) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pi"));
    command
        .arg("--root")
        .arg(sandbox.workspace())
        .arg("--manifest")
        .arg(manifest)
        .env_remove("PI_PACKAGE_MANIFEST");
    for (name, value) in sandbox.environment() {
        command.env(name, value);
    }
    let output = command.output().expect("run pi");
    assert!(
        output.status.success(),
        "launcher failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "launcher output is not a batch ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn frame(batch: &Value) -> String {
    batch
        .get("actions")
        .and_then(Value::as_array)
        .expect("actions")
        .iter()
        .filter(|action| action.get("kind").and_then(Value::as_str) == Some("ansi"))
        .filter_map(|action| {
            action
                .get("payload")
                .and_then(|payload| payload.get("data"))
                .and_then(Value::as_str)
        })
        .collect()
}

fn action_kinds(batch: &Value) -> Vec<String> {
    batch
        .get("actions")
        .and_then(Value::as_array)
        .expect("actions")
        .iter()
        .filter_map(|action| action.get("kind").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Writes a private manifest that selects every shipped package except one
/// tree. Manifest entries are absolute, so the copy behaves exactly like the
/// shipped index while living in the sandbox.
fn manifest_without(sandbox: &Sandbox, tree: &str) -> PathBuf {
    let selected = manifest_packages(&pi_rs_builtins::manifest_path())
        .into_iter()
        .filter(|path| {
            path.parent()
                .and_then(Path::file_name)
                .is_none_or(|name| name != tree)
        })
        .map(|path| Value::String(text(&path)))
        .collect::<Vec<_>>();
    let path = sandbox.path("workspace").join(format!("no-{tree}.json"));
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"version": 1, "packages": selected})).expect("manifest"),
    )
    .expect("write manifest");
    path
}

#[test]
fn the_default_distribution_starts_input_ready() {
    let sandbox = Sandbox::new();
    let batch = run_launcher(&sandbox, &pi_rs_builtins::manifest_path());
    let screen = frame(&batch);

    assert!(screen.contains("pi · "), "header missing: {screen}");
    assert!(screen.contains("idle"), "not input-ready: {screen}");
    assert!(screen.contains("enter send"), "footer missing: {screen}");
    assert!(
        !action_kinds(&batch).iter().any(|kind| kind == "shutdown"),
        "startup should not shut down: {:?}",
        action_kinds(&batch)
    );
    // Only mechanism actions cross into Rust.
    for kind in action_kinds(&batch) {
        assert_eq!(kind, "ansi", "unexpected host action {kind}");
    }
    // Persistence is policy over a conversation, not a startup side effect:
    // a launch that says nothing leaves the state root untouched.
    assert!(
        session_logs(&sandbox).is_empty(),
        "startup wrote a session log: {:?}",
        session_logs(&sandbox)
    );
    assert!(
        !sandbox.path("state").join("pi").exists(),
        "startup created a state root"
    );
}

#[test]
fn the_default_distribution_selects_a_model_without_configuration() {
    let sandbox = Sandbox::new();
    let screen = frame(&run_launcher(&sandbox, &pi_rs_builtins::manifest_path()));

    assert!(
        screen.contains("claude-sonnet-4-5"),
        "no default model in the header: {screen}"
    );
    assert!(
        !screen.contains("no model"),
        "default model not applied: {screen}"
    );
}

/// The shipped configuration package is part of the distribution, so an
/// ordinary `config.lua` outranks the distribution default without replacing
/// or forking anything: its stage runs at order `-200`, before the
/// distribution's own `-100` model stage.
#[test]
fn a_user_configuration_outranks_the_distribution_model() {
    let sandbox = Sandbox::new();
    sandbox.write_configuration("return { model = { provider = 'openai', id = 'gpt-5.1' } }\n");

    let screen = frame(&run_launcher(&sandbox, &pi_rs_builtins::manifest_path()));

    assert!(
        screen.contains("gpt-5.1"),
        "configured model missing from the header: {screen}"
    );
    assert!(
        !screen.contains("claude-sonnet-4-5"),
        "distribution default still applied: {screen}"
    );
}

/// Sessions are optional: a distribution index without the session tree is
/// still the same input-ready product, and nothing degrades to a diagnostic.
#[test]
fn a_distribution_without_the_session_package_starts_input_ready() {
    let sandbox = Sandbox::new();
    let suppressed = manifest_without(&sandbox, "session");

    let shipped = frame(&run_launcher(&sandbox, &pi_rs_builtins::manifest_path()));
    let without = frame(&run_launcher(&sandbox, &suppressed));

    assert_eq!(
        shipped, without,
        "suppressing the session package changed the first frame"
    );
    assert!(without.contains("idle"), "not input-ready: {without}");
}

#[test]
fn every_shipped_source_copied_to_disk_behaves_identically() {
    let sandbox = Sandbox::new();
    let copy = sandbox.path("distribution");
    let manifest = pi_rs_builtins::manifest_path();
    let base = manifest.parent().expect("manifest directory");

    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(&manifest, copy.join("default.json")).unwrap();
    for path in manifest_packages(&manifest) {
        let relative = path.strip_prefix(base).expect("package inside the tree");
        let target = copy.join(relative);
        std::fs::create_dir_all(target.parent().expect("target directory")).unwrap();
        std::fs::copy(&path, &target).unwrap();
    }

    let shipped = run_launcher(&sandbox, &manifest);
    let copied = run_launcher(&sandbox, &copy.join("default.json"));

    assert_eq!(
        frame(&shipped),
        frame(&copied),
        "the copied distribution renders a different frame"
    );
    assert_eq!(action_kinds(&shipped), action_kinds(&copied));
}

// ---------------------------------------------------------------------------
// Offline fixture provider
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

fn text_stream(model: &Model, text: &str) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let partial = assistant(
        model,
        vec![AssistantContent::Text(TextContent::new(text))],
        StopReason::Stop,
    );
    stream.push(AssistantMessageEvent::Start {
        partial: partial.clone(),
    });
    stream.push(AssistantMessageEvent::TextDelta {
        content_index: 0,
        delta: text.to_owned(),
        partial: partial.clone(),
    });
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message: partial,
    });
    stream.end();
    stream
}

/// A provider response that settles without deltas: `Start` then `Done`.
///
/// The shipped transcript keeps an already-streamed row as it was streamed and
/// repaints a non-streamed one from the settled `agent_message`, so a
/// non-streaming response is where a render stage's transform is observable in
/// the frame as well as in the persisted record.
fn settled_stream(model: &Model, text: &str) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let message = assistant(
        model,
        vec![AssistantContent::Text(TextContent::new(text))],
        StopReason::Stop,
    );
    stream.push(AssistantMessageEvent::Start {
        partial: message.clone(),
    });
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::Stop,
        message,
    });
    stream.end();
    stream
}

fn read_call(path: &str) -> ToolCall {
    let mut arguments = serde_json::Map::new();
    arguments.insert("path".to_owned(), Value::String(path.to_owned()));
    ToolCall {
        r#type: ToolCallType::ToolCall,
        id: "call-1".to_owned(),
        name: "read".to_owned(),
        arguments,
        thought_signature: None,
    }
}

fn tool_stream(model: &Model, call: ToolCall) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let message = assistant(
        model,
        vec![AssistantContent::ToolCall(call.clone())],
        StopReason::ToolUse,
    );
    stream.push(AssistantMessageEvent::Start {
        partial: message.clone(),
    });
    stream.push(AssistantMessageEvent::ToolCallEnd {
        content_index: 0,
        tool_call: call,
        partial: message.clone(),
    });
    stream.push(AssistantMessageEvent::Done {
        reason: StopReason::ToolUse,
        message,
    });
    stream.end();
    stream
}

fn fixture_stream(
    model: &Model,
    context: &Context,
    attempts: &Arc<AtomicUsize>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    attempts.fetch_add(1, Ordering::SeqCst);
    match model.id.as_str() {
        "tools" => {
            if context
                .messages
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                Ok(text_stream(model, "note read"))
            } else {
                Ok(tool_stream(model, read_call("note.txt")))
            }
        }
        // One settled assistant message, no deltas and no tool loop: the
        // shortest real turn a composition scenario can assert on.
        "plain" => Ok(settled_stream(model, "plain answer")),
        "unauthorized" => Err(ProtocolError("401 unauthorized: invalid api key".into())),
        other => Err(ProtocolError(format!("unknown fixture {other}"))),
    }
}

struct Fixture {
    owner: String,
}

impl Fixture {
    fn install(api: &str) -> Self {
        let owner = format!("pi-rs-app-distribution-{api}");
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

fn fixture_model(api: &str, id: &str) -> Value {
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

// ---------------------------------------------------------------------------
// Offline default journey
// ---------------------------------------------------------------------------

/// The whole shipped distribution, loaded exactly as the launcher loads it,
/// inside one sandbox: private XDG roots, private workspace.
struct Distribution {
    host: Host,
    sandbox: Sandbox,
}

impl Distribution {
    fn new() -> Self {
        Self::from_packages(
            Sandbox::new(),
            &manifest_packages(&pi_rs_builtins::manifest_path()),
        )
    }

    /// The sandbox is built by the caller so a scenario can write its
    /// `config.lua` and its packages *before* the host loads anything: the
    /// configuration is read on the first application dispatch.
    fn from_packages(sandbox: Sandbox, packages: &[PathBuf]) -> Self {
        Self::from_packages_with_timeout(
            sandbox,
            packages,
            HostConfig::default().dispatch_timeout_ms,
        )
    }

    /// The same distribution under an explicit per-dispatch watchdog budget,
    /// so a scenario can observe the bound rather than wait for the default.
    fn from_packages_with_timeout(
        sandbox: Sandbox,
        packages: &[PathBuf],
        dispatch_timeout_ms: i64,
    ) -> Self {
        let host = Host::new(HostConfig {
            dispatch_timeout_ms,
            cwd: Some(text(&sandbox.workspace())),
            environment: Some(sandbox.environment()),
        })
        .unwrap();
        for path in packages {
            host.load_package(PackageSource::File { path })
                .unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
        }
        Self { host, sandbox }
    }

    fn workspace(&self) -> PathBuf {
        self.sandbox.workspace()
    }

    /// The launcher publishes the resolved root on every application
    /// dispatch; the in-process journey mirrors that context exactly.
    fn try_dispatch(&self, event: Value) -> Result<DispatchBatch, HostError> {
        let context = json!({"root": text(&self.workspace())});
        self.host
            .dispatch(DispatchRequest::new(RootKind::Application, event, context))
    }

    fn dispatch(&self, event: Value) -> DispatchBatch {
        self.try_dispatch(event)
            .unwrap_or_else(|error| panic!("application dispatch failed: {error}"))
    }

    /// A full repaint of the retained frame as plain text.
    fn screen(&self) -> String {
        let batch = self.dispatch(json!({"kind": "resize", "columns": 80, "rows": 24}));
        batch
            .actions
            .iter()
            .filter(|action| action.kind == "ansi")
            .filter_map(|action| action.payload.get("data").and_then(Value::as_str))
            .collect()
    }

    /// One `session` command, answered by the shipped session package's
    /// application stage and returned as its `session_result` payload.
    fn session(&self, command: &str) -> Value {
        let batch = self.dispatch(json!({"kind": "session", "command": command}));
        batch
            .actions
            .iter()
            .find(|action| action.kind == "session_result")
            .map(|action| action.payload.clone())
            .unwrap_or(Value::Null)
    }

    fn start_with_fixture(&self, api: &str, id: &str) {
        self.dispatch(json!({"kind": "startup"}));
        self.dispatch(json!({"kind": "configure", "model": fixture_model(api, id)}));
    }
}

#[test]
fn the_default_distribution_completes_a_prompt_and_a_tool_call_offline() {
    let _fixture = Fixture::install("distribution-tools");
    let distribution = Distribution::new();
    std::fs::write(distribution.workspace().join("note.txt"), "alpha\n").unwrap();

    distribution.start_with_fixture("distribution-tools", "tools");
    distribution.dispatch(json!({"kind": "input", "data": "read the note\r"}));

    let screen = distribution.screen();
    assert!(
        screen.contains("you  read the note"),
        "user row missing: {screen}"
    );
    assert!(
        screen.contains("+ read →"),
        "shipped read tool row missing: {screen}"
    );
    assert!(
        screen.contains("pi   note read"),
        "assistant follow-up missing: {screen}"
    );
    assert!(screen.contains("idle"), "turn not settled: {screen}");
}

#[test]
fn the_default_distribution_reports_missing_credentials() {
    let _fixture = Fixture::install("distribution-auth");
    let distribution = Distribution::new();

    distribution.start_with_fixture("distribution-auth", "unauthorized");
    distribution.dispatch(json!({"kind": "input", "data": "hello\r"}));

    let screen = distribution.screen();
    assert!(
        screen.contains("provider credentials missing or rejected"),
        "credential guidance absent: {screen}"
    );
}

// ---------------------------------------------------------------------------
// Optional persistence
// ---------------------------------------------------------------------------

fn records(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("read session log")
        .lines()
        .skip(1) // the store's own format header
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .expect("record")
                .get("value")
                .cloned()
                .expect("record value")
        })
        .collect()
}

/// The shipped distribution persists a real conversation: one log under the
/// canonical XDG state entry, carrying the same public agent vocabulary the
/// session package folds, and the `session` command answers from inside the
/// distribution rather than from a test driver.
#[test]
fn a_conversation_persists_one_session_log_under_the_state_root() {
    let _fixture = Fixture::install("distribution-session");
    let distribution = Distribution::new();

    distribution.start_with_fixture("distribution-session", "tools");
    assert!(
        session_logs(&distribution.sandbox).is_empty(),
        "configuring a model started a log before the conversation"
    );

    std::fs::write(distribution.workspace().join("note.txt"), "alpha\n").unwrap();
    distribution.dispatch(json!({"kind": "input", "data": "read the note\r"}));

    let logs = session_logs(&distribution.sandbox);
    assert_eq!(logs.len(), 1, "expected exactly one session log: {logs:?}");
    let written = records(&logs[0]);
    let kinds = written
        .iter()
        .filter_map(|record| record.get("kind").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            "header", "model", "message", "message", "message", "message"
        ],
        "unexpected persisted vocabulary: {written:#?}"
    );
    assert_eq!(written[2]["role"], "user");
    assert_eq!(written[2]["text"], "read the note");
    assert_eq!(written[4]["role"], "tool");
    assert_eq!(written[4]["name"], "read");

    let status = distribution.session("status");
    assert_eq!(status["ok"], true, "session status refused: {status}");
    // user prompt, the tool-calling assistant turn, its tool result, and the
    // assistant's follow-up.
    assert_eq!(status["session"]["messages"], 4);
    assert_eq!(status["directory"], text(&distribution.sandbox.sessions()));
}

/// Removing the session package from the index leaves exactly the ephemeral
/// product: the same completed turn, nothing written, and the unanswered
/// `session` command reported by the application root instead.
#[test]
fn suppressing_the_session_package_leaves_an_ephemeral_conversation() {
    let _fixture = Fixture::install("distribution-ephemeral");
    let selected = manifest_packages(&pi_rs_builtins::manifest_path())
        .into_iter()
        .filter(|path| {
            path.parent()
                .and_then(Path::file_name)
                .is_none_or(|name| name != "session")
        })
        .collect::<Vec<_>>();
    let distribution = Distribution::from_packages(Sandbox::new(), &selected);

    distribution.start_with_fixture("distribution-ephemeral", "tools");
    std::fs::write(distribution.workspace().join("note.txt"), "alpha\n").unwrap();
    distribution.dispatch(json!({"kind": "input", "data": "read the note\r"}));

    let screen = distribution.screen();
    assert!(
        screen.contains("pi   note read"),
        "the conversation still completes: {screen}"
    );
    assert!(
        session_logs(&distribution.sandbox).is_empty(),
        "a suppressed session package wrote records"
    );
    assert!(
        !distribution.sandbox.path("state").join("pi").exists(),
        "a suppressed session package created the state root"
    );
    assert_eq!(
        distribution.session("status"),
        Value::Null,
        "a suppressed session package still answered a command"
    );
}

// ---------------------------------------------------------------------------
// Root replacement at distribution level
// ---------------------------------------------------------------------------

/// A file-backed agent root with no provider, no tool loop, and no shared
/// module: everything it needs is the public event vocabulary the shipped
/// application coordinator already dispatches (`configure`, `prompt`) and the
/// public action vocabulary the shipped frontend and session package already
/// read. Its priority is *below* the shipped agent's `0`, so nothing it does
/// can be explained by outbidding a registration.
const REPLACEMENT_AGENT: &str = r#"
local pi = ...
local roots = pi.roots.v1

roots.register({
  kind = "agent",
  id = "acceptance.agent",
  active = true,
  priority = -10,
  dispatch = function(snapshot)
    local event = snapshot.event
    local kind = type(event) == "table" and event.kind or nil
    if kind == "configure" then
      local model = type(event.model) == "table" and event.model.id or nil
      roots.action("agent_configured", { model = model })
      return
    end
    if kind == "prompt" then
      local text = tostring(event.text or "")
      roots.action("agent_turn_start", { prompt = text })
      roots.action("agent_message", { text = "replacement agent: " .. text })
      roots.action("agent_status", { state = "idle", messages = 2 })
      return
    end
  end,
})
"#;

/// The shipped agent is replaceable from ordinary configuration, at
/// distribution level: the whole shipped index is loaded, a `config.lua`
/// loads one file-backed package and names its agent root, and the rest of
/// the distribution keeps working over it. Nothing is forked, suppressed, or
/// outbid, and the replacement shares no module with the shipped agent.
#[test]
fn a_configured_package_replaces_the_shipped_agent_root() {
    let sandbox = Sandbox::new();
    sandbox.write_package("agent.lua", REPLACEMENT_AGENT);
    sandbox.write_configuration(
        r#"
return {
  packages = { "agent.lua" },
  roots = { agent = "acceptance.agent" },
}
"#,
    );
    let distribution = Distribution::from_packages(
        sandbox,
        &manifest_packages(&pi_rs_builtins::manifest_path()),
    );

    distribution.dispatch(json!({"kind": "startup"}));
    distribution.dispatch(json!({"kind": "input", "data": "who answers?\r"}));

    // The shipped frontend still owns the whole presentation: its chrome, its
    // user row, its assistant row, its idle status.
    let screen = distribution.screen();
    assert!(screen.contains("pi · "), "shipped chrome missing: {screen}");
    assert!(
        screen.contains("you  who answers?"),
        "shipped user row missing: {screen}"
    );
    assert!(
        screen.contains("pi   replacement agent: who answers?"),
        "the replacement agent did not answer: {screen}"
    );
    assert!(screen.contains("idle"), "turn not settled: {screen}");

    // ... and the shipped session package folds the replacement's batch
    // through the same public action vocabulary, with no source check.
    let logs = session_logs(&distribution.sandbox);
    assert_eq!(logs.len(), 1, "expected exactly one session log: {logs:?}");
    let written = records(&logs[0]);
    let kinds = written
        .iter()
        .filter_map(|record| record.get("kind").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec!["header", "model", "message", "message"],
        "a replacement agent was persisted differently: {written:#?}"
    );
    assert_eq!(written[2]["role"], "user");
    assert_eq!(written[2]["text"], "who answers?");
    assert_eq!(written[3]["role"], "assistant");
    assert_eq!(written[3]["text"], "replacement agent: who answers?");

    let status = distribution.session("status");
    assert_eq!(status["ok"], true, "session status refused: {status}");
    assert_eq!(status["session"]["messages"], 2);
}

/// The control for the claim above: the same package, loaded by the same
/// configuration, with only `roots.agent` removed. Priority resolution keeps
/// the shipped agent, so the replacement wins by being *named*, never by
/// registering.
#[test]
fn a_loaded_replacement_agent_does_not_take_over_by_registering() {
    let _fixture = Fixture::install("distribution-unselected");
    let sandbox = Sandbox::new();
    sandbox.write_package("agent.lua", REPLACEMENT_AGENT);
    sandbox.write_configuration("return { packages = { 'agent.lua' } }\n");
    let distribution = Distribution::from_packages(
        sandbox,
        &manifest_packages(&pi_rs_builtins::manifest_path()),
    );

    distribution.start_with_fixture("distribution-unselected", "unauthorized");
    distribution.dispatch(json!({"kind": "input", "data": "who answers?\r"}));

    let screen = distribution.screen();
    assert!(
        !screen.contains("replacement agent"),
        "an unselected registration took over the kind: {screen}"
    );
    assert!(
        screen.contains("provider credentials missing or rejected"),
        "the shipped agent did not run the turn: {screen}"
    );
}

// ---------------------------------------------------------------------------
// Composition across the whole distribution
// ---------------------------------------------------------------------------

/// One ordinary file-backed extension: a single public `agent`/`render` stage
/// that marks every settled assistant message.
///
/// It owns no root, defines no module, shares no state with the distribution,
/// and never asks who produced an action or who else is registered. `order` is
/// the only thing that decides when it runs, which is what makes two of these
/// composable without either one being aware of the other.
fn marking_extension(id: &str, mark: &str, order: i64) -> String {
    format!(
        r#"
local pi = ...
local roots = pi.roots.v1

-- Dispatch snapshots are immutable, so a stage that changes an action rebuilds
-- it rather than writing through the read-only view.
local function copy(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {{}}
  for key, item in pairs(value) do
    out[key] = copy(item)
  end
  return out
end

roots.middleware.register({{
  kind = "agent",
  phase = "render",
  id = "{id}",
  order = {order},
  handler = function(snapshot)
    local replaced = {{}}
    for index, action in ipairs(snapshot.actions) do
      local next_action = copy(action)
      if next_action.kind == "agent_message" then
        local payload = next_action.payload or {{}}
        payload.text = tostring(payload.text or "") .. "{mark}"
        next_action.payload = payload
      end
      replaced[index] = next_action
    end
    return {{ actions = replaced }}
  end,
}})
"#
    )
}

/// The whole shipped distribution plus the extensions an ordinary user
/// configuration loads from the canonical packages resource.
fn distribution_with_extensions(
    extensions: &[(&str, String)],
    configuration: &str,
) -> Distribution {
    let sandbox = Sandbox::new();
    for (name, source) in extensions {
        sandbox.write_package(name, source);
    }
    sandbox.write_configuration(configuration);
    Distribution::from_packages(
        sandbox,
        &manifest_packages(&pi_rs_builtins::manifest_path()),
    )
}

/// Every assistant text the shipped session package persisted for this run,
/// in turn order.
fn persisted_assistant_texts(distribution: &Distribution) -> Vec<String> {
    let logs = session_logs(&distribution.sandbox);
    assert_eq!(logs.len(), 1, "expected exactly one session log: {logs:?}");
    records(&logs[0])
        .iter()
        .filter(|record| record.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|record| {
            record
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// Two extensions compose over the whole distribution with no privileged
/// ordering anywhere: neither is in the shipped manifest, neither outranks the
/// other, and the *shipped* session package's recording stage (`agent`/`render`
/// at order `100`) is simply the third stage in the same chain. It records what
/// the two extensions produced, because it reads the public action vocabulary
/// and never asks which source produced an action.
#[test]
fn two_configured_extensions_compose_without_privileged_ordering() {
    let _fixture = Fixture::install("distribution-compose");
    let distribution = distribution_with_extensions(
        &[
            (
                "mark-a.lua",
                marking_extension("acceptance.mark.a", " [a]", 10),
            ),
            (
                "mark-b.lua",
                marking_extension("acceptance.mark.b", " [b]", 20),
            ),
        ],
        "return { packages = { 'mark-a.lua', 'mark-b.lua' } }\n",
    );

    distribution.start_with_fixture("distribution-compose", "plain");
    distribution.dispatch(json!({"kind": "input", "data": "compose\r"}));

    // Lower `order` runs first, so `[a]` is appended before `[b]`.
    let screen = distribution.screen();
    assert!(
        screen.contains("pi   plain answer [a] [b]"),
        "the two extensions did not compose in declared order: {screen}"
    );
    assert!(
        screen.contains("pi \u{b7} "),
        "the shipped frontend stopped rendering its chrome: {screen}"
    );
    assert_eq!(
        persisted_assistant_texts(&distribution),
        vec!["plain answer [a] [b]"],
        "the shipped session package did not record the composed batch"
    );
}

/// The control for the claim above: the same two files, the same names, the
/// same `packages` order, the same registration order — only the two `order`
/// numbers trade places, and the composition flips with them. Nothing about
/// load order, file name, or source decides the chain.
#[test]
fn swapping_only_the_declared_order_swaps_the_composition() {
    let _fixture = Fixture::install("distribution-swapped");
    let distribution = distribution_with_extensions(
        &[
            (
                "mark-a.lua",
                marking_extension("acceptance.mark.a", " [a]", 20),
            ),
            (
                "mark-b.lua",
                marking_extension("acceptance.mark.b", " [b]", 10),
            ),
        ],
        "return { packages = { 'mark-a.lua', 'mark-b.lua' } }\n",
    );

    distribution.start_with_fixture("distribution-swapped", "plain");
    distribution.dispatch(json!({"kind": "input", "data": "compose\r"}));

    let screen = distribution.screen();
    assert!(
        screen.contains("pi   plain answer [b] [a]"),
        "the declared order did not decide the composition: {screen}"
    );
}

/// A shipped stage holds no privileged *position* either. An extension at
/// order `300` runs after the shipped session package's `100`, so the frame
/// carries its mark while the persisted record does not: the shipped stage is
/// ordered, not final.
#[test]
fn a_configured_extension_orders_itself_after_the_shipped_session_stage() {
    let _fixture = Fixture::install("distribution-late");
    let distribution = distribution_with_extensions(
        &[(
            "mark-late.lua",
            marking_extension("acceptance.mark.late", " [late]", 300),
        )],
        "return { packages = { 'mark-late.lua' } }\n",
    );

    distribution.start_with_fixture("distribution-late", "plain");
    distribution.dispatch(json!({"kind": "input", "data": "compose\r"}));

    let screen = distribution.screen();
    assert!(
        screen.contains("pi   plain answer [late]"),
        "a stage after the shipped one did not reach the frame: {screen}"
    );
    assert_eq!(
        persisted_assistant_texts(&distribution),
        vec!["plain answer"],
        "the shipped session stage recorded a later stage's output"
    );
}

/// Retiring one extension is ordinary lifecycle cleanup, and it holds across
/// the whole graph: the configuration drops `mark-b.lua`, one public
/// `config_reload` disposes exactly that package, and its stage stops running.
/// `mark-a.lua` is *kept*, not reloaded — the configuration reconciles the live
/// generation against the selected one — so the surviving extension keeps its
/// registration and its place in the chain.
#[test]
fn a_retired_extension_stops_composing_after_a_reload() {
    let _fixture = Fixture::install("distribution-retired");
    let distribution = distribution_with_extensions(
        &[
            (
                "mark-a.lua",
                marking_extension("acceptance.mark.a", " [a]", 10),
            ),
            (
                "mark-b.lua",
                marking_extension("acceptance.mark.b", " [b]", 20),
            ),
        ],
        "return { packages = { 'mark-a.lua', 'mark-b.lua' } }\n",
    );

    distribution.start_with_fixture("distribution-retired", "plain");
    distribution.dispatch(json!({"kind": "input", "data": "first\r"}));

    distribution
        .sandbox
        .write_configuration("return { packages = { 'mark-a.lua' } }\n");
    distribution.dispatch(json!({"kind": "config_reload"}));
    distribution.dispatch(json!({"kind": "input", "data": "second\r"}));

    assert_eq!(
        persisted_assistant_texts(&distribution),
        vec!["plain answer [a] [b]", "plain answer [a]"],
        "a disposed extension's stage kept running, or a kept one lost its place"
    );
}

/// A package that refuses to load. Nothing in the distribution is special
/// about the failure: `packages.load` raises, and the configuration's reload
/// unwinds around it.
const BROKEN_EXTENSION: &str = "error('acceptance: this extension refuses to load')\n";

/// A failed reload rolls back to the previous generation and the distribution
/// keeps composing: the broken package leaves no stage behind, and the
/// extension that was already live is neither disposed nor reloaded.
#[test]
fn a_failed_reload_leaves_the_previous_composition_running() {
    let _fixture = Fixture::install("distribution-rollback");
    let distribution = distribution_with_extensions(
        &[(
            "mark-a.lua",
            marking_extension("acceptance.mark.a", " [a]", 10),
        )],
        "return { packages = { 'mark-a.lua' } }\n",
    );

    distribution.start_with_fixture("distribution-rollback", "plain");
    distribution.dispatch(json!({"kind": "input", "data": "first\r"}));

    distribution
        .sandbox
        .write_package("broken.lua", BROKEN_EXTENSION);
    distribution
        .sandbox
        .write_configuration("return { packages = { 'mark-a.lua', 'broken.lua' } }\n");
    distribution.dispatch(json!({"kind": "config_reload"}));
    distribution.dispatch(json!({"kind": "input", "data": "second\r"}));

    let screen = distribution.screen();
    assert!(
        screen.contains("idle"),
        "a refused reload left the distribution unusable: {screen}"
    );
    assert_eq!(
        persisted_assistant_texts(&distribution),
        vec!["plain answer [a]", "plain answer [a]"],
        "a refused reload changed the live composition"
    );
}

/// Two extensions claiming the same stage identity conflict deterministically,
/// and the outcome does not depend on which one the configuration lists first:
/// the host refuses the second registration by `kind/phase/id`, that package's
/// load fails, and the whole reload unwinds. Neither declared order composes,
/// so the conflict is decided by the identity, not by arrival.
#[test]
fn two_extensions_claiming_one_stage_identity_conflict_either_way() {
    for order in [["mark-a.lua", "clash.lua"], ["clash.lua", "mark-a.lua"]] {
        let _fixture = Fixture::install("distribution-conflict");
        let listed = format!(
            "return {{ packages = {{ '{}', '{}' }} }}\n",
            order[0], order[1]
        );
        let distribution = distribution_with_extensions(
            &[
                (
                    "mark-a.lua",
                    marking_extension("acceptance.mark.a", " [a]", 10),
                ),
                // Same kind, same phase, same id, different source.
                (
                    "clash.lua",
                    marking_extension("acceptance.mark.a", " [clash]", 20),
                ),
            ],
            &listed,
        );

        distribution.start_with_fixture("distribution-conflict", "plain");
        distribution.dispatch(json!({"kind": "input", "data": "compose\r"}));

        assert_eq!(
            persisted_assistant_texts(&distribution),
            vec!["plain answer"],
            "a conflicting pair composed anyway, listed as {order:?}"
        );
    }
}

/// A file-backed frontend root: the shipped application coordinator's public
/// contract and nothing else. It turns `input` into one `frontend_submit`
/// intent and renders the agent batch the coordinator hands back. Like the
/// replacement agent above, it registers *below* the shipped frontend's `0`.
const REPLACEMENT_FRONTEND: &str = r#"
local pi = ...
local roots = pi.roots.v1

roots.register({
  kind = "frontend",
  id = "acceptance.frontend",
  active = true,
  priority = -10,
  dispatch = function(snapshot)
    local event = snapshot.event
    local kind = type(event) == "table" and event.kind or nil
    if kind == "input" then
      local text = string.gsub(tostring(event.data or ""), "\r", "")
      roots.action("frontend_submit", { text = text })
      return
    end
    if kind == "agent" then
      for _, action in ipairs(event.actions or {}) do
        if action.kind == "agent_message" then
          local payload = action.payload or {}
          roots.action("ansi", {
            data = "replacement frontend: " .. tostring(payload.text or ""),
          })
        end
      end
      return
    end
  end,
})
"#;

/// The ANSI payloads one dispatch published, in order.
fn ansi(batch: &DispatchBatch) -> String {
    batch
        .actions
        .iter()
        .filter(|action| action.kind == "ansi")
        .filter_map(|action| action.payload.get("data").and_then(Value::as_str))
        .collect()
}

/// Two roots replaced at once, from one configuration, and the rest of the
/// distribution still composes over both. This is the claim package-level
/// replacement cannot make: the shipped index registers a root for every kind,
/// so replacing two of them at the same time is where a hidden dependency
/// between shipped packages would show up. Neither replacement shares a module
/// with the package it displaces, and both register below the shipped
/// priority, so only `roots` naming them resolves them.
#[test]
fn two_roots_are_replaced_simultaneously_and_the_rest_composes() {
    let sandbox = Sandbox::new();
    sandbox.write_package("agent.lua", REPLACEMENT_AGENT);
    sandbox.write_package("frontend.lua", REPLACEMENT_FRONTEND);
    sandbox.write_configuration(
        r#"
return {
  packages = { "agent.lua", "frontend.lua" },
  roots = { agent = "acceptance.agent", frontend = "acceptance.frontend" },
}
"#,
    );
    let distribution = Distribution::from_packages(
        sandbox,
        &manifest_packages(&pi_rs_builtins::manifest_path()),
    );

    distribution.dispatch(json!({"kind": "startup"}));
    let batch = distribution.dispatch(json!({"kind": "input", "data": "who answers?\r"}));

    // The replacement frontend rendered the replacement agent's message, so
    // both roots are live in the same turn.
    let painted = ansi(&batch);
    assert!(
        painted.contains("replacement frontend: replacement agent: who answers?"),
        "the two replacements did not compose: {painted}"
    );
    assert!(
        !painted.contains("enter send"),
        "the shipped frontend still painted its footer: {painted}"
    );
    assert!(
        !distribution.screen().contains("pi \u{b7} "),
        "the shipped frontend still painted its chrome"
    );

    // The shipped session package, untouched by either replacement, still
    // folds the batch through the same public action vocabulary.
    assert_eq!(
        persisted_assistant_texts(&distribution),
        vec!["replacement agent: who answers?"],
        "the shipped session package stopped recording across two replacements"
    );
    let status = distribution.session("status");
    assert_eq!(status["ok"], true, "session status refused: {status}");
    assert_eq!(status["session"]["messages"], 2);
}

/// An extension whose stage never returns once a prompt reaches the agent.
/// Everything before that is ordinary, so the package loads, registers, and
/// composes exactly like any other — which is the point: a runaway stage is
/// not detectable at load time.
const RUNAWAY_EXTENSION: &str = r#"
local pi = ...

pi.roots.v1.middleware.register({
  kind = "agent",
  phase = "event",
  id = "acceptance.runaway",
  order = -10,
  handler = function(snapshot)
    local event = snapshot.event
    if type(event) == "table" and event.kind == "prompt" then
      while true do end
    end
    return nil
  end,
})
"#;

/// One extension cannot hang the product. The runaway stage is bounded by the
/// per-dispatch watchdog, the dispatch that reached it fails by name, and the
/// distribution keeps answering every dispatch that does not go through that
/// stage — the shipped session command and the shipped frontend's repaint.
#[test]
fn a_runaway_extension_is_bounded_by_the_watchdog() {
    let sandbox = Sandbox::new();
    sandbox.write_package("runaway.lua", RUNAWAY_EXTENSION);
    sandbox.write_configuration("return { packages = { 'runaway.lua' } }\n");
    let distribution = Distribution::from_packages_with_timeout(
        sandbox,
        &manifest_packages(&pi_rs_builtins::manifest_path()),
        400,
    );

    distribution.dispatch(json!({"kind": "startup"}));
    let refused = distribution
        .try_dispatch(json!({"kind": "input", "data": "hang\r"}))
        .expect_err("a runaway stage must hit the watchdog");
    // The runaway stage sits under a *nested* root dispatch — the shipped
    // coordinator asking the agent root — so the watchdog's stop is raised
    // into the calling Lua frame rather than returned as the host-level
    // `Timeout` a top-level dispatch yields. Either way the bound is the same
    // watchdog and the cost is one refused dispatch.
    let message = refused.to_string();
    assert!(
        matches!(refused, HostError::Lua(_)) && message.contains("timed out (watchdog, 400ms"),
        "the watchdog did not bound the runaway stage: {refused:?}"
    );

    // The dispatch failed; the product did not.
    let status = distribution.session("status");
    assert_eq!(
        status["ok"], true,
        "the shipped session command stopped answering: {status}"
    );
    let screen = distribution.screen();
    assert!(
        screen.contains("pi \u{b7} "),
        "the shipped frontend stopped repainting: {screen}"
    );
}

/// An extension that consumes a *shipped* module at an explicit version.
///
/// It registers nothing. Its whole behaviour is at load time: require
/// `pi.config.paths` at `version`, then refuse to load unless the module that
/// came back is the real path policy. A wrong version therefore fails the
/// package, and a stub would fail it too.
fn shipped_module_consumer(version: &str) -> String {
    format!(
        r#"
local pi = ...
local module = pi.kernel.v1.module
local paths = module.require("pi.config.paths", "{version}")

local row = ((paths.resolve({{}}) or {{}}).resources or {{}})["sessions"]
if type(row) ~= "table" or type(row.destination) ~= "string" then
  error("pi.config.paths@{version} did not resolve a sessions destination")
end
"#
    )
}

/// A shipped module is versioned for an extension exactly like any other
/// module: requiring `pi.config.paths@1` hands over the real path policy, and
/// requiring a version nothing defines fails that package's load, which rolls
/// the whole reload back and takes the extension beside it with it. Neither
/// outcome depends on the module being shipped rather than file-backed.
#[test]
fn a_shipped_module_is_versioned_for_extensions_like_any_other() {
    for (version, expected) in [("1", vec!["plain answer [a]"]), ("2", vec!["plain answer"])] {
        let _fixture = Fixture::install("distribution-module");
        let distribution = distribution_with_extensions(
            &[
                (
                    "mark-a.lua",
                    marking_extension("acceptance.mark.a", " [a]", 10),
                ),
                ("uses-paths.lua", shipped_module_consumer(version)),
            ],
            "return { packages = { 'mark-a.lua', 'uses-paths.lua' } }\n",
        );

        distribution.start_with_fixture("distribution-module", "plain");
        distribution.dispatch(json!({"kind": "input", "data": "compose\r"}));

        assert_eq!(
            persisted_assistant_texts(&distribution),
            expected,
            "requiring pi.config.paths@{version} composed unexpectedly"
        );
    }
}
