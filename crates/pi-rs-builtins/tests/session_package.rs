//! Deterministic acceptance for the shipped Lua session package.
//!
//! Every scenario drives the ordinary file-backed session package through the
//! public kernel transaction. The host contributes an immutable environment
//! snapshot, path arithmetic, bounded filesystem effects, the append-only
//! record store, and root middleware; the record schema, reconstruction,
//! naming, branch meaning, compaction, retention, and legacy rule all live in
//! the Lua under `crates/pi-rs-builtins/session/`.
//!
//! The matrices below are the PLAN 4.3 acceptance: suppression leaves the
//! ephemeral application untouched, a file-backed replacement persists a
//! different schema, and the branch, compact, resume, corruption,
//! stale-handle, and legacy-read/XDG-write paths are covered.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The session package in load order, plus the one path policy it asks for
/// its directories. `pi.config.paths@1` is an ordinary module identity from
/// the configuration package, listed here exactly as a distribution manifest
/// would list it.
fn package_files() -> Vec<PathBuf> {
    let root = pi_rs_builtins::package_root();
    vec![
        root.join("config").join("paths.lua"),
        root.join("session").join("records.lua"),
        root.join("session").join("store.lua"),
        root.join("session").join("init.lua"),
    ]
}

/// Stub roots that publish the shipped vocabularies without the shipped
/// policy: the agent root replays whatever action list the event carries, and
/// the application root reports anything the session stage did not answer.
/// Scenarios therefore state an agent batch literally instead of steering a
/// provider fixture; `shipped_agent_turn_is_persisted` proves the real agent
/// emits this same vocabulary.
const DRIVER: &str = r#"
local pi = ...
local roots = pi.roots.v1

local function clone(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, item in pairs(value) do
    out[key] = clone(item)
  end
  return out
end

roots.register({
  kind = "agent",
  id = "test.agent",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    for _, action in ipairs(snapshot.event.actions or {}) do
      roots.action(action.kind, clone(action.payload) or {})
    end
  end,
})

roots.register({
  kind = "application",
  id = "test.application",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    roots.action("unhandled", { kind = tostring(snapshot.event.kind) })
  end,
})
"#;

struct Fixture {
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("home")).unwrap();
        std::fs::create_dir_all(directory.path().join("state")).unwrap();
        Self { directory }
    }

    fn home(&self) -> PathBuf {
        self.directory.path().join("home")
    }

    fn state(&self) -> PathBuf {
        self.directory.path().join("state")
    }

    /// Canonical XDG destination: `$XDG_STATE_HOME/pi/sessions`.
    fn sessions(&self) -> PathBuf {
        self.state().join("pi").join("sessions")
    }

    /// Read-only inherited location: `$HOME/.pi/agent/sessions`.
    fn legacy(&self) -> PathBuf {
        self.home().join(".pi").join("agent").join("sessions")
    }

    fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("HOME".to_owned(), text(&self.home())),
            ("XDG_STATE_HOME".to_owned(), text(&self.state())),
        ])
    }

    fn start(&self, packages: &[&str]) -> Host {
        self.start_with(packages, HostConfig::default())
    }

    fn start_with(&self, packages: &[&str], base: HostConfig) -> Host {
        let host = Host::new(HostConfig {
            cwd: Some(text(self.directory.path())),
            environment: Some(self.environment()),
            ..base
        })
        .unwrap();
        for path in package_files() {
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
        }
        for (index, source) in packages.iter().enumerate() {
            let path = self.directory.path().join(format!("driver-{index}.lua"));
            std::fs::write(&path, source).unwrap();
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load driver {index}: {error}"));
        }
        host
    }

    /// A host with the driver roots but *without* the session package: the
    /// suppression and replacement scenarios.
    fn start_bare(&self, packages: &[&str]) -> Host {
        let host = Host::new(HostConfig {
            cwd: Some(text(self.directory.path())),
            environment: Some(self.environment()),
            ..HostConfig::default()
        })
        .unwrap();
        for (index, source) in packages.iter().enumerate() {
            let path = self.directory.path().join(format!("bare-{index}.lua"));
            std::fs::write(&path, source).unwrap();
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load bare package {index}: {error}"));
        }
        host
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn turn(host: &Host, actions: Value) -> DispatchBatch {
    host.dispatch(DispatchRequest::new(
        RootKind::Agent,
        json!({ "kind": "turn", "actions": actions }),
        Value::Null,
    ))
    .unwrap_or_else(|error| panic!("agent dispatch failed: {error}"))
}

fn command(host: &Host, event: Value) -> Value {
    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            event,
            Value::Null,
        ))
        .unwrap_or_else(|error| panic!("application dispatch failed: {error}"));
    let action = batch
        .actions
        .first()
        .unwrap_or_else(|| panic!("session command published no action"));
    json!({ "kind": action.kind, "payload": action.payload })
}

