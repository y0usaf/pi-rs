#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Deterministic fixture tests for resource discovery (PLAN 9.7): provenance,
//! precedence, dedupe, toggles, reload, load order, trust, and attribution.
//! Every resolution is a pure function of the filesystem + settings snapshot.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use pi_rs_host::resource_loader::{Origin, ResourceLoader};
use pi_rs_host::settings_manager::{SettingsManager, SettingsManagerCreateOptions};

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn loader(cwd: &Path, agent_dir: &Path, trusted: bool) -> ResourceLoader {
    let settings = Arc::new(Mutex::new(SettingsManager::create(
        cwd,
        Some(agent_dir.to_path_buf()),
        SettingsManagerCreateOptions {
            project_trusted: Some(trusted),
        },
    )));
    ResourceLoader::new(&cwd.to_string_lossy(), &agent_dir.to_string_lossy(), settings)
}

fn no_selectors() -> BTreeMap<String, (Vec<String>, Vec<String>)> {
    BTreeMap::new()
}

/// Build a standard fixture: agent (user) and project roots with all four
/// resource kinds, including a collision (same-named file in both scopes).
fn build_fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let agent = root.join("agent");
    let cwd = root.join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();

    // User extensions: one file + one init.lua directory.
    write(&agent.join("extensions/user-tool.lua"), "return {}\n");
    write(&agent.join("extensions/kit/init.lua"), "return {}\n");

    // Project extensions: a collision file and a package.json manifest dir.
    write(&cwd.join(".pi/extensions/shared.lua"), "return {}\n");
    write(&agent.join("extensions/shared.lua"), "return {}\n");
    write(
        &cwd.join(".pi/extensions/manifested/package.json"),
        r#"{"pi":{"extensions":["init.lua"]}}"#,
    );
    write(&cwd.join(".pi/extensions/manifested/init.lua"), "return {}\n");

    // Skills: SKILL.md in a subdir + a top-level md in "pi" mode.
    write(&agent.join("skills/general/SKILL.md"), "# General\n");
    write(&agent.join("skills/loose.md"), "# Loose\n");

    // Prompts: md files.
    write(&agent.join("prompts/review.md"), "review\n");
    write(&cwd.join(".pi/prompts/review.md"), "project review\n");

    // Themes: json files with a collision.
    write(&agent.join("themes/dark.json"), r##"{"accent":"#111111"}"##);
    write(&cwd.join(".pi/themes/dark.json"), r##"{"accent":"#222222"}"##);
    (agent, cwd)
}

#[test]
fn precedence_project_wins_over_user_and_package_last() {
    let root = tempfile::tempdir().unwrap();
    let (agent, cwd) = build_fixture(root.path());
    let loader = loader(&cwd, &agent, true);

    let paths = loader.resolve(&no_selectors());

    // Collision: project extension wins over user (first in resolved order).
    let ext_names: Vec<String> = paths
        .extensions
        .iter()
        .map(|r| r.path.clone())
        .collect();
    assert!(ext_names.iter().any(|p| p.contains(".pi/extensions/shared.lua")), "{ext_names:?}");
    let shared_position = ext_names
        .iter()
        .position(|p| p.contains(".pi/extensions/shared.lua"))
        .unwrap();
    let user_position = ext_names
        .iter()
        .position(|p| p.contains("agent/extensions/shared.lua"))
        .unwrap();
    assert!(shared_position < user_position, "project beats user: {ext_names:?}");

    // Theme collision: project theme wins.
    let dark = paths
        .themes
        .iter()
        .find(|r| r.path.ends_with("dark.json"))
        .unwrap();
    assert!(dark.path.contains(".pi/themes/dark.json"), "{}", dark.path);

    // Prompt collision: project wins.
    let review = paths
        .prompts
        .iter()
        .find(|r| r.path.ends_with("review.md"))
        .unwrap();
    assert!(review.path.contains(".pi/prompts"), "{}", review.path);

    // All resources carry attribution metadata.
    let theme = &paths.themes[0];
    assert_eq!(theme.metadata.origin, Origin::TopLevel);
    assert_eq!(theme.metadata.source, "auto");
    assert_eq!(theme.metadata.scope, pi_rs_host::packages::Scope::Project);
    assert!(paths.themes.iter().all(|r| r.enabled), "auto-discovered = enabled");

    // Load order: project-local (rank 0) then project-auto (1), user-auto (3).
    // Everything here is auto except package resources; assert rank monotonicity
    // by sorting precedence: project entries precede user entries.
    let first_project = paths.extensions[0].metadata.scope;
    assert_eq!(first_project, pi_rs_host::packages::Scope::Project);
}

#[test]
fn all_four_kinds_discovered_with_manifest_entries() {
    let root = tempfile::tempdir().unwrap();
    let (agent, cwd) = build_fixture(root.path());
    let loader = loader(&cwd, &agent, true);
    let paths = loader.resolve(&no_selectors());

    assert!(!paths.extensions.is_empty());
    assert!(
        paths
            .extensions
            .iter()
            .any(|r| r.path.contains("manifested/init.lua")),
        "package.json pi manifest extension resolved"
    );
    assert!(paths.skills.iter().any(|r| r.path.ends_with("SKILL.md")));
    assert!(paths.skills.iter().any(|r| r.path.ends_with("loose.md")));
    assert!(!paths.prompts.is_empty());
    assert!(!paths.themes.is_empty());

    // SKILL.md dir discovery: no nested .md files at the root of a skills dir
    // unless the file itself is the entry (Pi mode).
    assert_eq!(
        paths.skills.iter().filter(|r| r.path.ends_with("general/SKILL.md")).count(),
        1
    );
}

#[test]
fn trust_gate_skips_project_resources() {
    let root = tempfile::tempdir().unwrap();
    let (agent, cwd) = build_fixture(root.path());
    let loader = loader(&cwd, &agent, false);
    let paths = loader.resolve(&no_selectors());

    assert!(
        paths.extensions.iter().all(|r| !r.path.contains(".pi/")),
        "untrusted project resources skipped: {:?}",
        paths.extensions
    );
    assert!(paths.themes.iter().all(|r| !r.path.contains(".pi/")));
    assert!(paths.prompts.iter().all(|r| !r.path.contains(".pi/")));
    // User resources remain.
    assert!(paths.extensions.iter().any(|r| r.path.contains("agent/extensions")));
}

#[test]
fn toggles_via_settings_overrides_and_selectors() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();
    write(&agent.join("extensions/a.lua"), "return {}\n");
    write(&agent.join("extensions/b.lua"), "return {}\n");
    write(&agent.join("skills/one/SKILL.md"), "# one\n");

    let settings = Arc::new(Mutex::new(SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    )));
    // Top-level settings entries: skills entry with a force-exclude pattern.
    // Like Pi, resource toggles live in the settings file (global/user scope),
    // not in ephemeral apply_overrides, which only touches the merged view.
    settings
        .lock()
        .unwrap()
        .set_skill_paths(&["-skills/one".to_owned()]);
    let loader = ResourceLoader::new(&cwd.to_string_lossy(), &agent.to_string_lossy(), settings);
    let paths = loader.resolve(&no_selectors());
    let one = paths.skills.iter().find(|r| r.path.ends_with("SKILL.md")).unwrap();
    assert!(!one.enabled, "force-exclude toggle disables the skill");

    // Selectors (config.enable/disable) force the enabled flag the other way.
    let mut selectors = BTreeMap::new();
    selectors.insert(
        "skills".to_owned(),
        (
            vec!["skills/one".to_owned()],
            Vec::new(),
        ),
    );
    let paths = loader.resolve(&selectors);
    let one = paths.skills.iter().find(|r| r.path.ends_with("SKILL.md")).unwrap();
    assert!(one.enabled, "selector force-include re-enables the skill");
}

