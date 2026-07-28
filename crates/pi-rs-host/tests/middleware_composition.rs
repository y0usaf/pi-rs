//! PLAN 3.2 middleware composition, module version conflicts, watchdog
//! isolation, and rollback invariants on the narrow walking-skeleton surface.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, HostError, PackageSource};

fn host(timeout_ms: i64) -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: timeout_ms,
        ..HostConfig::default()
    })
    .expect("host")
}

fn request(root: RootKind, event: serde_json::Value) -> DispatchRequest {
    DispatchRequest::new(root, event, serde_json::Value::Null)
}

#[test]
fn event_middleware_transforms_and_short_circuits_in_order() {
    let host = host(5_000);
    host.load(
        "memory://mw-roots",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "agent", id = "base", dispatch = function(snapshot)
  roots.action("root", { text = snapshot.event.text })
end })
"#,
    )
    .expect("root package");
    // Two event stages compose in ascending order: the first rewrites the
    // event the root sees, the second appends. A later stage stops the
    // chain for a flagged event so the root never runs.
    host.load(
        "memory://mw-event",
        r#"
local pi = ...
local middleware = pi.roots.v1.middleware
middleware.register({
  kind = "agent", phase = "event", id = "first", order = 10,
  handler = function(snapshot)
    return { event = { kind = snapshot.event.kind, text = (snapshot.event.text or "") .. "-a" } }
  end,
})
middleware.register({
  kind = "agent", phase = "event", id = "second", order = 20,
  handler = function(snapshot)
    if snapshot.event.text == "stop-a" then
      return { stop = true, actions = { { kind = "suppressed", payload = {} } } }
    end
    return { event = { kind = snapshot.event.kind, text = snapshot.event.text .. "-b" } }
  end,
})
"#,
    )
    .expect("middleware package");

    let batch = host
        .dispatch(request(
            RootKind::Agent,
            serde_json::json!({"kind":"turn","text":"go"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions.len(), 1);
    assert_eq!(batch.actions[0].kind, "root");
    assert_eq!(batch.actions[0].payload["text"], "go-a-b");

    // Short-circuit: the root is skipped and the stage's actions publish.
    let batch = host
        .dispatch(request(
            RootKind::Agent,
            serde_json::json!({"kind":"turn","text":"stop"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions.len(), 1);
    assert_eq!(batch.actions[0].kind, "suppressed");
}

#[test]
fn render_middleware_transforms_settled_actions() {
    let host = host(5_000);
    host.load(
        "memory://mw-render-root",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "frontend", id = "base", dispatch = function()
  roots.action("ansi", { data = "frame" })
  roots.action("status", { ready = true })
end })
"#,
    )
    .expect("root package");
    host.load(
        "memory://mw-render",
        r#"
local pi = ...
local middleware = pi.roots.v1.middleware
middleware.register({
  kind = "frontend", phase = "render", id = "tag", order = 5,
  handler = function(snapshot)
    local next = {}
    for _, action in ipairs(snapshot.actions) do
      local payload = {}
      for key, value in pairs(action.payload) do payload[key] = value end
      payload.tagged = true
      next[#next + 1] = { kind = action.kind, payload = payload }
    end
    return { actions = next }
  end,
})
"#,
    )
    .expect("middleware package");

    let batch = host
        .dispatch(request(
            RootKind::Frontend,
            serde_json::json!({"kind":"render"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions.len(), 2);
    assert!(
        batch
            .actions
            .iter()
            .all(|action| action.payload["tagged"] == true)
    );
    assert_eq!(batch.actions[0].kind, "ansi");
    assert_eq!(batch.actions[1].kind, "status");
}

#[test]
fn failing_render_middleware_rolls_back_the_whole_dispatch() {
    let host = host(5_000);
    host.load(
        "memory://mw-fail-root",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "application", id = "base", dispatch = function()
  roots.action("ghost", { value = true })
end })
"#,
    )
    .expect("root package");
    host.load(
        "memory://mw-fail-render",
        r#"
local pi = ...
pi.roots.v1.middleware.register({
  kind = "application", phase = "render", id = "explode",
  handler = function() error("render transform failed") end,
})
"#,
    )
    .expect("middleware package");

    let error = host
        .dispatch(request(
            RootKind::Application,
            serde_json::json!({"kind":"startup"}),
        ))
        .expect_err("the failing transform must error the dispatch");
    assert!(
        matches!(error, HostError::Lua(ref message) if message.contains("render transform failed")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn middleware_registration_conflicts_and_scope_rollback() {
    let host = host(5_000);
    host.load(
        "memory://mw-owner-a",
        r#"
local pi = ...
pi.roots.v1.middleware.register({
  kind = "agent", phase = "event", id = "shared", handler = function() end,
})
"#,
    )
    .expect("first package");
    // A different source registering the same kind/phase/id conflicts
    // deterministically.
    let conflict = host
        .load(
            "memory://mw-owner-b",
            r#"
local pi = ...
pi.roots.v1.middleware.register({
  kind = "agent", phase = "event", id = "shared", handler = function() end,
})
"#,
        )
        .expect_err("duplicate middleware id from another source must conflict");
    assert!(
        matches!(conflict, HostError::Conflict(ref message) if message.contains("agent/event/shared")),
        "unexpected conflict: {conflict:?}"
    );

    // Rollback: a package that fails after registering middleware publishes
    // nothing; its stage never runs.
    host.load(
        "memory://mw-roots-rb",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "agent", id = "rollback-base", dispatch = function(snapshot)
  roots.action("seen", { text = snapshot.event.text })
end })
"#,
    )
    .expect("base root");
    let rollback = host.load(
        "memory://mw-rollback",
        r#"
local pi = ...
pi.roots.v1.middleware.register({
  kind = "agent", phase = "event", id = "rolled-back",
  handler = function(snapshot)
    return { event = { kind = snapshot.event.kind, text = "rewritten" } }
  end,
})
error("load fails after registration")
"#,
    );
    assert!(rollback.is_err(), "failing load must not publish");

    let batch = host
        .dispatch(request(
            RootKind::Agent,
            serde_json::json!({"kind":"turn","text":"original"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions[0].payload["text"], "original");
}

#[test]
fn disposed_package_middleware_stops_running() {
    let host = host(5_000);
    host.load(
        "memory://mw-d-root",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "agent", id = "dispose-base", dispatch = function(snapshot)
  roots.action("seen", { text = snapshot.event.text })
end })
"#,
    )
    .expect("base root");
    let handle = host
        .load_package(PackageSource::Memory {
            key: "memory://mw-d-stage",
            source: r#"
local pi = ...
pi.roots.v1.middleware.register({
  kind = "agent", phase = "event", id = "disposable",
  handler = function(snapshot)
    return { event = { kind = snapshot.event.kind, text = "rewritten" } }
  end,
})
"#,
        })
        .expect("middleware package");

    let batch = host
        .dispatch(request(
            RootKind::Agent,
            serde_json::json!({"kind":"turn","text":"x"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions[0].payload["text"], "rewritten");

    host.dispose_package(&handle).expect("dispose");
    let batch = host
        .dispatch(request(
            RootKind::Agent,
            serde_json::json!({"kind":"turn","text":"x"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions[0].payload["text"], "x");
}

#[test]
fn middleware_stages_are_watchdog_isolated_per_stage() {
    // One runaway middleware stage hits its own watchdog budget while a
    // cooperative root on the same surface keeps working afterwards.
    let host = host(400);
    host.load(
        "memory://mw-wd-root",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "agent", id = "watchdog-base", dispatch = function()
  roots.action("ok", {})
end })
"#,
    )
    .expect("base root");
    host.load(
        "memory://mw-wd-stage",
        r#"
local pi = ...
pi.roots.v1.middleware.register({
  kind = "agent", phase = "event", id = "busy",
  handler = function() while true do end end,
})
"#,
    )
    .expect("busy middleware");

    let error = host
        .dispatch(request(RootKind::Agent, serde_json::json!({"kind":"turn"})))
        .expect_err("a busy middleware stage must hit the watchdog");
    assert!(
        matches!(error, HostError::Timeout(400)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn module_version_conflicts_are_deterministic_on_this_surface() {
    let host = host(5_000);
    host.load(
        "memory://mod-a",
        r#"
local pi = ...
pi.roots.v1.module.define({
  name = "shared", version = "1.0.0",
  factory = function() return { value = 1 } end,
})
"#,
    )
    .expect("first module");
    let conflict = host
        .load(
            "memory://mod-b",
            r#"
local pi = ...
pi.roots.v1.module.define({
  name = "shared", version = "1.0.0",
  factory = function() return { value = 2 } end,
})
"#,
        )
        .expect_err("same name@version from another source must conflict");
    assert!(
        matches!(conflict, HostError::Lua(ref message) if message.contains("shared@1.0.0")),
        "unexpected conflict: {conflict:?}"
    );

    // A distinct exact version from another package loads fine, and each
    // consumer resolves the exact version it declared.
    host.load(
        "memory://mod-c",
        r#"
local pi = ...
local module = pi.roots.v1.module
module.define({
  name = "shared", version = "2.0.0",
  factory = function() return { value = 2 } end,
})
module.define({
  name = "consumer", version = "1.0.0",
  dependencies = { old = { name = "shared", version = "1.0.0" },
                   new = { name = "shared", version = "2.0.0" } },
  factory = function(deps)
    return { old = deps.old.value, new = deps.new.value }
  end,
})
pi.roots.v1.register({ kind = "agent", id = "module-consumer", dispatch = function()
  local resolved = pi.roots.v1.module.require("consumer", "1.0.0")
  pi.roots.v1.action("versions", { old = resolved.old, new = resolved.new })
end })
"#,
    )
    .expect("versioned consumer");

    let batch = host
        .dispatch(request(RootKind::Agent, serde_json::json!({"kind":"turn"})))
        .expect("dispatch");
    assert_eq!(batch.actions[0].payload["old"], 1);
    assert_eq!(batch.actions[0].payload["new"], 2);
}

#[test]
fn nested_root_dispatch_composes_middleware() {
    // Middleware bound to the nested root kind applies inside a
    // roots.v1.dispatch call, exactly as at the top level.
    let host = host(5_000);
    host.load(
        "memory://mw-nest",
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({ kind = "frontend", id = "nest-front", dispatch = function(snapshot)
  roots.action("rendered", { text = snapshot.event.text })
end })
roots.register({ kind = "application", id = "nest-app", dispatch = function()
  local batch = roots.dispatch("frontend", { kind = "render", text = "inner" })
  for _, action in ipairs(batch.actions) do
    roots.action(action.kind, action.payload)
  end
end })
pi.roots.v1.middleware.register({
  kind = "frontend", phase = "event", id = "nest-tag",
  handler = function(snapshot)
    return { event = { kind = snapshot.event.kind, text = snapshot.event.text .. "-mw" } }
  end,
})
"#,
    )
    .expect("nested package");

    let batch = host
        .dispatch(request(
            RootKind::Application,
            serde_json::json!({"kind":"startup"}),
        ))
        .expect("dispatch");
    assert_eq!(batch.actions.len(), 1);
    assert_eq!(batch.actions[0].kind, "rendered");
    assert_eq!(batch.actions[0].payload["text"], "inner-mw");
}
