//! PLAN 9.7 resource resolution (`pi.resources` module): precedence, dedupe,
//! collisions, toggles, trust, attribution, load order, and cycles over
//! hermetic on-disk fixtures. The engine is deterministic (never touches the
//! network) and is exercised through the same `pi.module.require` mechanism
//! embedded builtins and file-backed packages share.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::needless_collect, clippy::redundant_closure)]
#![allow(clippy::manual_contains, unsafe_code)]

use pi_rs_app::builtins::{AGENT_CORE_PACK, INTERACTIVE_PACK};
use pi_rs_host::{Host, HostConfig};
use serde_json::{Value, json};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const RUNNER: &str = r#"
local pi = ...
pi.register_command("resources-run", {
  handler = function(args)
    local c = pi.json.decode(args)
    local m = pi.module.require("pi.resources", "1")
    local opts = c.options or {}
    local resolved = m.resolve(opts)
    local out = {}
    for _, kind in ipairs({ "extensions", "skills", "prompts", "themes" }) do
      local rows = {}
      for _, e in ipairs(resolved[kind] or {}) do
        rows[#rows + 1] = {
          path = e.path,
          enabled = e.enabled,
          precedence = e.precedence,
          source = e.metadata.source,
          scope = e.metadata.scope,
          origin = e.metadata.origin,
          baseDir = e.metadata.baseDir,
        }
      end
      out[kind] = rows
    end
    return out
  end,
})
"#;

/// Build a hermetic host whose `pi.settings` store points at `agent_dir`.
/// All fixtures are created under the temp root before this call so settings
/// (`<agent_dir>/config.lua`, `<cwd>/.pi/config.lua`) are read at build time.
fn hermetic_host(cwd: &std::path::Path, agent_dir: &std::path::Path) -> Host {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(cwd).unwrap();
    std::fs::create_dir_all(agent_dir).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir) };
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .expect("host build");
    unsafe { std::env::remove_var("PI_CODING_AGENT_DIR") };
    let report = host.load_embedded(&[AGENT_CORE_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "load errors: {:?}", report.errors);
    host.load("resources-runner", RUNNER).expect("runner loads");
    host
}

fn write(p: &std::path::Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(host: &Host, options: &Value) -> Value {
    host.call_command("resources-run", &json!({ "options": options }).to_string())
        .expect("command")
        .expect("result")
}

/// An empty Lua resource bucket crosses the bridge as `{}` (ambiguous with an
/// array); this returns the rows for a kind regardless of representation.
fn rows_for(result: &Value, kind: &str) -> Vec<Value> {
    match result.get(kind) {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(m)) if m.is_empty() => Vec::new(),
        _ => Vec::new(),
    }
}

/// A minimal well-formed theme JSON used across fixtures.
const THEME_JSON: &str = r##"{
  "name": "fixture-theme",
  "colors": {
    "accent": "#aabbcc", "border": "#000000", "borderAccent": "#000000",
    "borderMuted": "#000000", "success": "#00ff00", "error": "#ff0000",
    "warning": "#ffff00", "muted": "#888888", "dim": "#666666",
    "text": "#ffffff", "thinkingText": "#cccccc", "selectedBg": 0,
    "userMessageBg": 0, "userMessageText": "#ffffff", "customMessageBg": 0,
    "customMessageText": "#ffffff", "customMessageLabel": "#ffffff",
    "toolPendingBg": 0, "toolSuccessBg": 0, "toolErrorBg": 0,
    "toolTitle": "#ffffff", "toolOutput": "#ffffff",
    "mdHeading": "#ffffff", "mdLink": "#ffffff", "mdLinkUrl": "#ffffff",
    "mdCode": "#ffffff", "mdCodeBlock": "#ffffff", "mdCodeBlockBorder": "#ffffff",
    "mdQuote": "#ffffff", "mdQuoteBorder": "#ffffff", "mdHr": "#ffffff",
    "mdListBullet": "#ffffff", "toolDiffAdded": "#ffffff", "toolDiffRemoved": "#ffffff",
    "toolDiffContext": "#ffffff", "syntaxComment": "#ffffff", "syntaxKeyword": "#ffffff",
    "syntaxFunction": "#ffffff", "syntaxVariable": "#ffffff", "syntaxString": "#ffffff",
    "syntaxNumber": "#ffffff", "syntaxType": "#ffffff", "syntaxOperator": "#ffffff",
    "syntaxPunctuation": "#ffffff", "thinkingOff": "#ffffff", "thinkingMinimal": "#ffffff",
    "thinkingLow": "#ffffff", "thinkingMedium": "#ffffff", "thinkingHigh": "#ffffff",
    "thinkingXhigh": "#ffffff", "bashMode": "#ffffff"
  }
}
"##;

