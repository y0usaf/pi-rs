//! PLAN 9.6 accept matrix at the interactive seam, self-contained (no shared
//! harness dependency): precedence, trust, CLI-side keybindings/theme wiring,
//! failed/partial declaration rollback, and repeated mutation round-trips
//! through the same /reload path the product uses.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Mutex;

use pi_rs_app::builtins::{CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

/// Process-global serializer for `PI_CODING_AGENT_DIR` (read at `Host::new`).
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn host(cwd: &str, project_trusted: bool) -> Host {
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_owned()),
        project_trusted,
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        TOOLS_PACK,
        CODING_AGENT_PACK,
        INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

const GLOBAL_CONFIG: &str = "local pi = ...\npi.config.settings({ theme = 'dark' })\npi.config.keybindings({ ['app.exit'] = 'ctrl+x' })\n";
const PROJECT_CONFIG: &str = "local pi = ...\npi.config.settings({ theme = 'light' })\npi.config.keybindings({ ['app.message.followUp'] = 'alt+f' })\n";
const BROKEN_PROJECT: &str = "local pi = ...\npi.config.settings({ theme = 'paper' })\npi.config.keybindings({ ['app.message.followUp'] = 'alt+g' })\nerror('broken project config')\n";
const NEXT_PROJECT: &str = "local pi = ...\npi.config.settings({ theme = 'dark' })\npi.config.keybindings({ ['app.message.followUp'] = 'alt+g' })\n";

fn stub_model() -> serde_json::Value {
    serde_json::json!({
        "id": "claude-parity-1", "name": "Claude Parity",
        "provider": "anthropic", "api": "anthropic-messages",
        "reasoning": false, "contextWindow": 200000, "maxTokens": 8192,
        "input": ["text"],
        "cost": { "input": 3, "output": 15, "cacheRead": 0.3, "cacheWrite": 3.75 },
        "baseUrl": "http://127.0.0.1:1"
    })
}

#[test]
fn config_lua_keybindings_and_theme_flow_through_startup_and_atomic_reload() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("project");
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    // Global + trusted project config.lua declarations; no JSON settings files.
    std::fs::write(agent_dir.join("config.lua"), GLOBAL_CONFIG).unwrap();
    let project_settings = cwd.join(".pi/config.lua");
    std::fs::write(&project_settings, PROJECT_CONFIG).unwrap();

    unsafe {
        std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir);
        std::env::set_var("PI_OFFLINE", "1");
    }
    let trusted = host(&cwd.to_string_lossy(), true);

    // The probe mutates the trusted project config, then publishes it through
    // the same /reload path the interactive frontend uses.
    let reload = |host: &Host, settings_after: &str| {
        let request = serde_json::json!({
            "colorMode": "truecolor", "version": "0.79.0",
            "cwd": cwd, "home": temp.path(), "agentDir": agent_dir,
            "settingsPath": project_settings,
            "settingsAfter": settings_after,
            "model": stub_model(),
        });
        host.call_command("interactive-reload-behavior", &request.to_string())
            .unwrap()
            .unwrap()
    };

    // Startup (no request.theme): the trusted project wins theme precedence,
    // the global keybinding survives, and the project keybinding lands in the
    // interactive runtime's effective key table.
    let startup = reload(&trusted, PROJECT_CONFIG);
    assert_eq!(startup["theme"], "light");
    assert_eq!(startup["keybindings"]["appExit"], "ctrl+x");
    assert_eq!(startup["keybindings"]["followUp"], "alt+f");
    assert_eq!(startup["failed"], false);

    // Failed/partial declaration: the whole next graph is rejected. The
    // declaration before the error is not published; the live keybindings and
    // theme stay on the previous complete snapshot.
    let failed = reload(&trusted, BROKEN_PROJECT);
    assert_eq!(failed["failed"], true);
    assert!(
        failed["errorText"]
            .as_str()
            .unwrap()
            .contains("broken project config")
    );
    assert_eq!(failed["theme"], "light");
    assert_eq!(failed["keybindings"]["appExit"], "ctrl+x");
    assert_eq!(failed["keybindings"]["followUp"], "alt+f");

    // Repeated mutation round-trip: a valid project edit publishes the whole
    // next graph — the theme override and the keybinding change land together,
    // and the untouched global keybinding is preserved by the merge.
    let applied = reload(&trusted, NEXT_PROJECT);
    assert_eq!(applied["failed"], false);
    assert_eq!(applied["theme"], "dark");
    assert_eq!(applied["keybindings"]["appExit"], "ctrl+x");
    assert_eq!(applied["keybindings"]["followUp"], "alt+g");

    // Trust: an untrusted project contributes nothing — global theme and
    // keybindings only, even after a reload over the same project file.
    let untrusted = host(&cwd.to_string_lossy(), false);
    let untrusted_result = reload(&untrusted, PROJECT_CONFIG);
    assert_eq!(untrusted_result["theme"], "dark");
    assert_eq!(untrusted_result["keybindings"]["appExit"], "ctrl+x");
    // The untrusted project's "alt+f" is rejected; the builtin default
    // ("alt+enter") remains in effect rather than the project binding.
    assert_eq!(
        untrusted_result["keybindings"]["followUp"],
        serde_json::Value::String("alt+enter".to_owned())
    );
}
