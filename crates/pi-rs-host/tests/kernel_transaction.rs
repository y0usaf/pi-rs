//! PLAN 1.1 kernel transaction, lifecycle, and source-neutrality invariants.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, HostError, PackageSource};

fn host(timeout_ms: i64) -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: timeout_ms,
        ..HostConfig::default()
    })
    .expect("host")
}

fn request(root: RootKind) -> DispatchRequest {
    DispatchRequest::new(
        root,
        serde_json::json!({"name":"event"}),
        serde_json::json!({"nested":{"value":42}}),
    )
}

#[test]
fn snapshot_is_immutable_and_actions_publish_in_call_order() {
    let host = host(5_000);
    host.load(
        "memory://transaction",
        r#"
local pi = ...
local k = pi.kernel.v1
k.root({
  kind = "application", id = "transaction", active = true, priority = 0,
  dispatch = function(snapshot)
    local event_mutates = pcall(function() snapshot.event.name = "changed" end)
    local context_mutates = pcall(function() snapshot.context.nested.value = 0 end)
    k.action("observe", {
      event = snapshot.event.name,
      context = snapshot.context.nested.value,
      event_mutates = event_mutates,
      context_mutates = context_mutates,
    })
    k.effect("timer", { milliseconds = 1 })
    k.action("finish", { ok = true })
  end,
})
"#,
    )
    .expect("package");

    let batch = host
        .dispatch(request(RootKind::Application))
        .expect("dispatch");
    assert_eq!(batch.actions.len(), 2);
    assert_eq!(batch.actions[0].sequence, 0);
    assert_eq!(batch.effects[0].sequence, 1);
    assert_eq!(batch.actions[1].sequence, 2);
    assert_eq!(batch.actions[0].kind, "observe");
    assert_eq!(batch.actions[0].payload["event"], "event");
    assert_eq!(batch.actions[0].payload["context"], 42);
    assert_eq!(batch.actions[0].payload["event_mutates"], false);
    assert_eq!(batch.actions[0].payload["context_mutates"], false);
}

#[test]
fn queued_actions_are_not_observable_until_dispatch_succeeds() {
    let host = host(5_000);
    host.load(
        "memory://publication",
        r#"
local pi = ...
local k = pi.kernel.v1
k.root({ kind="agent", id="publication", active=true, priority=0,
  dispatch=function()
    k.action("early", { value = 1 })
    pi.sleep(100)
    k.action("late", { value = 2 })
  end,
})
"#,
    )
    .expect("package");

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let worker = host.clone();
    let join = std::thread::spawn(move || {
        tx.send(worker.dispatch(request(RootKind::Agent))).unwrap();
    });
    std::thread::sleep(Duration::from_millis(20));
    assert!(rx.try_recv().is_err(), "no partial batch may be published");
    let batch = rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    join.join().unwrap();
    assert_eq!(
        batch
            .actions
            .iter()
            .map(|action| action.kind.as_str())
            .collect::<Vec<_>>(),
        ["early", "late"]
    );
}

#[test]
fn failed_dispatch_discards_its_entire_batch() {
    let host = host(5_000);
    host.load(
        "memory://failed-dispatch",
        r#"
local pi = ...
local k = pi.kernel.v1
k.root({ kind="frontend", id="failure", active=true, priority=0,
  dispatch=function()
    k.action("ghost", { value = true })
    error("dispatch failed")
  end,
})
"#,
    )
    .expect("package");

    let error = host
        .dispatch(request(RootKind::Frontend))
        .expect_err("failed dispatch has no batch");
    assert!(error.to_string().contains("dispatch failed"));
}

#[test]
fn action_batches_are_count_bounded_and_fail_without_publication() {
    let host = host(5_000);
    host.load(
        "memory://batch-bound",
        r#"
local k = (...).kernel.v1
k.root({ kind="session", id="bound", active=true, priority=0,
  dispatch=function()
    for index=1,1025 do k.action("item", { index=index }) end
  end })
"#,
    )
    .expect("package");
    let error = host
        .dispatch(request(RootKind::Session))
        .expect_err("oversized batch is rejected as one transaction");
    assert!(error.to_string().contains("1024 queued actions"));
}