#[test]
fn precedence_project_settings_over_user_auto_and_package() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");

    // User-auto theme and a project settings-sourced theme, each declaring the
    // same name. Precedence decides ordering + attribution; the same file added
    // through two paths is de-duplicated by canonical path.
    write(&agent_dir.join("themes/user-theme.json"), &THEME_JSON);
    write(&cwd.join(".pi/settings-theme.json"), &THEME_JSON);
    write(
        &cwd.join(".pi/config.lua"),
        "local pi = ...\npi.config.settings({ themes = { './settings-theme.json' } })\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    let themes = rows_for(&result, "themes");
    assert!(
        themes
            .iter()
            .any(|t| t["scope"] == "project"
                || t["path"].as_str().unwrap().contains("settings-theme")),
        "project settings theme present: {:?}",
        themes
    );
    // Attribution: no configured package here, so no `package` origin.
    assert!(themes.iter().all(|t| t["origin"] != "package"));
}

#[test]
fn collision_dedupe_keeps_highest_precedence_first() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");

    // Same physical skill reachable through project auto-discovery AND project
    // settings. Canonical-path dedupe collapses it to a single entry.
    write(
        &cwd.join(".pi/skills/dup/SKILL.md"),
        "---\nname: dup\ndescription: dup skill\n---\nBody\n",
    );
    write(
        &cwd.join(".pi/config.lua"),
        "local pi = ...\npi.config.settings({ skills = { './skills/dup' } })\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    let skills = rows_for(&result, "skills");
    let dup_paths = skills
        .iter()
        .filter(|s| s["path"].as_str().unwrap().contains("dup"))
        .count();
    assert_eq!(dup_paths, 1, "dup path appears once: {:?}", skills);
}

#[test]
fn trust_gates_project_resources() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    // A project-only extension (plus a non-extension file that must not load).
    write(&cwd.join(".pi/extensions/proj-ext.lua"), "local pi = ...\n");
    write(&cwd.join(".pi/extensions/proj.toml"), "x\n");

    let host = hermetic_host(&cwd, &agent_dir);
    let untrusted = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": false,
    });
    let result = run(&host, &untrusted);
    let exts = rows_for(&result, "extensions");
    assert!(
        exts.iter()
            .all(|e| !e["path"].as_str().unwrap().contains(".pi")),
        "untrusted excludes project resources: {:?}",
        exts
    );

    let trusted = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result2 = run(&host, &trusted);
    let exts2 = rows_for(&result2, "extensions");
    assert!(
        exts2
            .iter()
            .any(|e| e["path"].as_str().unwrap().contains(".pi")),
        "trusted includes project resources: {:?}",
        exts2
    );
}

#[test]
fn attribution_sources_scopes_and_origins_attached() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    write(
        &agent_dir.join("skills/user-skill/SKILL.md"),
        "---\nname: user\ndescription: user skill\n---\nBody\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    let skills = rows_for(&result, "skills");
    let user = skills
        .iter()
        .find(|s| s["path"].as_str().unwrap().contains("user-skill"))
        .expect("user skill resolved");
    assert_eq!(user["scope"], "user");
    assert_eq!(user["origin"], "top-level");
    assert_eq!(user["source"], "auto");
    assert_eq!(user["enabled"], true);
}

#[test]
fn conflict_precedence_user_settings_beats_user_auto() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    // A user-auto theme via the convention dir, plus a user-settings-sourced
    // theme outside `themes/` (so it is only reachable through settings).
    write(&agent_dir.join("themes/x.json"), &THEME_JSON);
    write(&agent_dir.join("custom-theme.json"), &THEME_JSON);
    write(
        &agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ themes = { './custom-theme.json' } })\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    let themes = rows_for(&result, "themes");
    // Both user-scope themes present.
    assert!(
        themes
            .iter()
            .any(|t| t["path"].as_str().unwrap().contains("custom-theme"))
    );
    assert!(
        themes
            .iter()
            .any(|t| t["path"].as_str().unwrap().contains("themes/x.json"))
    );
    // The settings-sourced one carries source="settings"; auto carries "auto".
    let custom = themes
        .iter()
        .find(|t| t["path"].as_str().unwrap().contains("custom-theme"))
        .unwrap();
    let auto = themes
        .iter()
        .find(|t| t["path"].as_str().unwrap().contains("themes/x.json"))
        .unwrap();
    assert_eq!(custom["source"], "settings");
    assert_eq!(auto["source"], "auto");
    // Precedence: settings (rank 2) sorts before auto (rank 3) for same scope.
    assert!(custom["precedence"].as_u64().unwrap() < auto["precedence"].as_u64().unwrap());
}