fn session(host: &Host, command_name: &str) -> Value {
    command(host, json!({ "kind": "session", "command": command_name }))["payload"].clone()
}

/// An empty Lua table converts to a JSON object, not an array, so every list
/// this package returns is read through one tolerant accessor.
fn rows(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

fn ok(payload: &Value) -> bool {
    payload["ok"] == json!(true)
}

fn action_list(actions: &[(&str, Value)]) -> Value {
    Value::Array(
        actions
            .iter()
            .map(|(kind, payload)| json!({ "kind": kind, "payload": payload }))
            .collect(),
    )
}

/// One user turn in the shipped agent's public action vocabulary.
fn text_turn(prompt: &str, reply: &str) -> Value {
    action_list(&[
        ("agent_turn_start", json!({ "prompt": prompt })),
        ("agent_status", json!({ "state": "streaming" })),
        (
            "agent_message",
            json!({ "text": reply, "stop_reason": "stop" }),
        ),
        ("agent_status", json!({ "state": "idle" })),
    ])
}

/// Every `*.jsonl` store in a directory, sorted by name.
fn stores(directory: &Path) -> Vec<PathBuf> {
    if !directory.exists() {
        return Vec::new();
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path
                .extension()
                .is_some_and(|extension| extension == "jsonl"))
            .then_some(path)
        })
        .collect();
    found.sort();
    found
}

/// The appended record values of one store, in append order. The framing
/// (`version`, `sequence`, `checksum`) belongs to the generic record store;
/// everything inside `value` is the session package's own schema.
fn records(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap_or_else(|error| panic!("{line}: {error}"))
                ["value"]
                .clone()
        })
        .collect()
}

fn only_store(directory: &Path) -> Vec<Value> {
    let found = stores(directory);
    assert_eq!(found.len(), 1, "expected one store in {directory:?}");
    records(&found[0])
}

