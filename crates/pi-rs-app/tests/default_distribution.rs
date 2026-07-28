//! Acceptance for the default distribution: the shipped manifest, the
//! installed launcher journey, and the offline default coding journey.
//!
//! The distribution is one declarative manifest over ordinary file-backed Lua
//! packages. Nothing here is embedded, concatenated, or privileged: the same
//! files copied anywhere on disk must behave identically, and the raw
//! zero-package launcher must stay reachable.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

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
    for tree in ["agent", "tools", "frontend", "defaults"] {
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
// Installed launcher journey
// ---------------------------------------------------------------------------

/// Runs the built `pi` binary with an explicit manifest. stdin is not a
/// terminal here, so the launcher serializes the startup batch — the same
/// batch an interactive session presents as its first frame.
fn run_launcher(manifest: &Path, root: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_pi"))
        .arg("--root")
        .arg(root)
        .arg("--manifest")
        .arg(manifest)
        .env_remove("PI_PACKAGE_MANIFEST")
        .output()
        .expect("run pi");
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

#[test]
fn the_default_distribution_starts_input_ready() {
    let scratch = tempfile::tempdir().unwrap();
    let batch = run_launcher(&pi_rs_builtins::manifest_path(), scratch.path());
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
}

#[test]
fn the_default_distribution_selects_a_model_without_configuration() {
    let scratch = tempfile::tempdir().unwrap();
    let screen = frame(&run_launcher(
        &pi_rs_builtins::manifest_path(),
        scratch.path(),
    ));

    assert!(
        screen.contains("claude-sonnet-4-5"),
        "no default model in the header: {screen}"
    );
    assert!(
        !screen.contains("no model"),
        "default model not applied: {screen}"
    );
}

#[test]
fn every_shipped_source_copied_to_disk_behaves_identically() {
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("distribution");
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

    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let shipped = run_launcher(&manifest, &workspace);
    let copied = run_launcher(&copy.join("default.json"), &workspace);

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
/// inside one workspace directory.
struct Distribution {
    host: Host,
    _directory: tempfile::TempDir,
}

impl Distribution {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host = Host::new(HostConfig {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..HostConfig::default()
        })
        .unwrap();
        for path in manifest_packages(&pi_rs_builtins::manifest_path()) {
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
        }
        Self {
            host,
            _directory: directory,
        }
    }

    fn workspace(&self) -> &Path {
        self._directory.path()
    }

    /// The launcher publishes the resolved root on every application
    /// dispatch; the in-process journey mirrors that context exactly.
    fn dispatch(&self, event: Value) -> DispatchBatch {
        let context = json!({"root": self.workspace().to_string_lossy()});
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