#[test]
fn failed_package_load_publishes_no_root_or_declaration() {
    let host = host(5_000);
    let failure = host.load(
        "memory://failed-package",
        r#"
local k = (...).kernel.v1
k.root({ kind="session", id="ghost-root", active=true, priority=0,
  dispatch=function() k.action("ghost", {}) end })
k.declare("theme", { id="ghost-theme" })
error("load failed")
"#,
    );
    assert!(failure.is_err());
    assert!(matches!(
        host.dispatch(request(RootKind::Session)),
        Err(HostError::UnknownRoot(_))
    ));

    host.load(
        "memory://replacement-package",
        r#"
local k = (...).kernel.v1
k.root({ kind="session", id="ghost-root", active=true, priority=0,
  dispatch=function() k.action("real", {}) end })
k.declare("theme", { id="ghost-theme" })
"#,
    )
    .expect("failed package left no declarations behind");
    assert_eq!(
        host.dispatch(request(RootKind::Session)).unwrap().actions[0].kind,
        "real"
    );
}

#[test]
fn read_handles_reject_stale_generations_after_scope_disposal() {
    let host = host(5_000);
    let package = host
        .load_package(PackageSource::Memory {
            key: "memory://handle-owner",
            source: "local _ = ...",
        })
        .expect("package");
    let handle = host.read_handle(serde_json::json!({"value":42}));
    assert_eq!(host.read(&handle).unwrap()["value"], 42);

    host.dispose_package(&package).expect("dispose");
    assert!(matches!(
        host.read(&handle),
        Err(HostError::StaleHandle { .. })
    ));
}

#[test]
fn lua_read_handles_reject_reuse_across_generation_change() {
    let host = host(5_000);
    host.load(
        "memory://lua-handle",
        r#"
local k = (...).kernel.v1
local saved
k.root({ kind="application", id="handle", active=true, priority=0,
  dispatch=function(snapshot)
    if saved == nil then
      saved = k.read_handle(snapshot.context)
      k.action("saved", {})
    else
      saved:read()
      k.action("unexpected", {})
    end
  end })
"#,
    )
    .expect("handle package");
    assert_eq!(
        host.dispatch(request(RootKind::Application))
            .unwrap()
            .actions[0]
            .kind,
        "saved"
    );
    let other = host
        .load_package(PackageSource::Memory {
            key: "memory://generation-change",
            source: "local _ = ...",
        })
        .unwrap();
    host.dispose_package(&other).unwrap();
    let error = host
        .dispatch(request(RootKind::Application))
        .expect_err("Lua handle is stale");
    assert!(error.to_string().contains("stale read handle generation"));
}

#[test]
fn busy_loop_root_is_watchdog_bounded() {
    let host = host(10);
    host.load(
        "memory://busy-root",
        r#"
local k = (...).kernel.v1
k.root({ kind="application", id="busy", active=true, priority=0,
  dispatch=function() while true do end end })
"#,
    )
    .expect("package");
    assert!(matches!(
        host.dispatch(request(RootKind::Application)),
        Err(HostError::Timeout(10))
    ));
}

fn conflict_host(order: [&str; 2]) -> (String, String) {
    let declaration_host = host(5_000);
    let declaration_error = declaration_host
        .load(
            order[0],
            "local k=(...).kernel.v1; k.declare('tool', { id='same' })",
        )
        .and_then(|()| {
            declaration_host.load(
                order[1],
                "local k=(...).kernel.v1; k.declare('tool', { id='same' })",
            )
        })
        .expect_err("duplicate declaration");

    let roots = host(5_000);
    for source in order {
        let id = source.trim_matches('/').replace([':', '.'], "-");
        roots
            .load(
                source,
                &format!(
                    "local k=(...).kernel.v1; k.root({{kind='agent', id='{id}', active=true, priority=7, dispatch=function() end}})"
                ),
            )
            .expect("root");
    }
    let root_error = roots
        .dispatch(request(RootKind::Agent))
        .expect_err("equal-priority roots conflict");
    (declaration_error.to_string(), root_error.to_string())
}

