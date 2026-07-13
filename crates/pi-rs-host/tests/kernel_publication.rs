//! PLAN 1.1 canonical package-publication and retained-adapter invariants.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::Path;
use std::time::Duration;

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, HostError, PackageSource};

fn host() -> Host {
    Host::new(HostConfig::default()).expect("host")
}

fn request(root: RootKind) -> DispatchRequest {
    DispatchRequest::new(root, serde_json::json!({}), serde_json::json!({}))
}

fn failed_package(marker: &Path) -> String {
    let marker = serde_json::to_string(&marker.to_string_lossy()).unwrap();
    format!(
        r#"
local pi = ...
local k = pi.kernel.v1
k.resource(function() pi.fs.write_file({marker}, "disposed") end)
k.root({{ kind="session", id="ghost-root", dispatch=function() end }})
k.declare("theme", {{ id="ghost-theme" }})
k.module.define({{ name="ghost.module", version="1", dependencies={{}}, factory=function() return {{}} end }})
pi.declare_package({{ command_visibility="internal" }})
pi.register_role({{ id="ghost-role", role="ghost", active=true, priority=0, handler=function() end }})
pi.register_tool({{ name="ghost-tool", execute=function() return {{}} end }})
pi.register_command("ghost-command", {{ handler=function() end }})
pi.register_provider("ghost-provider", {{ baseUrl="https://ghost.invalid" }})
pi.unregister_provider("prior-provider")
pi.on("ghost-lifecycle", function() return "ghost" end)
pi.events.on("ghost-bus", function() end)
pi.register_render_middleware("ghost-row", {{ name="ghost-renderer", render=function() end }})
pi.register_ui_slot("ghost-slot", {{ name="ghost-ui", render=function() end }})
pi.register_shortcut("ctrl+g", {{ handler=function() end }})
pi.register_flag("ghost-flag", {{ type="string", default="ghost" }})
error("rollback every side effect")
"#
    )
}

fn assert_failed_load_rolled_back(host: &Host, marker: &Path) {
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "disposed");
    assert!(host.tools().unwrap().is_empty());
    assert!(host.commands().unwrap().is_empty());
    assert!(host.flags().unwrap().is_empty());
    assert!(host.roles().unwrap().is_empty());
    assert_eq!(
        host.providers()
            .unwrap()
            .into_iter()
            .map(|provider| provider.name)
            .collect::<Vec<_>>(),
        ["prior-provider"]
    );
    assert!(
        host.emit("ghost-lifecycle", &serde_json::json!({}))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        host.dispatch(request(RootKind::Session)),
        Err(HostError::UnknownRoot(_))
    ));

    host.load(
        "memory://rollback-verifier",
        r#"
local pi = ...
local k = pi.kernel.v1
pi.register_command("verify-rollback", { handler=function()
  local module_ok = pcall(function() k.module.require("ghost.module", "1") end)
  return {
    theme=#k.registered("theme"), tool=#k.registered("tool"),
    command=#k.registered("command") - 1, provider=#k.registered("provider"),
    event=#k.registered("event"), renderer=#k.registered("renderer"),
    ui_slot=#k.registered("ui_slot"), keymap=#k.registered("keymap"),
    flag=#k.registered("flag"), module_ok=module_ok,
    shortcuts=#pi.registered_shortcuts(),
    renderers=#pi.registered_render_middlewares(), slots=#pi.registered_ui_slots(),
  }
end })
"#,
    )
    .unwrap();
    let observed = host.call_command("verify-rollback", "").unwrap().unwrap();
    assert_eq!(
        observed,
        serde_json::json!({
            "theme":0,"tool":0,"command":0,"provider":1,"event":0,
            "renderer":0,"ui_slot":0,"keymap":0,"flag":0,
            "module_ok":false,"shortcuts":0,"renderers":0,"slots":0
        })
    );

    host.load(
        "memory://replacement",
        r#"
local k = (...).kernel.v1
k.root({ kind="session", id="ghost-root", dispatch=function() k.action("replacement", {}) end })
k.declare("theme", { id="ghost-theme" })
k.module.define({ name="ghost.module", version="1", dependencies={}, factory=function() return {} end })
"#,
    )
    .expect("failed package retained no canonical conflict");
    assert_eq!(
        host.dispatch(request(RootKind::Session)).unwrap().actions[0].kind,
        "replacement"
    );
}

