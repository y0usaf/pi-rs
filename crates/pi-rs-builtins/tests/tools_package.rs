//! Deterministic acceptance for the shipped Lua core-tool package.
//!
//! Every scenario drives the ordinary file-backed tool package through the
//! public kernel transaction: a driver package looks tools up in the one tool
//! declaration path and executes them. No product logic and no privileged
//! executor lives in Rust; `read`, `write`, `edit`, and `bash` are the Lua
//! under `crates/pi-rs-builtins/tools/`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The tool package needs only the tool declaration path from the agent
/// package; turn policy and the agent root stay out of these scenarios.
fn package_files() -> Vec<std::path::PathBuf> {
    let root = pi_rs_builtins::package_root();
    let mut files = vec![root.join("agent").join("tools.lua")];
    for file in [
        "paths.lua",
        "render.lua",
        "locks.lua",
        "read.lua",
        "write.lua",
        "edit.lua",
        "bash.lua",
        "init.lua",
    ] {
        files.push(root.join("tools").join(file));
    }
    files
}

/// Driver root: executes one declared tool per dispatch and republishes the
/// result as data. `call_while_locked` holds the mutation lock first, so the
/// serialization policy is observable without a second host thread.
///
/// The public filesystem effect resolves relative paths against the host
/// process directory, so the driver redeclares the shipped tools with an
/// explicit workspace root — the same knob a distribution or a session
/// package uses.
const DRIVER: &str = r#"
local pi = ...
local roots = pi.roots.v1
local module = pi.kernel.v1.module
local registry = module.require("pi.agent.tools", "1")
local suite = module.require("pi.tools.suite", "1")
local locks = module.require("pi.tools.locks", "1")
local root = __ROOT__

suite.unregister(registry)
suite.declare(registry, { shared = { root = root } })

-- Configured variants prove per-tool settings and leave the four shipped
-- declarations in place.
suite.declare(registry, {
  suppress = { read = true, edit = true },
  tools = {
    write = { name = "write_small", root = root, max_bytes = 64, wait_ms = 0 },
    bash = { name = "bash_small", root = root, max_output_bytes = 256 },
  },
})