fn kinds(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .map(|value| value["kind"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn conversation(payload: &Value) -> Vec<String> {
    assert!(ok(payload), "{payload}");
    rows(&payload["conversation"])
        .iter()
        .map(|message| {
            format!(
                "{}:{}",
                message["role"].as_str().unwrap_or_default(),
                message["text"].as_str().unwrap_or_default()
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Suppression and replacement
// ---------------------------------------------------------------------------

/// Without the session package nothing is persisted, no directory is created,
/// and a `session` event reaches the application root as an ordinary unhandled
/// event: 3.6's useful ephemeral application, unchanged.
#[test]
fn suppressing_the_package_leaves_the_ephemeral_application() {
    let fixture = Fixture::new();
    let host = fixture.start_bare(&[DRIVER]);

    let batch = turn(&host, text_turn("hello", "hi"));
    assert_eq!(
        batch
            .actions
            .iter()
            .map(|action| action.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            "agent_turn_start",
            "agent_status",
            "agent_message",
            "agent_status"
        ]
    );

    let answered = command(&host, json!({ "kind": "session", "command": "status" }));
    assert_eq!(answered["kind"], "unhandled");
    assert_eq!(answered["payload"]["kind"], "session");

    assert!(!fixture.sessions().exists(), "a suppressed package wrote");
    assert!(!fixture.state().join("pi").exists());
}

/// The recording seam is public: a small file-backed package registers the
/// same `agent`/`render` stage and persists its own schema at its own
/// destination, with no shipped session package loaded.
#[test]
fn a_file_backed_replacement_persists_a_different_schema() {
    let fixture = Fixture::new();
    let destination = fixture.directory.path().join("replacement");
    let replacement = format!(
        r#"
local pi = ...
local records = pi.records.v1
local middleware = pi.roots.v1.middleware
local store = nil

middleware.register({{
  kind = "agent",
  phase = "render",
  id = "example.transcript",
  order = 100,
  handler = function(snapshot)
    if store == nil then
      store = records.create({{ directory = "{directory}", name = "transcript" }})
    end
    local lines = {{}}
    for _, action in ipairs(snapshot.actions) do
      lines[#lines + 1] = action.kind
    end
    store:append({{ batch = table.concat(lines, ","), schema = "example/1" }})
    return nil
  end,
}})
"#,
        directory = text(&destination)
    );

    let host = fixture.start_bare(&[DRIVER, &replacement]);
    turn(&host, text_turn("hello", "hi"));

    let written = only_store(&destination);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0]["schema"], "example/1");
    assert_eq!(
        written[0]["batch"],
        "agent_turn_start,agent_status,agent_message,agent_status"
    );
    assert!(
        !fixture.sessions().exists(),
        "the shipped destination was used"
    );
}

// ---------------------------------------------------------------------------
// Recording and reconstruction
// ---------------------------------------------------------------------------

/// A first turn creates one log under the canonical XDG state entry, and only
/// there: the legacy directory is never created, let alone written.
#[test]
fn a_turn_persists_the_conversation_under_xdg() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);

    turn(&host, text_turn("hello", "hi there"));

    let written = only_store(&fixture.sessions());
    assert_eq!(kinds(&written), vec!["header", "message", "message"]);
    assert_eq!(written[0]["schema"], 1);
    assert_eq!(written[1]["role"], "user");
    assert_eq!(written[1]["text"], "hello");
    assert_eq!(written[2]["role"], "assistant");
    assert_eq!(written[2]["text"], "hi there");
    assert!(
        !fixture.legacy().exists(),
        "the legacy directory was created"
    );

    let status = session(&host, "status");
    assert!(ok(&status), "{status}");
    assert_eq!(status["session"]["messages"], 2);
    assert_eq!(status["source"], "absent");
    assert_eq!(status["directory"], text(&fixture.sessions()));
}

/// Tool settlement is persisted from the same public vocabulary: the tool
/// result keeps its call id, tool name, and success flag.
#[test]
fn tool_results_and_notes_are_recorded() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);

    turn(
        &host,
        action_list(&[
            ("agent_configured", json!({ "model": "fixture/tools" })),
            ("agent_turn_start", json!({ "prompt": "read it" })),
            ("agent_message", json!({ "text": "", "tool_calls": 1 })),
            (
                "agent_tool_result",
                json!({ "id": "call-1", "name": "read", "ok": true, "output": "contents" }),
            ),
            ("agent_message", json!({ "text": "done" })),
            ("agent_error", json!({ "reason": "provider error" })),
        ]),
    );

    let written = only_store(&fixture.sessions());
    assert_eq!(
        kinds(&written),
        vec![
            "header", "model", "message", "message", "message", "message", "note"
        ]
    );
    let tool = &written[4];
    assert_eq!(tool["role"], "tool");
    assert_eq!(tool["call_id"], "call-1");
    assert_eq!(tool["name"], "read");
    assert_eq!(tool["ok"], true);
    assert_eq!(tool["text"], "contents");
    assert_eq!(written[6]["topic"], "error");

    // Notes are provenance, not conversation.
    let described = session(&host, "status");
    assert_eq!(described["session"]["messages"], 4);
    assert_eq!(described["session"]["model"], "fixture/tools");
}

/// The shipped agent, driven by a fixture provider, emits exactly the actions
/// this package records. Without this scenario the stub driver above would be
/// testing itself.
#[test]
fn shipped_agent_turn_is_persisted() {
    let _fixture_api = FixtureApi::install("session-fixture");
    let fixture = Fixture::new();
    let host = Host::new(HostConfig {
        cwd: Some(text(fixture.directory.path())),
        environment: Some(fixture.environment()),
        ..HostConfig::default()
    })
    .unwrap();
    let root = pi_rs_builtins::package_root();
    let mut files = package_files();
    for file in [
        "queue.lua",
        "tools.lua",
        "credentials.lua",
        "turn.lua",
        "init.lua",
    ] {
        files.push(root.join("agent").join(file));
    }
    for path in files {
        host.load_package(PackageSource::File { path: &path })
            .unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
    }

    host.dispatch(DispatchRequest::new(
        RootKind::Agent,
        json!({
            "kind": "prompt",
            "text": "hello",
            "model": {
                "id": "text",
                "name": "text",
                "api": "session-fixture",
                "provider": "fixture",
                "baseUrl": "http://127.0.0.1:1",
                "reasoning": false,
                "input": ["text"],
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                "contextWindow": 4096,
                "maxTokens": 128,
            },
        }),
        Value::Null,
    ))
    .expect("shipped agent dispatch");

    let written = only_store(&fixture.sessions());
    assert_eq!(kinds(&written), vec!["header", "message", "message"]);
    assert_eq!(written[1]["text"], "hello");
    assert_eq!(written[2]["text"], "recorded reply");
}

/// A resumed log reconstructs the conversation, and appending continues in the
/// same file rather than starting a second one.
#[test]
fn resume_reconstructs_and_continues_the_same_log() {
    let fixture = Fixture::new();
    let first = fixture.start(&[DRIVER]);
    turn(&first, text_turn("first", "one"));
    let id = session(&first, "status")["session"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(first);

    let host = fixture.start(&[DRIVER]);
    let resumed = command(
        &host,
        json!({ "kind": "session", "command": "resume", "id": id }),
    )["payload"]
        .clone();
    assert!(ok(&resumed), "{resumed}");
    assert_eq!(resumed["messages"], 2);
    assert_eq!(resumed["origin"], "canonical");

    turn(&host, text_turn("second", "two"));
    assert_eq!(stores(&fixture.sessions()).len(), 1);

    let described = command(
        &host,
        json!({ "kind": "session", "command": "describe", "id": id }),
    )["payload"]
        .clone();
    assert_eq!(
        conversation(&described),
        vec![
            "user:first",
            "assistant:one",
            "user:second",
            "assistant:two"
        ]
    );
}

/// A reset conversation is a different conversation: the log ends and the next
/// turn opens a new one, so a resumed log is never a mix of two.
#[test]
fn reset_ends_the_log_and_the_next_turn_opens_another() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);

    turn(&host, text_turn("first", "one"));
    turn(&host, action_list(&[("agent_reset", json!({}))]));
    turn(&host, text_turn("second", "two"));

    let found = stores(&fixture.sessions());
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        kinds(&records(&found[0])),
        vec!["header", "message", "message"]
    );
    assert_eq!(records(&found[1])[1]["text"], "second");
}

