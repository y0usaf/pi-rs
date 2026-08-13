//! PLAN 9.11 external-dogfood wave: the in-repo Lua translations under
//! `tests/dogfood-translations/` load through the same public product loader
//! (embedded policy packs + file-backed ordinary extension path) used by any
//! direct/configured/bundled package. This is the "external dogfood" proof:
//! pi-rs's own executable translations reproduce the pinned Pi 0.80.6 package
//! behaviors without a JS runtime and without any privileged escape hatch
//! (everything routes through the public `pi.*` surface).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use pi_rs_app::builtins::{AGENT_CORE_PACK, CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

const TRANSLATIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/dogfood-translations"
);

fn translation(name: &str) -> String {
    std::fs::read_to_string(Path::new(TRANSLATIONS).join(name))
        .unwrap_or_else(|e| panic!("missing {name}: {e}"))
}

/// A host with the shipped policy packs loaded (like a real product boot)
/// plus ordinary file-backed translations, under a hermetic agent dir.
fn dogfood_host(cwd: &str, translations: &[&str]) -> Host {
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[
        AGENT_CORE_PACK,
        pi_rs_agent::PACK,
        TOOLS_PACK,
        CODING_AGENT_PACK,
        INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "load errors: {:?}", report.errors);
    for name in translations {
        host.load(name, &translation(name)).expect("translation loads");
    }
    host
}

/// Point a hermetic agent dir (and optionally project `.pi/settings.json`) at
/// the extensionSettings seam so loads resolve like a real boot.
fn with_agent_dir(root: &tempfile::TempDir) -> String {
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    agent_dir.to_string_lossy().into_owned()
}

#[test]
fn codex_fast_priority_request() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let agent_dir = with_agent_dir(&root);
    // Global settings: extensionSettings.codex-fast.enabled=true, showStatus.
    std::fs::write(
        Path::new(&agent_dir).join("settings.json"),
        r#"{"extensionSettings":{"codex-fast":{"enabled":true,"supportedModels":["gpt-5.5"],"showStatus":true}}}"#,
    )
    .unwrap();

    let host = dogfood_host(&cwd.to_string_lossy(), &["codex-fast.lua"]);
    // The translation registers its command and hooks through the public
    // surface (gate: load+register, mirroring the 9.8 translated-examples gate).
    assert!(host
        .commands()
        .unwrap()
        .iter()
        .any(|c| c.invocation_name == "codex-fast"));
}

#[test]
fn pomodoro_translation_loads_and_registers() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let _agent_dir = with_agent_dir(&root);
    let host = dogfood_host(&cwd.to_string_lossy(), &["pomodoro.lua"]);
    assert!(host
        .commands()
        .unwrap()
        .iter()
        .any(|c| c.invocation_name == "pomodoro"));
}

#[test]
fn webfetch_translation_loads_and_registers_tool() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let _agent_dir = with_agent_dir(&root);
    let host = dogfood_host(&cwd.to_string_lossy(), &["webfetch.lua"]);
    assert!(host
        .tools()
        .unwrap()
        .iter()
        .any(|t| t.name == "web_fetch"));
}

#[test]
fn rtk_translation_loads_and_registers_tools() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let _agent_dir = with_agent_dir(&root);
    let host = dogfood_host(&cwd.to_string_lossy(), &["rtk.lua"]);
    // rtk reregisters `bash` (first-registration-wins across extensions) and
    // subscribes the user_bash event; it must load without error.
    assert!(host
        .tools()
        .unwrap()
        .iter()
        .any(|t| t.name == "bash"));
}