#[test]
fn package_resources_resolve_with_package_origin_and_collide_last() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();

    // A local package with a pi manifest: manifest-declared extensions and
    // themes (Pi semantics: a pi manifest means convention dirs are NOT
    // scanned for kinds the manifest does not mention).
    let package = root.path().join("pkg");
    write(
        &package.join("package.json"),
        r#"{"pi":{"extensions":["init.lua"],"themes":["themes/dark.json"]}}"#,
    );
    write(&package.join("init.lua"), "return {}\n");
    write(&package.join("themes/dark.json"), r##"{"accent":"#333333"}"##);
    write(&package.join("themes/hidden.json"), r##"{"accent":"#999999"}"##);

    let settings = Arc::new(Mutex::new(SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    )));
    settings
        .lock()
        .unwrap()
        .set_packages(vec![serde_json::json!(package.to_string_lossy())]);
    let loader = ResourceLoader::new(&cwd.to_string_lossy(), &agent.to_string_lossy(), settings.clone());
    let paths = loader.resolve(&no_selectors());

    let package_theme = paths
        .themes
        .iter()
        .find(|r| r.path.ends_with("dark.json"))
        .unwrap();
    assert_eq!(package_theme.metadata.origin, Origin::Package);
    assert_eq!(package_theme.metadata.source, package.to_string_lossy());
    let package_ext = paths
        .extensions
        .iter()
        .find(|r| r.path.ends_with("pkg/init.lua"))
        .unwrap();
    assert_eq!(package_ext.metadata.origin, Origin::Package);

    // A manifest-less package: convention dirs are scanned (skills here),
    // and the undeclared themes/hidden.json stays out.
    let plain = root.path().join("plain");
    write(&plain.join("skills/general/SKILL.md"), "# General\n");
    settings
        .lock()
        .unwrap()
        .set_packages(vec![
            serde_json::json!(package.to_string_lossy()),
            serde_json::json!(plain.to_string_lossy()),
        ]);
    let paths = loader.resolve(&no_selectors());
    assert!(paths.skills.iter().any(|r| r.path.ends_with("plain/skills/general/SKILL.md")));
    assert!(!paths.themes.iter().any(|r| r.path.ends_with("hidden.json")));
    let dark = paths.themes.iter().find(|r| r.path.ends_with("dark.json")).unwrap();
    assert_eq!(dark.metadata.origin, Origin::Package);
    // A user theme with the same canonical name wins the collision.
    write(&agent.join("themes/dark.json"), r##"{"accent":"#444444"}"##);
    let paths = loader.resolve(&no_selectors());
    let dark = paths
        .themes
        .iter()
        .find(|r| r.path.ends_with("dark.json"))
        .unwrap();
    assert!(
        dark.path.contains("agent/themes"),
        "top-level beats package: {}",
        dark.path
    );
}

#[test]
fn dedupe_keeps_first_canonical_path_and_reload_sees_new_files() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();
    write(&agent.join("themes/a.json"), r#"{"name":"a"}"#);

    // The same physical file is configured twice through different raw
    // strings (project and user scopes; the ".." variant exercises the
    // canonical-path dedupe rather than the exact-string accumulator one).
    let settings = Arc::new(Mutex::new(SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    )));
    settings
        .lock()
        .unwrap()
        .set_theme_paths(&[agent.join("themes/a.json").to_string_lossy().into_owned()]);
    settings
        .lock()
        .unwrap()
        .set_project_theme_paths(&[
            agent.join("themes/../themes/a.json").to_string_lossy().into_owned(),
        ])
        .unwrap();

    let loader = ResourceLoader::new(&cwd.to_string_lossy(), &agent.to_string_lossy(), settings);
    let before = loader.resolve(&no_selectors());
    assert_eq!(before.themes.len(), 1, "same canonical path deduped to one");
    assert!(before.themes[0].path.ends_with("themes/a.json"));

    // Reload: a new file appears in the next resolve (pure re-read).
    write(&agent.join("themes/new.json"), r#"{"name":"new"}"#);
    let after = loader.resolve(&no_selectors());
    assert_eq!(after.themes.len(), 2);
    assert!(after.themes.iter().any(|r| r.path.ends_with("new.json")));
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn lua_packages_surface_resolves_and_declares_resources() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();
    write(&agent.join("themes/dark.json"), r##"{"accent":"#555555"}"##);
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent) };

    let host = pi_rs_host::Host::new(pi_rs_host::HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        project_trusted: true,
        ..pi_rs_host::HostConfig::default()
    })
    .unwrap();
    host.load(
        "resource-probe.lua",
        r##"
local pi = ...
pi.register_command("resource-probe", { handler = function()
  local resolved = pi.packages.resolve()
  local dark = nil
  for _, theme in ipairs(resolved.themes) do
    if theme.path:match("dark%.json$") then dark = theme end
  end
  pi.packages.declare_resource("theme", "custom", { accent = "#f00" })
  local custom = pi.packages.resource("theme", "custom")
  return {
    theme_path = dark and dark.path or nil,
    theme_enabled = dark and dark.enabled or nil,
    custom_accent = custom and custom.accent or nil,
    all = #pi.packages.all_resources("theme"),
  }
end })
"##,
    )
    .unwrap();
    let result = host.call_command("resource-probe", "").unwrap().unwrap();
    assert!(result["theme_path"].as_str().unwrap().contains("agent/themes"));
    assert_eq!(result["theme_enabled"], serde_json::json!(true));
    assert_eq!(result["custom_accent"], serde_json::json!("#f00"));
}