// ---------------------------------------------------------------------------
// Naming, compaction, branching, retention
// ---------------------------------------------------------------------------

/// A name is an appended record, so renaming is durable and reconstructed by
/// the same fold that rebuilds the conversation.
#[test]
fn naming_and_compaction_are_appended_records() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);
    turn(&host, text_turn("first", "one"));
    turn(&host, text_turn("second", "two"));

    let named = command(
        &host,
        json!({ "kind": "session", "command": "name", "title": "refactor" }),
    )["payload"]
        .clone();
    assert!(ok(&named), "{named}");
    assert_eq!(named["title"], "refactor");

    let compacted = command(
        &host,
        json!({
            "kind": "session",
            "command": "compact",
            "through": 3,
            "summary": "we discussed one and two",
        }),
    )["payload"]
        .clone();
    assert!(ok(&compacted), "{compacted}");
    // Three messages collapse into one summary, the fourth survives.
    assert_eq!(compacted["messages"], 2);
    assert_eq!(compacted["compactions"], 1);

    let written = only_store(&fixture.sessions());
    assert_eq!(
        kinds(&written),
        vec![
            "header",
            "message",
            "message",
            "message",
            "message",
            "title",
            "compaction"
        ]
    );

    let id = compacted["id"].as_str().unwrap().to_owned();
    let described = command(
        &host,
        json!({ "kind": "session", "command": "describe", "id": id }),
    )["payload"]
        .clone();
    assert_eq!(
        conversation(&described),
        vec!["user:we discussed one and two", "assistant:two"]
    );
    assert_eq!(described["title"], "refactor");
}