#[test]
fn failed_memory_file_and_embedded_loads_rollback_every_side_effect_family() {
    for provenance in ["memory", "file", "embedded"] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join(format!("{provenance}.txt"));
        let source = failed_package(&marker);
        let host = host();
        host.load(
            "memory://prior",
            "local pi=...; pi.register_provider('prior-provider', {baseUrl='https://prior.invalid'})",
        )
        .unwrap();
        let result = match provenance {
            "memory" => host
                .load_package(PackageSource::Memory {
                    key: "memory://failed-all",
                    source: &source,
                })
                .map(|_| ()),
            "file" => {
                let path = directory.path().join("failed-all.lua");
                std::fs::write(&path, &source).unwrap();
                host.load_package(PackageSource::File { path: &path })
                    .map(|_| ())
            }
            "embedded" => host
                .load_package(PackageSource::Embedded {
                    name: "failed-all",
                    source: &source,
                })
                .map(|_| ()),
            _ => unreachable!(),
        };
        assert!(result.is_err(), "{provenance} load must fail");
        assert_failed_load_rolled_back(&host, &marker);
    }
}

#[test]
fn canonical_and_adapter_views_share_state_without_double_publication() {
    let host = host();
    host.load(
        "memory://shared-state",
        r#"
local pi = ...
local k = pi.kernel.v1
pi.register_tool({ name="shared-tool", label="first", execute=function() return {version=1} end })
pi.register_tool({ name="shared-tool", label="second", execute=function() return {version=2} end })
pi.register_provider("shared-provider", { baseUrl="https://first.invalid" })
pi.register_provider("shared-provider", { name="Shared", baseUrl="https://second.invalid" })
pi.register_shortcut("CTRL+S", { handler=function() end })
pi.register_render_middleware("row", { name="shared-renderer", order=2, render=function() end })
pi.register_ui_slot("footer", { name="shared-slot", order=3, render=function() end })
pi.on("shared-event", function() end)
pi.register_command("inspect-shared-state", { handler=function()
  local function ids(kind)
    local out = {}
    for _, declaration in ipairs(k.registered(kind)) do
      out[#out+1] = declaration.declaration_id
    end
    return out
  end
  return {
    tools=ids("tool"), providers=ids("provider"), keymaps=ids("keymap"),
    renderers=ids("renderer"), slots=ids("ui_slot"), events=ids("event"),
    adapter_tools=#pi.registered_tools(), adapter_shortcuts=#pi.registered_shortcuts(),
    adapter_renderers=#pi.registered_render_middlewares(), adapter_slots=#pi.registered_ui_slots(),
  }
end })
"#,
    )
    .unwrap();

    assert_eq!(host.tools().unwrap().len(), 1);
    assert_eq!(host.tools().unwrap()[0].meta["label"], "second");
    assert_eq!(
        host.call_tool("shared-tool", "call", &serde_json::json!({}))
            .unwrap()["version"],
        2
    );
    let providers = host.providers().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].config["baseUrl"], "https://second.invalid");
    assert_eq!(providers[0].config["name"], "Shared");

    let observed = host
        .call_command("inspect-shared-state", "")
        .unwrap()
        .unwrap();
    assert_eq!(observed["tools"], serde_json::json!(["shared-tool"]));
    assert_eq!(
        observed["providers"],
        serde_json::json!(["shared-provider"])
    );
    assert_eq!(observed["keymaps"], serde_json::json!(["ctrl+s"]));
    assert_eq!(
        observed["renderers"],
        serde_json::json!(["row\0shared-renderer"])
    );
    assert_eq!(
        observed["slots"],
        serde_json::json!(["footer\0shared-slot"])
    );
    assert_eq!(observed["events"].as_array().unwrap().len(), 1);
    for key in [
        "adapter_tools",
        "adapter_shortcuts",
        "adapter_renderers",
        "adapter_slots",
    ] {
        assert_eq!(observed[key], 1, "{key}");
    }

    let conflict = host
        .load(
            "memory://canonical-conflict",
            "local k=(...).kernel.v1; k.declare('tool', {id='shared-tool'})",
        )
        .unwrap_err()
        .to_string();
    assert!(conflict.contains("memory://canonical-conflict <> memory://shared-state"));
    assert_eq!(
        host.tools().unwrap().len(),
        1,
        "failed conflict published nothing"
    );
}