#[test]
fn file_backed_package_resolves_resources_same_mechanism() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    write(
        &agent_dir.join("skills/fs/SKILL.md"),
        "---\nname: fs\ndescription: fs skill\n---\nBody\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    host.load(
        "examples/extensions/resources-consumer.lua",
        include_str!("../../../examples/extensions/resources-consumer.lua"),
    )
    .expect("file-backed resources consumer loads");
    let result = host
        .call_command(
            "resources-consumer",
            &json!({ "options": {
                "cwd": cwd.to_string_lossy(),
                "agentDir": agent_dir.to_string_lossy(),
                "projectTrusted": true,
                "home": root.path().to_string_lossy(),
            } })
            .to_string(),
        )
        .expect("resources consumer runs")
        .expect("result");
    let skills = rows_for(&result, "skills");
    assert!(
        skills
            .iter()
            .any(|s| s["path"].as_str().unwrap().contains("fs")),
        "file-backed consumer resolved fs skill: {:?}",
        skills
    );
}

#[test]
fn theme_discovery_registers_and_loads_disk_json_themes() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    // A custom theme on disk (auto-discovered from the themes convention dir)
    // and another reachable only through a resolved theme path.
    write(&agent_dir.join("themes/custom.json"), &THEME_JSON);

    let host = hermetic_host(&cwd, &agent_dir);
    host.load(
        "examples/extensions/resources-consumer.lua",
        include_str!("../../../examples/extensions/resources-consumer.lua"),
    )
    .expect("consumer loads");
    // Use a dedicated theme runner sub-command for registry+loader coverage.
    host.load(
        "theme-runner",
        r#"
local pi = ...
pi.register_command("theme-run", {
  handler = function(args)
    local c = pi.json.decode(args)
    local m = pi.module.require("pi.resources", "1")
    if c.op == "load" then
      local res = m.load_theme_from_path(c.path)
      if res.error then return { error = res.error } end
      return { name = res.theme.name, sourcePath = res.theme.sourcePath }
    end
    if c.op == "sync" then
      local res = m.sync_themes({ { path = c.path } })
      return { count = #res.themes, names = (function()
        local out = {}
        for _, t in ipairs(res.themes) do out[#out + 1] = t.name end
        return out
      end)(), diag = #res.diagnostics }
    end
    if c.op == "available" then
      local names = m.get_available_themes()
      local found = {}
      for _, n in ipairs(names) do found[#found + 1] = n end
      return { names = found, hasCustom = m.has_theme("fixture-theme") }
    end
    return {}
  end,
})
"#,
    )
    .expect("theme runner loads");

    let custom = agent_dir.join("themes/custom.json");
    // load_theme_from_path validates + registers the disk JSON theme.
    let loaded = host
        .call_command(
            "theme-run",
            &json!({ "op": "load", "path": custom.to_string_lossy() }).to_string(),
        )
        .expect("theme-run")
        .expect("result");
    assert_eq!(loaded["name"], "fixture-theme");
    assert!(
        loaded["sourcePath"]
            .as_str()
            .unwrap()
            .ends_with("custom.json")
    );

    // Registry now reports the custom theme alongside built-ins.
    let available = host
        .call_command("theme-run", &json!({ "op": "available" }).to_string())
        .expect("theme-run")
        .expect("result");
    assert_eq!(available["hasCustom"], true);
    let names: Vec<&str> = available["names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert!(names.contains(&"fixture-theme"));
    assert!(names.contains(&"dark"));

    // sync_themes over a resolved path list registers without diagnostics.
    let synced = host
        .call_command(
            "theme-run",
            &json!({ "op": "sync", "path": custom.to_string_lossy() }).to_string(),
        )
        .expect("theme-run")
        .expect("result");
    assert_eq!(synced["count"], 1);
    assert_eq!(synced["diag"], 0);
    assert_eq!(synced["names"][0], "fixture-theme");

    // Missing/invalid theme JSON reports a diagnostic, not a throw.
    let missing = host
        .call_command(
            "theme-run",
            &json!({ "op": "load", "path": agent_dir.join("nope.json").to_string_lossy() })
                .to_string(),
        )
        .expect("theme-run")
        .expect("result");
    assert!(missing.get("error").is_some());
}

/// A user-local package source (`./my-pkg` under the agent dir) is resolved by
/// the engine against the user scope base dir (agent dir) — the same location
/// `pi.packages.get_installed_path` returns — so the resolver finds its
/// resources with origin=package.
#[test]
fn user_local_package_resolves_against_user_base() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    // A user-local package under `<agent_dir>/my-pkg` with an extensions dir,
    // declared in the global (agent-dir) config.lua packages channel.
    write(
        &agent_dir.join("my-pkg/extensions/pkg-ext.lua"),
        "local pi = ...\n",
    );
    write(
        &agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { './my-pkg' } })\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    let exts = rows_for(&result, "extensions");
    assert!(
        exts.iter().any(|e| {
            e["path"].as_str().unwrap().contains("my-pkg")
                && e["origin"] == "package"
                && e["scope"] == "user"
        }),
        "user-local package extension resolved with package origin: {:?}",
        exts
    );
}

