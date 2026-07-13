#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::{Host, HostConfig};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn reload_rereads_settings_and_project_context_through_product_policy() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("project");
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let context_path = cwd.join("AGENTS.md");
    std::fs::write(&context_path, "initial project rule").unwrap();
    let settings_path = agent_dir.join("config.lua");
    std::fs::write(
        &settings_path,
        "local pi = ...\npi.config.settings({ theme = 'dark', hideThinkingBlock = false })\n",
    )
    .unwrap();

    unsafe {
        std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir);
        std::env::set_var("PI_OFFLINE", "1");
    }
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..Default::default()
    })
    .unwrap();
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let request = serde_json::json!({
        "theme": "dark",
        "colorMode": "truecolor",
        "version": "0.79.0",
        "cwd": cwd,
        "home": temp.path(),
        "agentDir": agent_dir,
        "contextPath": context_path,
        "contextAfter": "reloaded project rule",
        "settingsPath": settings_path,
        "settingsAfter": "local pi = ...\npi.config.settings({ theme = 'light', hideThinkingBlock = true })\n",
        "model": {
            "id": "claude-parity-1",
            "name": "Claude Parity",
            "provider": "anthropic",
            "api": "anthropic-messages",
            "reasoning": false,
            "contextWindow": 200000,
            "maxTokens": 8192,
            "input": ["text"],
            "cost": { "input": 3, "output": 15, "cacheRead": 0.3, "cacheWrite": 3.75 },
            "baseUrl": "http://127.0.0.1:1"
        }
    });
    let result = host
        .call_command("interactive-reload-behavior", &request.to_string())
        .unwrap()
        .unwrap();

    assert!(
        result["before"]
            .as_str()
            .unwrap()
            .contains("initial project rule")
    );
    assert!(
        !result["before"]
            .as_str()
            .unwrap()
            .contains("reloaded project rule")
    );
    assert!(
        result["after"]
            .as_str()
            .unwrap()
            .contains("reloaded project rule")
    );
    assert!(
        !result["after"]
            .as_str()
            .unwrap()
            .contains("initial project rule")
    );
    assert_eq!(result["theme"], "light");
    assert_eq!(result["hideThinking"], true);
    assert_eq!(
        result["status"],
        "Reloaded keybindings, extensions, skills, prompts, themes"
    );
    assert_eq!(result["failed"], false);
}