local function copy(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, item in pairs(value) do
    out[key] = copy(item)
  end
  return out
end

local function run(name, arguments)
  local entry = registry.find(name)
  if entry == nil then
    roots.action("tool_missing", { name = name })
    return
  end
  local ok, result = pcall(entry.execute, {
    id = "call-1",
    name = name,
    arguments = copy(arguments) or {},
  })
  if not ok then
    roots.action("tool_raised", { name = name, error = tostring(result) })
    return
  end
  if type(result) ~= "table" then
    result = { output = tostring(result), is_error = false }
  end
  roots.action("tool_result", {
    name = name,
    output = result.output,
    is_error = result.is_error == true,
    details = result.details,
    serialize = entry.serialize,
  })
end

roots.register({
  kind = "application",
  id = "tools-driver",
  dispatch = function(snapshot)
    local event = snapshot.event
    if event.kind == "call" then
      run(event.name, event.arguments)
    elseif event.kind == "call_while_locked" then
      local token = locks.acquire(root .. "/" .. event.lock)
      run(event.name, event.arguments)
      locks.release(token)
    elseif event.kind == "declarations" then
      local names = {}
      for index, entry in ipairs(registry.list()) do
        names[index] = { name = entry.name, serialize = entry.serialize, owner = entry.owner }
      end
      roots.action("declarations", {
        tools = names,
        wire = registry.declarations(),
        suite = suite.names(),
      })
    else
      roots.action("unknown_event", { kind = tostring(event.kind) })
    end
  end,
})
"#;

/// Cancellation source under test: the pre-start probe answers "live" and the
/// first output probe answers "cancelled", so the tool aborts mid-command.
const CANCELLING_BASH: &str = r#"
local pi = ...
local module = pi.kernel.v1.module
local registry = module.require("pi.agent.tools", "1")
local bash = module.require("pi.tools.bash", "1")

local probes = 0
bash.declare(registry, {
  name = "bash_cancelling",
  cancelled = function()
    probes = probes + 1
    return probes > 1
  end,
})
"#;

struct Harness {
    host: Host,
    directory: tempfile::TempDir,
}

impl Harness {
    fn new(extra: &[(&str, &str)]) -> Self {
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
        let driver = directory.path().join("driver.lua");
        let source = DRIVER.replace(
            "__ROOT__",
            &serde_json::to_string(&directory.path().to_string_lossy().into_owned()).unwrap(),
        );
        std::fs::write(&driver, source).unwrap();
        host.load_package(PackageSource::File { path: &driver })
            .unwrap_or_else(|error| panic!("load driver: {error}"));
        for (name, source) in extra {
            let path = directory.path().join(name);
            std::fs::write(&path, source).unwrap();
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {name}: {error}"));
        }
        Self { host, directory }
    }

    fn dispatch(&self, event: Value) -> DispatchBatch {
        self.host
            .dispatch(DispatchRequest::new(
                RootKind::Application,
                event,
                Value::Null,
            ))
            .unwrap_or_else(|error| panic!("driver dispatch failed: {error}"))
    }

    fn call(&self, name: &str, arguments: Value) -> Value {
        let batch = self.dispatch(json!({
            "kind": "call",
            "name": name,
            "arguments": arguments,
        }));
        let action = batch
            .actions
            .first()
            .unwrap_or_else(|| panic!("{name} produced no action"));
        assert_eq!(
            action.kind, "tool_result",
            "unexpected {:?}",
            action.payload
        );
        action.payload.clone()
    }

    fn seed(&self, name: &str, contents: &str) {
        std::fs::write(self.directory.path().join(name), contents).unwrap();
    }

    fn contents(&self, name: &str) -> String {
        std::fs::read_to_string(self.directory.path().join(name)).unwrap()
    }

    fn exists(&self, name: &str) -> bool {
        self.directory.path().join(name).exists()
    }
}

fn output(result: &Value) -> &str {
    result["output"].as_str().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

#[test]
fn read_returns_numbered_lines_and_metadata() {
    let harness = Harness::new(&[]);
    harness.seed("notes.txt", "alpha\nbeta\ngamma\n");

    let result = harness.call("read", json!({"path": "notes.txt"}));

    assert_eq!(result["is_error"], false, "{result:?}");
    assert_eq!(output(&result), "1| alpha\n2| beta\n3| gamma");
    assert_eq!(result["details"]["lines"], 3);
    assert_eq!(result["details"]["shown"], 3);
    assert_eq!(result["details"]["bytes"], 17);
    assert_eq!(result["details"]["truncated"], false);
    assert_eq!(result["serialize"], false);
}

#[test]
fn read_windows_lines_and_reports_truncation() {
    let harness = Harness::new(&[]);
    let body: String = (1..=50).map(|index| format!("line {index}\n")).collect();
    harness.seed("long.txt", &body);

    let window = harness.call(
        "read",
        json!({"path": "long.txt", "offset": 10, "limit": 3}),
    );
    assert_eq!(output(&window), "10| line 10\n11| line 11\n12| line 12");
    assert_eq!(window["details"]["first_line"], 10);
    assert_eq!(window["details"]["last_line"], 12);
    assert_eq!(window["details"]["truncated"], true);

    let whole = harness.call("read", json!({"path": "long.txt"}));
    assert_eq!(whole["details"]["shown"], 50);
    assert_eq!(whole["details"]["truncated"], false);
}

#[test]
fn read_reports_path_errors_without_touching_the_filesystem() {
    // A second declaration without a workspace root exercises the shipped
    // default: relative paths only, absolute paths refused.
    const ROOTLESS: &str = r#"
local pi = ...
local module = pi.kernel.v1.module
local registry = module.require("pi.agent.tools", "1")
module.require("pi.tools.read", "1").declare(registry, { name = "read_rootless" })
"#;
    let harness = Harness::new(&[("rootless.lua", ROOTLESS)]);

    let missing = harness.call("read", json!({"path": "absent.txt"}));
    assert_eq!(missing["is_error"], true);
    assert!(output(&missing).starts_with("read failed:"), "{missing:?}");

    let escape = harness.call("read", json!({"path": "../outside.txt"}));
    assert_eq!(escape["is_error"], true);
    assert!(
        output(&escape).contains("escapes the workspace root"),
        "{escape:?}"
    );

    let outside = harness.call("read", json!({"path": "/etc/hostname"}));
    assert_eq!(outside["is_error"], true);
    assert!(
        output(&outside).contains("escapes the workspace root"),
        "{outside:?}"
    );

    let absolute = harness.call("read_rootless", json!({"path": "/etc/hostname"}));
    assert_eq!(absolute["is_error"], true);
    assert!(
        output(&absolute).contains("absolute paths are not allowed"),
        "{absolute:?}"
    );

    let wrong_type = harness.call("read", json!({"path": 7}));
    assert_eq!(wrong_type["is_error"], true);
    assert!(
        output(&wrong_type).contains("must be a string"),
        "{wrong_type:?}"
    );
}

// ---------------------------------------------------------------------------
// write
// ---------------------------------------------------------------------------

#[test]
fn write_creates_then_updates_with_diff_render_data() {
    let harness = Harness::new(&[]);

    let created = harness.call("write", json!({"path": "out.txt", "content": "one\ntwo\n"}));
    assert_eq!(created["is_error"], false);
    assert_eq!(created["details"]["created"], true);
    assert_eq!(created["details"]["added"], 2);
    assert_eq!(created["details"]["revision"], 1);
    assert_eq!(harness.contents("out.txt"), "one\ntwo\n");

    let updated = harness.call(
        "write",
        json!({"path": "out.txt", "content": "one\ntwo two\n"}),
    );
    assert_eq!(updated["details"]["created"], false);
    assert_eq!(updated["details"]["added"], 1);
    assert_eq!(updated["details"]["removed"], 1);
    assert_eq!(updated["details"]["revision"], 2);
    assert!(output(&updated).contains("- two"), "{updated:?}");
    assert!(output(&updated).contains("+ two two"), "{updated:?}");
    let rows = updated["details"]["diff"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "context" && row["text"] == "one"),
        "{rows:?}"
    );
}

#[test]
fn write_refuses_oversize_content_and_bad_paths() {
    let harness = Harness::new(&[]);

    let oversize = harness.call(
        "write_small",
        json!({"path": "big.txt", "content": "x".repeat(128)}),
    );
    assert_eq!(oversize["is_error"], true);
    assert!(
        output(&oversize).contains("exceeds the 64 byte limit"),
        "{oversize:?}"
    );
    assert!(!harness.exists("big.txt"));

    let escape = harness.call("write", json!({"path": "../escape.txt", "content": "no"}));
    assert_eq!(escape["is_error"], true);
    assert!(
        output(&escape).contains("escapes the workspace root"),
        "{escape:?}"
    );
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

#[test]
fn edit_replaces_a_unique_span_and_renders_the_diff() {
    let harness = Harness::new(&[]);
    harness.seed("code.rs", "fn main() {\n    let value = 1;\n}\n");

    let result = harness.call(
        "edit",
        json!({"path": "code.rs", "old_text": "let value = 1;", "new_text": "let value = 42;"}),
    );

    assert_eq!(result["is_error"], false);
    assert_eq!(result["details"]["replacements"], 1);
    assert_eq!(
        harness.contents("code.rs"),
        "fn main() {\n    let value = 42;\n}\n"
    );
    assert!(
        output(&result).contains("+     let value = 42;"),
        "{result:?}"
    );
    assert_eq!(result["serialize"], true);
}

#[test]
fn edit_rejects_ambiguous_and_missing_spans() {
    let harness = Harness::new(&[]);
    harness.seed("dup.txt", "same\nsame\nother\n");

    let ambiguous = harness.call(
        "edit",
        json!({"path": "dup.txt", "old_text": "same", "new_text": "changed"}),
    );
    assert_eq!(ambiguous["is_error"], true);
    assert!(output(&ambiguous).contains("2 matches"), "{ambiguous:?}");
    assert_eq!(harness.contents("dup.txt"), "same\nsame\nother\n");

    let missing = harness.call(
        "edit",
        json!({"path": "dup.txt", "old_text": "absent", "new_text": "x"}),
    );
    assert_eq!(missing["is_error"], true);
    assert!(output(&missing).contains("was not found"), "{missing:?}");

    let all = harness.call(
        "edit",
        json!({
            "path": "dup.txt",
            "old_text": "same",
            "new_text": "changed",
            "replace_all": true,
        }),
    );
    assert_eq!(all["details"]["replacements"], 2);
    assert_eq!(harness.contents("dup.txt"), "changed\nchanged\nother\n");
}

#[test]
fn edit_refuses_a_stale_revision_guard() {
    let harness = Harness::new(&[]);

    let written = harness.call("write", json!({"path": "guard.txt", "content": "alpha\n"}));
    assert_eq!(written["details"]["revision"], 1);

    let stale = harness.call(
        "edit",
        json!({
            "path": "guard.txt",
            "old_text": "alpha",
            "new_text": "beta",
            "expected_revision": 0,
        }),
    );
    assert_eq!(stale["is_error"], true);
    assert!(output(&stale).contains("stale revision 0"), "{stale:?}");
    assert_eq!(harness.contents("guard.txt"), "alpha\n");

    let current = harness.call(
        "edit",
        json!({
            "path": "guard.txt",
            "old_text": "alpha",
            "new_text": "beta",
            "expected_revision": 1,
        }),
    );
    assert_eq!(current["is_error"], false);
    assert_eq!(harness.contents("guard.txt"), "beta\n");
}

// ---------------------------------------------------------------------------
// bash
// ---------------------------------------------------------------------------

#[test]
fn bash_reports_output_streams_and_exit_code() {
    let harness = Harness::new(&[]);

    let ok = harness.call("bash", json!({"command": "printf 'hello\\n'"}));
    assert_eq!(ok["is_error"], false);
    assert_eq!(output(&ok), "hello\n");
    assert_eq!(ok["details"]["code"], 0);
    assert_eq!(ok["details"]["killed"], false);

    let failed = harness.call("bash", json!({"command": "printf 'bad\\n' >&2; exit 3"}));
    assert_eq!(failed["is_error"], true);
    assert!(output(&failed).contains("[stderr]\nbad"), "{failed:?}");
    assert!(output(&failed).contains("[exit 3]"), "{failed:?}");
    assert_eq!(failed["details"]["code"], 3);
}

#[test]
fn bash_bounds_large_output() {
    let harness = Harness::new(&[]);

    let result = harness.call("bash_small", json!({"command": "seq 1 5000"}));

    assert_eq!(result["details"]["truncated"], true);
    assert!(output(&result).len() < 512, "{}", output(&result).len());
    assert!(output(&result).contains("truncated"), "{result:?}");
}

#[test]
fn bash_kills_a_timed_out_command() {
    let harness = Harness::new(&[]);

    let result = harness.call("bash", json!({"command": "sleep 30", "timeout_ms": 300}));

    assert_eq!(result["is_error"], true);
    assert_eq!(result["details"]["killed"], true);
    assert_eq!(result["details"]["cancelled"], false);
    assert!(output(&result).contains("timed out"), "{result:?}");
}

#[test]
fn bash_cancellation_kills_the_whole_process_tree() {
    let harness = Harness::new(&[("cancelling.lua", CANCELLING_BASH)]);

    let result = harness.call(
        "bash_cancelling",
        json!({
            "command": "echo start\n( sleep 1; echo leaked > leaked.txt ) &\nwait",
            "timeout_ms": 10000,
        }),
    );

    assert_eq!(result["details"]["killed"], true);
    assert_eq!(result["details"]["cancelled"], true);
    assert!(
        result["details"]["process_group"].is_string(),
        "no process group observed: {result:?}"
    );

    // The backgrounded grandchild outlives a plain child kill; the tool kills
    // the command's process group, so the marker file is never written.
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert!(
        !harness.exists("leaked.txt"),
        "background child survived cancellation"
    );
}

// ---------------------------------------------------------------------------
// Declarations, serialization, replacement
// ---------------------------------------------------------------------------

#[test]
fn mutating_tools_declare_serialized_settlement() {
    let harness = Harness::new(&[]);

    let batch = harness.dispatch(json!({"kind": "declarations"}));
    let payload = batch.actions.first().unwrap().payload.clone();
    let tools = payload["tools"].as_array().unwrap();

    let serialize = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} in {tools:?}"))["serialize"]
            .clone()
    };
    assert_eq!(serialize("read"), json!(false));
    assert_eq!(serialize("write"), json!(true));
    assert_eq!(serialize("edit"), json!(true));
    assert_eq!(serialize("bash"), json!(true));

    let wire = payload["wire"].as_array().unwrap();
    let first = &wire[0];
    assert_eq!(first["name"], "read");
    assert!(first["description"].as_str().unwrap().contains("Read"));
    assert!(first["parameters"]["properties"]["path"].is_object());
    assert_eq!(
        payload["suite"],
        json!(["read", "write", "edit", "bash"]),
        "{payload:?}"
    );
}

