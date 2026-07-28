//! Package composition and lifecycle driven from ordinary file-backed packages.
//!
//! A supervisor package composes other packages, selects between generations,
//! swaps one for another, and is cascade-disposed with everything it composed.
//! Every location, order, and swap decision is Lua; Rust only loads bytes,
//! bounds nesting, and disposes scopes.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const SUPERVISOR: &str = r#"
local pi = ...
local packages = pi.packages.v1
local kernel = pi.kernel.v1
local roots = pi.roots.v1
local effects = pi.effects.v1

local directory = effects.env.get("PI_TEST_PACKAGE_DIR")
if not directory then error("PI_TEST_PACKAGE_DIR is required") end

local composed = {}

local function compose(file)
  local handle = packages.load({ path = effects.path.join(directory, file) })
  composed[#composed + 1] = handle
  return handle
end

-- Selection policy: the newest registered tool generation wins. Both
-- generations may be registered at once, so a failed load never leaves the
-- application without one.
local function selected()
  local best = nil
  for _, entry in ipairs(kernel.registered("tool")) do
    if best == nil or entry.sequence > best.sequence then best = entry end
  end
  return best
end

local function sources()
  local names = {}
  for _, entry in ipairs(packages.list()) do
    names[#names + 1] = effects.path.basename(entry.source)
  end
  return names
end

compose("answer_one.lua")

roots.register({
  kind = "application",
  id = "supervisor",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    local event = snapshot.event
    if event.kind == "ask" then
      local tool = selected()
      roots.action("answered", { id = tool.declaration_id, answer = tool.answer })
      return
    end
    if event.kind == "swap" then
      local previous = composed[#composed]
      local ok, failure = pcall(compose, event.file)
      if not ok then
        roots.action("swap-failed", {
          message = tostring(failure),
          selected = selected().declaration_id,
          loaded = sources(),
        })
        return
      end
      previous:dispose()
      roots.action("swapped", {
        selected = selected().declaration_id,
        loaded = sources(),
        previous_disposed = previous:disposed(),
      })
      return
    end
    error("unsupported event " .. tostring(event.kind))
  end,
})
"#;

const ANSWER_ONE: &str = r#"
local pi = ...
local kernel = pi.kernel.v1
local effects = pi.effects.v1

kernel.declare("tool", { id = "answer-one", answer = "one" })

-- Package-owned cleanup: cascade disposal must reach a composed package.
kernel.resource(function()
  local directory = effects.env.get("PI_TEST_PACKAGE_DIR")
  effects.fs.write(effects.path.join(directory, "answer_one.disposed"), "one")
end)
"#;

const ANSWER_TWO: &str = r#"
local pi = ...
pi.kernel.v1.declare("tool", { id = "answer-two", answer = "two" })
"#;

const BROKEN: &str = r#"
local pi = ...
pi.kernel.v1.declare("tool", { id = "answer-broken", answer = "broken" })
error("deliberate load failure")
"#;

fn host_for(directory: &Path) -> Host {
    let mut environment = BTreeMap::new();
    environment.insert(
        "PI_TEST_PACKAGE_DIR".to_owned(),
        directory.to_string_lossy().into_owned(),
    );
    Host::new(HostConfig {
        environment: Some(environment),
        ..HostConfig::default()
    })
    .expect("host starts")
}

fn write(directory: &Path, name: &str, source: &str) {
    std::fs::write(directory.join(name), source).expect("write package");
}

fn ask(host: &Host, event: serde_json::Value) -> Result<DispatchBatch, pi_rs_host::HostError> {
    host.dispatch(DispatchRequest::new(
        RootKind::Application,
        event,
        serde_json::json!({}),
    ))
}

#[test]
fn a_composed_package_serves_and_is_cascade_disposed() {
    let directory = tempfile::tempdir().expect("temporary package directory");
    write(directory.path(), "supervisor.lua", SUPERVISOR);
    write(directory.path(), "answer_one.lua", ANSWER_ONE);
    let host = host_for(directory.path());
    let supervisor = host
        .load_package(PackageSource::File {
            path: &directory.path().join("supervisor.lua"),
        })
        .expect("supervisor loads and composes its child");

    let batch = ask(&host, serde_json::json!({ "kind": "ask" })).expect("ask dispatch");
    assert_eq!(batch.actions[0].kind, "answered");
    assert_eq!(
        batch.actions[0].payload,
        serde_json::json!({ "id": "answer-one", "answer": "one" })
    );

    let marker = directory.path().join("answer_one.disposed");
    assert!(!marker.exists(), "child cleanup must not run while loaded");

    host.dispose_package(&supervisor)
        .expect("supervisor disposal succeeds");
    assert!(
        marker.exists(),
        "disposing the composing package must dispose what it composed"
    );
    assert!(
        ask(&host, serde_json::json!({ "kind": "ask" })).is_err(),
        "no application root remains after cascade disposal"
    );
}

#[test]
fn a_failed_swap_leaves_the_previous_generation_selected() {
    let directory = tempfile::tempdir().expect("temporary package directory");
    write(directory.path(), "supervisor.lua", SUPERVISOR);
    write(directory.path(), "answer_one.lua", ANSWER_ONE);
    write(directory.path(), "answer_two.lua", ANSWER_TWO);
    write(directory.path(), "broken.lua", BROKEN);
    let host = host_for(directory.path());
    host.load_package(PackageSource::File {
        path: &directory.path().join("supervisor.lua"),
    })
    .expect("supervisor loads");

    let failed = ask(
        &host,
        serde_json::json!({ "kind": "swap", "file": "broken.lua" }),
    )
    .expect("failed swap still publishes a batch");
    assert_eq!(failed.actions[0].kind, "swap-failed");
    let payload = &failed.actions[0].payload;
    assert_eq!(payload["selected"], "answer-one");
    assert_eq!(payload["loaded"], serde_json::json!(["answer_one.lua"]));
    assert!(
        payload["message"]
            .as_str()
            .expect("failure message")
            .contains("deliberate load failure")
    );

    let swapped = ask(
        &host,
        serde_json::json!({ "kind": "swap", "file": "answer_two.lua" }),
    )
    .expect("swap dispatch");
    assert_eq!(swapped.actions[0].kind, "swapped");
    assert_eq!(
        swapped.actions[0].payload,
        serde_json::json!({
            "selected": "answer-two",
            "loaded": ["answer_two.lua"],
            "previous_disposed": true,
        })
    );

    let answered = ask(&host, serde_json::json!({ "kind": "ask" })).expect("ask dispatch");
    assert_eq!(
        answered.actions[0].payload,
        serde_json::json!({ "id": "answer-two", "answer": "two" })
    );

    // The same source may not be composed twice at once, exactly as through the
    // host package API.
    let duplicate = ask(
        &host,
        serde_json::json!({ "kind": "swap", "file": "answer_two.lua" }),
    )
    .expect("duplicate swap publishes a batch");
    assert_eq!(duplicate.actions[0].kind, "swap-failed");
    assert_eq!(duplicate.actions[0].payload["selected"], "answer-two");
    assert!(
        duplicate.actions[0].payload["message"]
            .as_str()
            .expect("failure message")
            .contains("already loaded")
    );
}

/// One chain file per level: every level composes the next and reports the
/// registered generations through the first level's root.
fn write_chain(directory: &Path, levels: usize) {
    for level in 1..=levels {
        let next = if level == levels {
            String::new()
        } else {
            format!(
                "local ok, failure = pcall(function()\n  pi.packages.v1.load({{ path = {:?} }})\nend)\nif not ok then\n  pi.kernel.v1.declare(\"tool\", {{ id = \"depth-guard\", answer = tostring(failure) }})\nend\n",
                directory
                    .join(format!("chain_{}.lua", level + 1))
                    .to_string_lossy()
            )
        };
        let root = if level == 1 {
            r#"
pi.roots.v1.register({
  kind = "application", id = "chain", active = true, priority = 0,
  dispatch = function()
    local ids = {}
    local guard = nil
    for _, entry in ipairs(pi.kernel.v1.registered("tool")) do
      ids[#ids + 1] = entry.declaration_id
      if entry.declaration_id == "depth-guard" then guard = entry.answer end
    end
    local composed = {}
    for _, entry in ipairs(pi.packages.v1.list()) do
      composed[#composed + 1] = entry.scope
    end
    table.sort(ids)
    pi.roots.v1.action("chain", { ids = ids, composed = #composed, guard = guard })
  end,
})
"#
        } else {
            ""
        };
        write(
            directory,
            &format!("chain_{level}.lua"),
            &format!(
                "local pi = ...\n{next}pi.kernel.v1.declare(\"tool\", {{ id = \"chain-{level}\" }})\n{root}"
            ),
        );
    }
}

#[test]
fn nested_package_loads_are_depth_bounded() {
    let directory = tempfile::tempdir().expect("temporary package directory");
    write_chain(directory.path(), 6);
    let host = host_for(directory.path());
    host.load_package(PackageSource::File {
        path: &directory.path().join("chain_1.lua"),
    })
    .expect("the bounded part of the chain loads");

    let batch = ask(&host, serde_json::json!({ "kind": "report" })).expect("chain dispatch");
    let payload = &batch.actions[0].payload;
    let ids = payload["ids"].as_array().expect("tool ids");
    let ids: Vec<&str> = ids.iter().filter_map(serde_json::Value::as_str).collect();
    assert!(
        payload["guard"]
            .as_str()
            .expect("depth guard message")
            .contains("exceeds depth 4"),
        "the refusal names the nesting bound"
    );
    assert_eq!(
        ids,
        vec![
            "chain-1",
            "chain-2",
            "chain-3",
            "chain-4",
            "chain-5",
            "depth-guard"
        ],
        "the sixth level is refused; the first five load"
    );
    assert_eq!(
        payload["composed"], 4,
        "at most four nested loads are composed"
    );
}