/// A compaction that covers no live message is refused rather than silently
/// truncating the conversation.
#[test]
fn an_impossible_compaction_is_refused() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);
    turn(&host, text_turn("first", "one"));

    let refused = command(
        &host,
        json!({
            "kind": "session",
            "command": "compact",
            "through": 9,
            "summary": "everything",
        }),
    )["payload"]
        .clone();
    assert!(!ok(&refused), "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("2 live messages"),
        "{refused}"
    );

    // The refusal changed nothing on disk.
    assert_eq!(
        kinds(&only_store(&fixture.sessions())),
        vec!["header", "message", "message"]
    );
}

/// Branching copies a prefix of the live log into a second store and
/// re-identifies the copy, so the parent stays exactly as it was and the
/// branch keeps writing where the fork happened.
#[test]
fn branching_copies_a_prefix_and_records_its_parent() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);
    turn(&host, text_turn("first", "one"));
    let parent = session(&host, "status")["session"].clone();
    let parent_id = parent["id"].as_str().unwrap().to_owned();

    let branched = command(
        &host,
        json!({ "kind": "session", "command": "branch", "id": "branch-a" }),
    )["payload"]
        .clone();
    assert!(ok(&branched), "{branched}");
    assert_eq!(branched["id"], "branch-a");
    assert_eq!(branched["parent"]["id"], parent_id.as_str());
    assert_eq!(branched["parent"]["records"], 3);

    // Writing continues in the branch; the parent is untouched.
    turn(&host, text_turn("second", "two"));
    let parent_records = records(&fixture.sessions().join(format!("{parent_id}.jsonl")));
    assert_eq!(kinds(&parent_records), vec!["header", "message", "message"]);

    let branch_records = records(&fixture.sessions().join("branch-a.jsonl"));
    assert_eq!(
        kinds(&branch_records),
        vec![
            "header", "message", "message", "branch", "message", "message"
        ]
    );

    let described = command(
        &host,
        json!({ "kind": "session", "command": "describe", "id": "branch-a" }),
    )["payload"]
        .clone();
    assert_eq!(
        conversation(&described),
        vec![
            "user:first",
            "assistant:one",
            "user:second",
            "assistant:two"
        ]
    );
    assert_eq!(described["parent"]["id"], parent_id.as_str());
}

/// Retention removes the oldest canonical logs, always keeps the live one, and
/// never touches the legacy directory.
#[test]
fn retention_keeps_the_newest_and_never_touches_legacy() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.legacy()).unwrap();
    let host = fixture.start(&[DRIVER]);

    // Three finished logs plus one live log.
    for index in 0..3 {
        turn(&host, text_turn(&format!("turn {index}"), "reply"));
        turn(&host, action_list(&[("agent_reset", json!({}))]));
    }
    turn(&host, text_turn("live", "reply"));
    let live = session(&host, "status")["session"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // A legacy log the retention rule must ignore.
    let legacy_store = fixture.legacy().join("inherited.jsonl");
    std::fs::write(
        &legacy_store,
        "{\"format\":\"pi-rs-records\",\"version\":1}\n",
    )
    .unwrap();

    assert_eq!(stores(&fixture.sessions()).len(), 4);
    let retained = command(
        &host,
        json!({ "kind": "session", "command": "retain", "keep": 2 }),
    )["payload"]
        .clone();
    assert!(ok(&retained), "{retained}");
    assert_eq!(rows(&retained["removed"]).len(), 2, "{retained}");

    let kept = stores(&fixture.sessions());
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert!(
        kept.iter()
            .any(|path| path.file_stem().unwrap() == live.as_str()),
        "the live log was removed: {kept:?}"
    );
    assert!(legacy_store.exists(), "retention removed a legacy log");
}