#[test]
fn a_held_path_lock_rejects_a_concurrent_mutation() {
    let harness = Harness::new(&[]);
    harness.seed("locked.txt", "before\n");

    let batch = harness.dispatch(json!({
        "kind": "call_while_locked",
        "lock": "locked.txt",
        "name": "write_small",
        "arguments": {"path": "locked.txt", "content": "after\n"},
    }));
    let payload = batch.actions.first().unwrap().payload.clone();

    assert_eq!(payload["is_error"], true);
    assert_eq!(payload["details"]["busy"], true);
    assert!(output(&payload).contains("path is busy"), "{payload:?}");
    assert_eq!(harness.contents("locked.txt"), "before\n");

    // The lock is released with the dispatch, so the same write settles next.
    let after = harness.call(
        "write_small",
        json!({"path": "locked.txt", "content": "after\n"}),
    );
    assert_eq!(after["is_error"], false);
    assert_eq!(harness.contents("locked.txt"), "after\n");
}

#[test]
fn each_tool_is_independently_suppressible_and_replaceable_from_disk() {
    const REPLACEMENT: &str = r#"
local pi = ...
local module = pi.kernel.v1.module
local registry = module.require("pi.agent.tools", "1")
local suite = module.require("pi.tools.suite", "1")

suite.unregister(registry, "bash")
suite.unregister(registry, "read")
registry.register({
  name = "read",
  description = "replacement read",
  owner = "disk-package",
  execute = function(call)
    return { output = "replaced:" .. tostring(call.arguments.path) }
  end,
})
"#;
    let harness = Harness::new(&[("replacement.lua", REPLACEMENT)]);
    harness.seed("notes.txt", "alpha\n");

    let batch = harness.dispatch(json!({"kind": "declarations"}));
    let payload = batch.actions.first().unwrap().payload.clone();
    let names: Vec<String> = payload["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(!names.contains(&"bash".to_owned()), "{names:?}");
    assert!(names.contains(&"write".to_owned()), "{names:?}");
    assert!(names.contains(&"edit".to_owned()), "{names:?}");

    let replaced = harness.call("read", json!({"path": "notes.txt"}));
    assert_eq!(output(&replaced), "replaced:notes.txt");
    assert_eq!(replaced["details"], Value::Null);

    let missing = harness.dispatch(json!({"kind": "call", "name": "bash", "arguments": {}}));
    assert_eq!(missing.actions.first().unwrap().kind, "tool_missing");

    // Suppressing one tool leaves the others working from the same package.
    let written = harness.call("write", json!({"path": "kept.txt", "content": "kept\n"}));
    assert_eq!(written["is_error"], false);
    assert_eq!(harness.contents("kept.txt"), "kept\n");
}