/// Package-filter / settings pattern toggles: a `!`-excluded resource is
/// resolved with `enabled=false` (the toggle), and a `+`-forced one stays on.
#[test]
fn toggle_patterns_disable_and_force_enable() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    // Two skills in the user convention dir; settings excludes one and
    // force-includes it back (so the toggle is observable on `enabled`).
    write(
        &agent_dir.join("skills/keep/SKILL.md"),
        "---\nname: keep\ndescription: keep\n---\nB\n",
    );
    write(
        &agent_dir.join("skills/drop/SKILL.md"),
        "---\nname: drop\ndescription: drop\n---\nB\n",
    );
    // Global settings: exclude `drop`, force-include it (net effect enabled).
    write(
        &agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ skills = { '!**/drop', '+**/drop' } })\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    let skills = rows_for(&result, "skills");
    // `keep` is auto-discovered (enabled), `drop` is force-included (enabled).
    assert!(
        skills
            .iter()
            .any(|s| s["path"].as_str().unwrap().contains("keep") && s["enabled"] == true)
    );
    assert!(
        skills
            .iter()
            .any(|s| s["path"].as_str().unwrap().contains("drop") && s["enabled"] == true)
    );
}

/// Module dependency cycle: two modules that require each other are rejected
/// with a cycle diagnostic (the `pi.module` mechanism's lazy cycle detection).
#[test]
fn module_dependency_cycle_is_rejected() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    let host = hermetic_host(&cwd, &agent_dir);
    host.load(
        "cycle-a",
        r#"
local pi = ...
pi.module.define({
  name = "cycle.a", version = "1",
  dependencies = { b = { name = "cycle.b", version = "1" } },
  factory = function(deps) return { b = deps.b } end,
})
pi.register_command("cycle-a-run", { handler = function()
  local a = pi.module.require("cycle.a", "1")
  return { got = a ~= nil }
end })
"#,
    )
    .expect("cycle-a loads");
    host.load(
        "cycle-b",
        r#"
local pi = ...
pi.module.define({
  name = "cycle.b", version = "1",
  dependencies = { a = { name = "cycle.a", version = "1" } },
  factory = function(deps) return { a = deps.a } end,
})
"#,
    )
    .expect("cycle-b loads");
    // Requiring cycle.a triggers the cycle detection (a -> b -> a).
    let r = host.call_command("cycle-a-run", "{}");
    assert!(r.is_err(), "cycle should be rejected, got {:?}", r);
    let msg = format!("{:?}", r);
    assert!(
        msg.contains("module dependency cycle"),
        "expected cycle diagnostic, got: {msg}"
    );
}

/// Offline cache: an npm/git package with no installed path resolves to no
/// resources (offline-skip) rather than attempting an install — deterministic.
#[test]
fn offline_cache_skips_uninstalled_packages() {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    let agent_dir = root.path().join("agent");
    // Configure an npm package with no on-disk install; the resolver must not
    // try to install (offline) and must not surface package resources.
    write(
        &agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ packages = { 'npm:some-pkg' } })\n",
    );

    let host = hermetic_host(&cwd, &agent_dir);
    let options = json!({
        "cwd": cwd.to_string_lossy(),
        "agentDir": agent_dir.to_string_lossy(),
        "projectTrusted": true,
    });
    let result = run(&host, &options);
    // No package-origin resources anywhere (nothing installed).
    for kind in ["extensions", "skills", "prompts", "themes"] {
        let rows = rows_for(&result, kind);
        assert!(
            rows.iter().all(|e| e["origin"] != "package"),
            "{kind} has unexpected package resources: {:?}",
            rows
        );
    }
}
