//! Module reload and ownership driven from ordinary file-backed packages.
//!
//! A package redefines and re-runs its own module while it keeps running, so
//! reload no longer requires disposing the whole package. Rust chooses no
//! reload order, no invalidation policy, and no cleanup hook: replacement is
//! `remove` then `define`, re-running a factory is `reset`, and a module's
//! resources are ordinary `kernel.resource` handles the module owner disposes.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::Path;

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const RELOADER: &str = r#"
local pi = ...
local kernel = pi.kernel.v1
local roots = pi.roots.v1

local runs = 0
local disposed = {}

-- Declaration is unchanged: `define` still refuses a duplicate identity, so a
-- replacement is explicitly `remove` then `define` (the same shape package
-- composition uses for atomic generation swaps).
local function define(tag)
  kernel.module.define({
    name = "app.policy",
    version = "1",
    factory = function()
      runs = runs + 1
      local label = tag .. ":" .. runs
      local resource = kernel.resource(function()
        disposed[#disposed + 1] = label
      end)
      return { tag = tag, run = runs, resource = resource }
    end,
  })
end

define("first")

local function states()
  local result = {}
  for _, entry in ipairs(kernel.module.list()) do
    result[#result + 1] = entry.name .. "@" .. entry.version .. ":" .. entry.state
  end
  return result
end

local function report(extra)
  local value = kernel.module.require("app.policy", "1")
  local payload = {
    tag = value.tag,
    run = value.run,
    states = states(),
    disposed = table.concat(disposed, ","),
  }
  for key, item in pairs(extra or {}) do payload[key] = item end
  roots.action("policy", payload)
end

roots.register({
  kind = "application",
  id = "reloader",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    local kind = snapshot.event.kind
    if kind == "read" then
      report(nil)
    elseif kind == "reset" then
      kernel.module.require("app.policy", "1").resource:dispose()
      local dropped = kernel.module.reset("app.policy", "1")
      local again = kernel.module.reset("app.policy", "1")
      report({ dropped = dropped, again = again })
    elseif kind == "replace" then
      kernel.module.require("app.policy", "1").resource:dispose()
      local removed = kernel.module.remove("app.policy", "1")
      local missing = kernel.module.remove("app.policy", "1")
      define(snapshot.event.tag)
      report({ removed = removed, missing = missing })
    else
      error("unexpected event " .. tostring(kind))
    end
  end,
})
"#;

const OWNER: &str = r#"
local pi = ...
local kernel = pi.kernel.v1
local roots = pi.roots.v1

kernel.module.define({
  name = "shared.value",
  version = "1",
  factory = function() return { text = "owned" } end,
})

-- A factory that tries to change its own declaration while dependents are
-- mid-resolution must be refused, not silently unwound.
kernel.module.define({
  name = "self.destruct",
  version = "1",
  factory = function()
    kernel.module.remove("self.destruct", "1")
    return { text = "unreachable" }
  end,
})

roots.register({
  kind = "application",
  id = "owner",
  active = true,
  priority = 0,
  dispatch = function()
    local ok, failure = pcall(kernel.module.require, "self.destruct", "1")
    roots.action("owner", {
      value = kernel.module.require("shared.value", "1").text,
      self_destruct_ok = ok,
      self_destruct_error = tostring(failure),
    })
  end,
})
"#;

const INTRUDER: &str = r#"
local pi = ...
local kernel = pi.kernel.v1
local roots = pi.roots.v1

roots.register({
  kind = "agent",
  id = "intruder",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    if snapshot.event.kind == "after-disposal" then
      local ok, failure = pcall(kernel.module.require, "shared.value", "1")
      roots.action("intruder", { require_ok = ok, require_error = tostring(failure) })
      return
    end
    local remove_ok, remove_error = pcall(kernel.module.remove, "shared.value", "1")
    local reset_ok, reset_error = pcall(kernel.module.reset, "shared.value", "1")
    roots.action("intruder", {
      remove_ok = remove_ok,
      remove_error = tostring(remove_error),
      reset_ok = reset_ok,
      reset_error = tostring(reset_error),
      still_readable = kernel.module.require("shared.value", "1").text,
    })
  end,
})
"#;

fn write_package(directory: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, source).expect("write file-backed package");
    path
}

fn dispatch(host: &Host, kind: RootKind, event: serde_json::Value) -> DispatchBatch {
    host.dispatch(DispatchRequest::new(kind, event, serde_json::json!({})))
        .expect("dispatch succeeds")
}

fn payload(batch: &DispatchBatch) -> serde_json::Value {
    assert_eq!(batch.actions.len(), 1, "one action per dispatch");
    batch.actions[0].payload.clone()
}

#[test]
fn a_package_reloads_its_own_module_without_being_disposed() {
    let directory = tempfile::tempdir().expect("temporary package directory");
    let path = write_package(directory.path(), "reloader.lua", RELOADER);
    let host = Host::new(HostConfig::default()).expect("host starts");
    let handle = host
        .load_package(PackageSource::File { path: &path })
        .expect("reloader package loads");

    // A require caches: the factory runs once no matter how often it resolves.
    let first = payload(&dispatch(
        &host,
        RootKind::Application,
        serde_json::json!({ "kind": "read" }),
    ));
    assert_eq!(first["tag"], "first");
    assert_eq!(first["run"], 1);
    assert_eq!(first["states"], serde_json::json!(["app.policy@1:loaded"]));
    assert_eq!(first["disposed"], "");
    let cached = payload(&dispatch(
        &host,
        RootKind::Application,
        serde_json::json!({ "kind": "read" }),
    ));
    assert_eq!(cached["run"], 1);

    // `reset` drops only the cached value: the same factory re-runs, and a
    // second reset reports that nothing was cached.
    let reset = payload(&dispatch(
        &host,
        RootKind::Application,
        serde_json::json!({ "kind": "reset" }),
    ));
    assert_eq!(reset["dropped"], true);
    assert_eq!(reset["again"], false);
    assert_eq!(reset["tag"], "first");
    assert_eq!(reset["run"], 2);
    assert_eq!(reset["disposed"], "first:1");
    assert_eq!(reset["states"], serde_json::json!(["app.policy@1:loaded"]));

    // `remove` + `define` swaps the implementation of a live identity, and the
    // order index is pruned so the reused identity is still listed once.
    let replaced = payload(&dispatch(
        &host,
        RootKind::Application,
        serde_json::json!({ "kind": "replace", "tag": "second" }),
    ));
    assert_eq!(replaced["removed"], true);
    assert_eq!(replaced["missing"], false);
    assert_eq!(replaced["tag"], "second");
    assert_eq!(replaced["run"], 3);
    assert_eq!(replaced["disposed"], "first:1,first:2");
    assert_eq!(
        replaced["states"],
        serde_json::json!(["app.policy@1:loaded"])
    );

    let after = payload(&dispatch(
        &host,
        RootKind::Application,
        serde_json::json!({ "kind": "read" }),
    ));
    assert_eq!(after["tag"], "second");
    assert_eq!(after["run"], 3);

    host.dispose_package(&handle).expect("package disposes");
    let stats = host.scope_stats(&handle).expect("scope stats");
    assert!(stats.disposed);
    assert_eq!(stats.resources, 0);
}

#[test]
fn module_lifecycle_is_scope_local_and_refuses_a_loading_factory() {
    let directory = tempfile::tempdir().expect("temporary package directory");
    let owner_path = write_package(directory.path(), "owner.lua", OWNER);
    let intruder_path = write_package(directory.path(), "intruder.lua", INTRUDER);
    let host = Host::new(HostConfig::default()).expect("host starts");
    let owner = host
        .load_package(PackageSource::File { path: &owner_path })
        .expect("owner package loads");
    host.load_package(PackageSource::File {
        path: &intruder_path,
    })
    .expect("intruder package loads");

    // A sibling package may read the module but may not reload it.
    let intruded = payload(&dispatch(
        &host,
        RootKind::Agent,
        serde_json::json!({ "kind": "probe" }),
    ));
    assert_eq!(intruded["remove_ok"], false);
    assert_eq!(intruded["reset_ok"], false);
    assert_eq!(intruded["still_readable"], "owned");
    let owner_source = owner_path.to_string_lossy().to_string();
    for key in ["remove_error", "reset_error"] {
        let message = intruded[key].as_str().expect("error message");
        assert!(
            message.contains("module shared.value@1 is owned by"),
            "{key} should name the ownership rule: {message}"
        );
        assert!(
            message.contains(&owner_source),
            "{key} should name the owning source: {message}"
        );
    }

    // A factory cannot change its own declaration while it is running.
    let owned = payload(&dispatch(
        &host,
        RootKind::Application,
        serde_json::json!({ "kind": "probe" }),
    ));
    assert_eq!(owned["value"], "owned");
    assert_eq!(owned["self_destruct_ok"], false);
    let failure = owned["self_destruct_error"]
        .as_str()
        .expect("error message")
        .to_string();
    assert!(
        failure.contains("is loading and cannot be changed"),
        "a loading factory must be refused: {failure}"
    );

    // Scope disposal remains the other lifecycle path: the owner's modules
    // leave with it, and the sibling sees an ordinary undefined diagnostic.
    host.dispose_package(&owner)
        .expect("owner package disposes");
    let after = payload(&dispatch(
        &host,
        RootKind::Agent,
        serde_json::json!({ "kind": "after-disposal" }),
    ));
    assert_eq!(after["require_ok"], false);
    let message = after["require_error"].as_str().expect("error message");
    assert!(
        message.contains("is not defined"),
        "a disposed module is undefined: {message}"
    );
}
