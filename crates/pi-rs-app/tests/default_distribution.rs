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
use pi_rs_host::{Host, HostConfig, PackageSource};
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
        let host = Host::new(HostConfig {
            cwd: Some(text(&sandbox.workspace())),
            environment: Some(sandbox.environment()),
            ..HostConfig::default()
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
    fn dispatch(&self, event: Value) -> DispatchBatch {
        let context = json!({"root": text(&self.workspace())});
        self.host
            .dispatch(DispatchRequest::new(RootKind::Application, event, context))
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