#[test]
fn scoped_provider_removal_has_one_canonical_and_adapter_view() {
    let host = host();
    host.load(
        "memory://provider-owner",
        "local pi=...; pi.register_provider('shared', {baseUrl='https://owner.invalid'})",
    )
    .unwrap();
    let remover = host
        .load_package(PackageSource::Memory {
            key: "memory://provider-remover",
            source: r#"
local pi=...
pi.unregister_provider("shared")
pi.register_command("provider-count", {handler=function()
  return {canonical=#pi.kernel.v1.registered("provider")}
end})
"#,
        })
        .unwrap();
    assert!(host.providers().unwrap().is_empty());
    let observed = host.call_command("provider-count", "").unwrap().unwrap();
    assert_eq!(observed["canonical"], 0);

    host.dispose_package(&remover).unwrap();
    assert_eq!(host.providers().unwrap().len(), 1);
}

#[test]
fn package_side_effects_become_observable_only_after_success() {
    let host = host();
    let loader = host.clone();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let join = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        loader.load(
            "memory://slow-failure",
            r#"
local pi = ...
pi.register_tool({name="never-visible", execute=function() end})
pi.sleep(100)
error("late failure")
"#,
        )
    });
    started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(20));

    let observer = host.clone();
    let (observed_tx, observed_rx) = std::sync::mpsc::sync_channel(1);
    let observe = std::thread::spawn(move || observed_tx.send(observer.tools()).unwrap());
    assert!(
        observed_rx.recv_timeout(Duration::from_millis(30)).is_err(),
        "the VM must not expose an in-flight package"
    );
    assert!(join.join().unwrap().is_err());
    assert!(
        observed_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .is_empty()
    );
    observe.join().unwrap();
}

const NEUTRAL_ADAPTER_PACKAGE: &str = r#"
local pi = ...
local k = pi.kernel.v1
k.resource(function() end)
pi.register_tool({name="neutral-tool", execute=function() return {ok=true} end})
pi.register_command("neutral-command", {handler=function() return "ok" end})
pi.register_provider("neutral-provider", {baseUrl="https://neutral.invalid"})
pi.on("neutral-event", function() return "ok" end)
k.root({kind="application", id="neutral", dispatch=function() k.action("ok", {}) end})
"#;

fn neutral_adapter_result(package: PackageSource<'_>) -> serde_json::Value {
    let host = host();
    let handle = host.load_package(package).unwrap();
    let before = host.scope_stats(&handle).unwrap();
    let result = serde_json::json!({
        "tools": host.tools().unwrap().len(),
        "tool_result": host.call_tool("neutral-tool", "call", &serde_json::json!({})).unwrap(),
        "commands": host.commands().unwrap().len(),
        "command_result": host.call_command("neutral-command", "").unwrap(),
        "providers": host.providers().unwrap().len(),
        "events": host.emit("neutral-event", &serde_json::json!({})).unwrap().len(),
        "actions": host.dispatch(request(RootKind::Application)).unwrap().actions.len(),
        "resources": before.resources,
    });
    host.dispose_package(&handle).unwrap();
    let after = host.scope_stats(&handle).unwrap();
    assert!(after.disposed && after.cancelled && after.resources == 0);
    result
}

#[test]
fn embedded_and_file_packages_have_equal_adapter_capability_and_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("neutral-adapters.lua");
    std::fs::write(&path, NEUTRAL_ADAPTER_PACKAGE).unwrap();
    let embedded = neutral_adapter_result(PackageSource::Embedded {
        name: "neutral-adapters",
        source: NEUTRAL_ADAPTER_PACKAGE,
    });
    let file = neutral_adapter_result(PackageSource::File { path: &path });
    assert_eq!(embedded, file);
}
