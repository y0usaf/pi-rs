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

/// A custom `.json` theme discovered from the agent-dir `themes/` convention
/// dir is applied after `/reload` when settings select it.
#[test]
fn reload_applies_custom_disk_theme_after_reload() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("project");
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent_dir).unwrap();
    // A custom theme in the agent-dir themes/ convention dir.
    let custom_theme = agent_dir.join("themes/coral.json");
    std::fs::create_dir_all(custom_theme.parent().unwrap()).unwrap();
    std::fs::write(
        &custom_theme,
        r##"{
  "name": "coral",
  "colors": {
    "accent": "#ff6b6b", "border": "#000000", "borderAccent": "#000000",
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
"##,
    )
    .unwrap();

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

    // Reload switches the settings theme to the custom `coral`.
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
        "settingsAfter": "local pi = ...\npi.config.settings({ theme = 'coral' })\n",
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

    // The custom theme is applied by name after reload.
    assert_eq!(result["theme"], "coral");
    assert_eq!(
        result["status"],
        "Reloaded keybindings, extensions, skills, prompts, themes"
    );
    assert_eq!(result["failed"], false);
}