// ---------------------------------------------------------------------------
// Legacy, corruption, stale handles
// ---------------------------------------------------------------------------

/// A log that exists only under `~/.pi/agent/sessions` is read, copied forward
/// into the canonical XDG entry, and continued there. The inherited file is
/// left byte-for-byte as it was.
#[test]
fn a_legacy_log_is_read_and_promoted_to_xdg() {
    let fixture = Fixture::new();

    // Produce a real log, then move it to the legacy location so the fixture
    // does not hand-write a format the package owns.
    let seed = fixture.start(&[DRIVER]);
    turn(&seed, text_turn("inherited", "reply"));
    let id = session(&seed, "status")["session"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(seed);
    std::fs::create_dir_all(fixture.legacy()).unwrap();
    let legacy_store = fixture.legacy().join(format!("{id}.jsonl"));
    std::fs::rename(
        fixture.sessions().join(format!("{id}.jsonl")),
        &legacy_store,
    )
    .unwrap();
    // The lock sidecar belongs to the store; a listing takes a shared lock on
    // it, so an inherited directory carries both files.
    std::fs::rename(
        fixture.sessions().join(format!("{id}.jsonl.lock")),
        fixture.legacy().join(format!("{id}.jsonl.lock")),
    )
    .unwrap();
    let before = std::fs::read(&legacy_store).unwrap();

    let host = fixture.start(&[DRIVER]);
    let listed = session(&host, "list");
    assert!(ok(&listed), "{listed}");
    let listed_rows = rows(&listed["sessions"]);
    assert_eq!(listed_rows.len(), 1, "{listed}");
    assert_eq!(listed_rows[0]["origin"], "legacy");
    assert_eq!(listed_rows[0]["id"], id.as_str());

    let resumed = command(
        &host,
        json!({ "kind": "session", "command": "resume", "id": id }),
    )["payload"]
        .clone();
    assert!(ok(&resumed), "{resumed}");
    assert_eq!(resumed["origin"], "promoted");
    assert_eq!(resumed["messages"], 2);
    assert_eq!(
        resumed["path"],
        text(&fixture.sessions().join(format!("{id}.jsonl")))
    );

    turn(&host, text_turn("after", "promotion"));
    let promoted = records(&fixture.sessions().join(format!("{id}.jsonl")));
    assert_eq!(
        kinds(&promoted),
        vec!["header", "message", "message", "note", "message", "message"]
    );
    assert_eq!(promoted[3]["topic"], "promoted");
    assert_eq!(
        std::fs::read(&legacy_store).unwrap(),
        before,
        "the legacy log was written"
    );
}

/// A torn tail — the classic crash between `write` and `sync` — is named, not
/// hidden: the listing reports it, resuming refuses, and a new turn still gets
/// a working log.
#[test]
fn a_torn_log_is_diagnosed_and_recording_recovers() {
    let fixture = Fixture::new();
    let seed = fixture.start(&[DRIVER]);
    turn(&seed, text_turn("first", "one"));
    let id = session(&seed, "status")["session"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(seed);

    let path = fixture.sessions().join(format!("{id}.jsonl"));
    let mut contents = std::fs::read(&path).unwrap();
    contents.truncate(contents.len() - 12);
    std::fs::write(&path, &contents).unwrap();

    let host = fixture.start(&[DRIVER]);
    let listed = session(&host, "list");
    let diagnostics = rows(&listed["diagnostics"]);
    assert_eq!(diagnostics.len(), 1, "{listed}");
    assert_eq!(diagnostics[0]["kind"], "partial-write");
    assert_eq!(diagnostics[0]["origin"], "canonical");
    assert!(rows(&listed["sessions"]).is_empty(), "{listed}");

    let refused = command(
        &host,
        json!({ "kind": "session", "command": "resume", "id": id }),
    )["payload"]
        .clone();
    assert!(!ok(&refused), "{refused}");

    turn(&host, text_turn("after", "the crash"));
    let healthy = stores(&fixture.sessions())
        .into_iter()
        .find(|candidate| candidate != &path)
        .expect("a new log after the torn one");
    assert_eq!(
        kinds(&records(&healthy)),
        vec!["header", "message", "message"]
    );
}

/// A foreign log — a record store this package did not write — folds into
/// whatever it does contain and says why, instead of raising or pretending the
/// file is empty.
#[test]
fn a_foreign_log_folds_with_diagnostics() {
    let fixture = Fixture::new();
    let foreign = format!(
        r#"
local pi = ...
local records = pi.records.v1
local store = records.create({{ directory = "{directory}", name = "foreign" }})
store:append({{ kind = "note-from-elsewhere", value = 1 }})
store:append({{ kind = "message", role = "user", text = "still readable" }})
store:append({{ shape = "not a record at all" }})
store:close()
"#,
        directory = text(&fixture.sessions())
    );

    let host = fixture.start(&[DRIVER, &foreign]);
    let described = command(
        &host,
        json!({ "kind": "session", "command": "describe", "id": "foreign" }),
    )["payload"]
        .clone();
    assert!(ok(&described), "{described}");
    assert_eq!(conversation(&described), vec!["user:still readable"]);
    assert_eq!(described["unknown"], 2);
    let diagnostics = rows(&described["diagnostics"]);
    assert_eq!(diagnostics.len(), 3, "{described}");
    assert!(
        diagnostics[0]
            .as_str()
            .unwrap()
            .contains("no session header"),
        "{described}"
    );
}

/// Closing the live session leaves a stale handle behind. The next recorded
/// batch refuses to use it, drops it, and starts a fresh log rather than
/// failing the agent turn.
#[test]
fn a_stale_handle_is_dropped_and_recording_continues() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);
    turn(&host, text_turn("first", "one"));

    let closed = session(&host, "close");
    assert!(ok(&closed), "{closed}");
    let idle = session(&host, "status");
    assert!(idle["session"].is_null(), "{idle}");

    // The agent turn still settles normally.
    let batch = turn(&host, text_turn("second", "two"));
    assert_eq!(batch.actions.len(), 4);
    let found = stores(&fixture.sessions());
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(records(&found[1])[1]["text"], "second");
}