#[test]
fn composable_declarations_and_versioned_modules_share_one_registry_path() {
    let host = host(5_000);
    host.load(
        "memory://composition",
        r#"
local k = (...).kernel.v1
k.module.define({ name="shared", version="1", dependencies={},
  factory=function() return { value=42 } end })
k.declare("renderer", { id="late", order=10 })
k.declare("renderer", { id="early", order=-10 })
k.root({ kind="application", id="composition", active=true, priority=0,
  dispatch=function()
    local ids = {}
    for _, declaration in ipairs(k.registered("renderer")) do
      ids[#ids + 1] = declaration.id
    end
    k.action("composition", {
      ids=ids,
      module=k.module.require("shared", "1").value,
    })
  end })
"#,
    )
    .expect("package");
    let batch = host.dispatch(request(RootKind::Application)).unwrap();
    assert_eq!(
        batch.actions[0].payload["ids"],
        serde_json::json!(["early", "late"])
    );
    assert_eq!(batch.actions[0].payload["module"], 42);
}

#[test]
fn root_and_declaration_conflicts_are_deterministic() {
    let forward = conflict_host(["source://a", "source://b"]);
    let reverse = conflict_host(["source://b", "source://a"]);
    assert_eq!(forward, reverse);
    assert!(forward.0.contains("source://a <> source://b"));
    assert!(forward.1.contains("source://a"));
    assert!(forward.1.contains("source://b"));
}

#[test]
fn cancellation_and_resource_disposal_are_scope_owned() {
    let directory = tempfile::tempdir().expect("tempdir");
    let marker = directory.path().join("disposed.txt");
    let marker_lua = serde_json::to_string(&marker.to_string_lossy()).unwrap();
    let source = format!(
        r#"
local pi = ...
local k = pi.kernel.v1
k.resource(function() pi.fs.write_file({marker_lua}, "disposed") end)
k.root({{ kind="frontend", id="cancellable", active=true, priority=0,
  dispatch=function()
    pi.sleep(10000)
    k.action("too-late", {{}})
  end,
}})
"#
    );
    let host = host(5_000);
    let package = host
        .load_package(PackageSource::Memory {
            key: "memory://scoped",
            source: &source,
        })
        .expect("package");
    assert_eq!(host.scope_stats(&package).unwrap().resources, 1);

    let worker = host.clone();
    let join = std::thread::spawn(move || worker.dispatch(request(RootKind::Frontend)));
    std::thread::sleep(Duration::from_millis(30));
    host.dispose_package(&package).expect("dispose");
    assert!(matches!(join.join().unwrap(), Err(HostError::Cancelled)));
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "disposed");
    let stats = host.scope_stats(&package).unwrap();
    assert!(stats.disposed);
    assert!(stats.cancelled);
    assert_eq!(stats.resources, 0);
}

#[test]
fn final_host_drop_cancels_scopes_and_runs_disposers() {
    let directory = tempfile::tempdir().expect("tempdir");
    let marker = directory.path().join("shutdown.txt");
    let marker_lua = serde_json::to_string(&marker.to_string_lossy()).unwrap();
    let source = format!(
        "local pi=...; pi.kernel.v1.resource(function() pi.fs.write_file({marker_lua}, 'shutdown') end)"
    );
    {
        let host = host(5_000);
        host.load_package(PackageSource::Memory {
            key: "memory://shutdown",
            source: &source,
        })
        .expect("package");
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "shutdown");
}

const SOURCE_NEUTRAL_PACKAGE: &str = r#"
local pi = ...
local k = pi.kernel.v1
k.resource(function() end)
k.declare("renderer", { id="probe", order=3 })
k.root({ kind="application", id="neutral", active=true, priority=0,
  dispatch=function(snapshot)
    k.action("capability", {
      version=k.api_version,
      value=snapshot.context.nested.value,
      module_type=type(k.module),
    })
    k.effect("lifecycle", { active=true })
  end,
})
"#;

fn source_neutral_result(package: PackageSource<'_>) -> (serde_json::Value, usize, bool) {
    let host = host(5_000);
    let package = host.load_package(package).expect("package");
    let before = host.scope_stats(&package).unwrap();
    let batch = host.dispatch(request(RootKind::Application)).unwrap();
    host.dispose_package(&package).unwrap();
    let after = host.scope_stats(&package).unwrap();
    (
        serde_json::json!({
            "version": batch.version,
            "actions": batch.actions,
            "effects": batch.effects.iter().map(|effect| serde_json::json!({
                "sequence": effect.sequence,
                "kind": effect.kind,
                "payload": effect.payload,
            })).collect::<Vec<_>>(),
        }),
        before.resources,
        after.disposed && after.cancelled && after.resources == 0,
    )
}

#[test]
fn embedded_and_file_packages_have_equal_capability_and_lifecycle() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("neutral.lua");
    std::fs::write(&path, SOURCE_NEUTRAL_PACKAGE).expect("file package");

    let embedded = source_neutral_result(PackageSource::Embedded {
        name: "neutral",
        source: SOURCE_NEUTRAL_PACKAGE,
    });
    let file = source_neutral_result(PackageSource::File {
        path: Path::new(&path),
    });
    assert_eq!(embedded, file);
}