/// Disposing the package releases the operating-system lock the live log
/// holds, so another process can open the same file immediately; the disposal
/// path is the record store's scope resource, not Lua garbage collection.
#[test]
fn disposing_the_package_releases_the_live_log() {
    let fixture = Fixture::new();
    let host = Host::new(HostConfig {
        cwd: Some(text(fixture.directory.path())),
        environment: Some(fixture.environment()),
        ..HostConfig::default()
    })
    .unwrap();
    let mut handles = Vec::new();
    for path in package_files() {
        handles.push(
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {}: {error}", path.display())),
        );
    }
    let driver = fixture.directory.path().join("driver.lua");
    std::fs::write(&driver, DRIVER).unwrap();
    host.load_package(PackageSource::File { path: &driver })
        .unwrap();

    turn(&host, text_turn("first", "one"));
    let path = stores(&fixture.sessions()).remove(0);

    // While the package is alive the log is locked against a second host.
    let second = fixture.start(&[DRIVER]);
    let blocked = command(
        &second,
        json!({
            "kind": "session",
            "command": "resume",
            "id": path.file_stem().unwrap().to_string_lossy(),
        }),
    )["payload"]
        .clone();
    assert!(!ok(&blocked), "{blocked}");
    assert!(
        blocked["error"].as_str().unwrap().contains("locked"),
        "{blocked}"
    );

    // Disposing the session package runs the store's disposer.
    host.dispose_package(handles.last().unwrap()).unwrap();
    let allowed = command(
        &second,
        json!({
            "kind": "session",
            "command": "resume",
            "id": path.file_stem().unwrap().to_string_lossy(),
        }),
    )["payload"]
        .clone();
    assert!(ok(&allowed), "{allowed}");
    assert_eq!(allowed["messages"], 2);
}

/// Every command is fail-closed: an unknown one is refused by name and nothing
/// is written.
#[test]
fn an_unknown_command_is_refused_by_name() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);

    let refused = session(&host, "teleport");
    assert!(!ok(&refused), "{refused}");
    assert_eq!(refused["command"], "teleport");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("unknown session command 'teleport'"),
        "{refused}"
    );
    assert!(!fixture.sessions().exists());
}

// ---------------------------------------------------------------------------
// Fixture provider for the shipped-agent scenario
// ---------------------------------------------------------------------------

use pi_rs_ai::registry::{ApiProvider, register_api_provider, unregister_api_providers};
use pi_rs_ai::transport::create_assistant_message_event_stream;
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, StopReason,
    TextContent, Usage, now_ms,
};

struct FixtureApi {
    owner: String,
}

impl FixtureApi {
    fn install(api: &str) -> Self {
        let owner = format!("pi-rs-builtins-session-{api}");
        register_api_provider(
            ApiProvider {
                api: api.to_owned(),
                stream: std::sync::Arc::new(|model, _, _| Ok(reply(model))),
                stream_simple: std::sync::Arc::new(|model, _, _| Ok(reply(model))),
            },
            Some(&owner),
        );
        Self { owner }
    }
}

impl Drop for FixtureApi {
    fn drop(&mut self) {
        unregister_api_providers(&self.owner);
    }
}

fn reply(model: &pi_rs_ai_types::Model) -> pi_rs_ai::transport::AssistantMessageEventStream {
    let message = AssistantMessage {
        role: AssistantRole::Assistant,
        content: vec![AssistantContent::Text(TextContent::new("recorded reply"))],
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_model: None,
        response_id: None,
        diagnostics: None,
        usage: Usage::default(),
        stop_reason: StopReason::Stop,
        error_message: None,
        timestamp: now_ms(),
    };
    let stream = create_assistant_message_event_stream();
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

/// A tool can emit far more output than the record store accepts in one
/// record, so text is truncated to the package's declared budget and flagged.
/// Losing the tail of one turn is recoverable; a refused append would lose the
/// turn itself.
#[test]
fn oversized_text_is_truncated_rather_than_refused() {
    let fixture = Fixture::new();
    let host = fixture.start(&[DRIVER]);
    let huge = "x".repeat(64 * 1024);

    turn(
        &host,
        action_list(&[
            ("agent_turn_start", json!({ "prompt": "dump it" })),
            (
                "agent_tool_result",
                json!({ "id": "call-1", "name": "read", "ok": true, "output": huge }),
            ),
        ]),
    );

    let written = only_store(&fixture.sessions());
    assert_eq!(kinds(&written), vec!["header", "message", "message"]);
    let text = written[2]["text"].as_str().unwrap();
    assert!(text.len() < 17 * 1024, "{} bytes stored", text.len());
    assert!(
        text.ends_with("[truncated]"),
        "{}",
        &text[text.len() - 32..]
    );
    assert_eq!(written[2]["truncated"], true);
}

/// Persistence is optional, so it must never be able to fail a turn: with no
/// usable state root the agent batch still settles untouched and the refusal is
/// reported only when the session is asked about.
#[test]
fn an_unusable_state_root_does_not_fail_the_turn() {
    let fixture = Fixture::new();
    let host = Host::new(HostConfig {
        cwd: Some(text(fixture.directory.path())),
        // No HOME and no XDG_STATE_HOME: the path policy reports the state
        // class unavailable rather than falling back to the working directory.
        environment: Some(BTreeMap::new()),
        ..HostConfig::default()
    })
    .unwrap();
    for path in package_files() {
        host.load_package(PackageSource::File { path: &path })
            .unwrap();
    }
    let driver = fixture.directory.path().join("driver.lua");
    std::fs::write(&driver, DRIVER).unwrap();
    host.load_package(PackageSource::File { path: &driver })
        .unwrap();

    let batch = turn(&host, text_turn("hello", "hi"));
    assert_eq!(batch.actions.len(), 4);

    let status = session(&host, "status");
    assert!(ok(&status), "{status}");
    assert!(status["session"].is_null(), "{status}");
    assert_eq!(status["error"], "no usable state root for sessions");
    assert!(stores(fixture.directory.path()).is_empty());
}
