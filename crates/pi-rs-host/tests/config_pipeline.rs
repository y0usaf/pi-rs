#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::settings_manager::{SettingsManager, SettingsManagerCreateOptions};
use pi_rs_host::{Host, HostConfig};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn config(settings: &str) -> String {
    format!("local pi = ...\npi.config.settings({settings})\n")
}

#[test]
fn global_project_trust_cli_and_json_ignorance_matrix() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent) };

    std::fs::write(
        agent.join("config.lua"),
        config("{ theme = 'dark', retry = { enabled = true, maxRetries = 2 }, enabledModels = { 'global' } }")
    ).unwrap();
    std::fs::write(
        cwd.join(".pi/config.lua"),
        config("{ theme = 'light', retry = { enabled = false }, enabledModels = { 'project' } }"),
    )
    .unwrap();

    // Every former JSON configuration entry point conflicts deliberately and is ignored.
    std::fs::write(agent.join("settings.json"), r#"{"theme":"json"}"#).unwrap();
    std::fs::write(agent.join("keybindings.json"), r#"{"app.exit":"ctrl+x"}"#).unwrap();
    std::fs::write(agent.join("models.json"), r#"{"providers":{"evil":{}}}"#).unwrap();
    std::fs::write(agent.join("theme.json"), r#"{"name":"json"}"#).unwrap();
    std::fs::write(cwd.join(".pi/settings.json"), r#"{"theme":"project-json"}"#).unwrap();

    let trusted = SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    );
    assert_eq!(trusted.get_theme().as_deref(), Some("light"));
    assert!(!trusted.get_retry_enabled());
    assert_eq!(trusted.get_retry_settings().max_retries, 2);
    assert_eq!(
        trusted.get_enabled_models(),
        Some(vec!["project".to_owned()])
    );

    let untrusted = SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(false),
        },
    );
    assert_eq!(untrusted.get_theme().as_deref(), Some("dark"));
    assert!(untrusted.get_retry_enabled());
    assert_eq!(
        untrusted.get_enabled_models(),
        Some(vec!["global".to_owned()])
    );

    let mut cli = trusted;
    cli.apply_overrides(&serde_json::Map::from_iter([
        ("theme".to_owned(), serde_json::json!("cli")),
        ("retry".to_owned(), serde_json::json!({"maxRetries": 9})),
    ]));
    assert_eq!(cli.get_theme().as_deref(), Some("cli"));
    assert!(!cli.get_retry_enabled());
    assert_eq!(cli.get_retry_settings().max_retries, 9);
}

#[test]
fn failed_partial_declarations_and_reload_roll_back_atomically() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(cwd.join(".pi")).unwrap();
    std::fs::create_dir_all(&agent).unwrap();

    std::fs::write(agent.join("config.lua"), config("{ theme = 'dark' }")).unwrap();
    std::fs::write(
        cwd.join(".pi/config.lua"),
        config("{ hideThinkingBlock = false }"),
    )
    .unwrap();
    let mut manager = SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions {
            project_trusted: Some(true),
        },
    );
    assert_eq!(manager.get_theme().as_deref(), Some("dark"));

    // A declaration before the error is not published, and the concurrently valid
    // global edit is not mixed into the old project snapshot.
    std::fs::write(agent.join("config.lua"), config("{ theme = 'light' }")).unwrap();
    std::fs::write(
        cwd.join(".pi/config.lua"),
        "local pi = ...\npi.config.settings({ hideThinkingBlock = true })\nerror('project exploded')\n",
    ).unwrap();
    let error = manager.try_reload().unwrap_err().to_string();
    assert!(error.contains("project exploded"), "{error}");
    assert_eq!(manager.get_theme().as_deref(), Some("dark"));
    assert!(!manager.get_hide_thinking_block());

    std::fs::write(
        cwd.join(".pi/config.lua"),
        config("{ hideThinkingBlock = true }"),
    )
    .unwrap();
    manager.try_reload().unwrap();
    assert_eq!(manager.get_theme().as_deref(), Some("light"));
    assert!(manager.get_hide_thinking_block());

    std::fs::write(
        agent.join("config.lua"),
        "local pi = ...\npi.config.settings({ theme = 'paper' })\nerror('global exploded')\n",
    )
    .unwrap();
    let fresh = SettingsManager::create(
        &cwd,
        Some(agent),
        SettingsManagerCreateOptions {
            project_trusted: Some(false),
        },
    );
    assert_eq!(
        fresh.get_theme(),
        None,
        "partial startup declaration leaked"
    );
}

#[test]
fn repeated_interactive_mutations_are_byte_idempotent_and_preserve_user_code() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    let user = "local pi = ...\n-- retained user declaration\npi.config.settings({ quietStartup = true })\n";
    std::fs::write(agent.join("config.lua"), user).unwrap();

    let mut manager = SettingsManager::create(
        &cwd,
        Some(agent.clone()),
        SettingsManagerCreateOptions::default(),
    );
    manager.set_theme("light");
    manager.set_enabled_models(Some(&["a".to_owned(), "b".to_owned()]));
    let once = std::fs::read_to_string(agent.join("config.lua")).unwrap();
    manager.set_theme("light");
    manager.set_enabled_models(Some(&["a".to_owned(), "b".to_owned()]));
    let twice = std::fs::read_to_string(agent.join("config.lua")).unwrap();

    assert_eq!(once, twice);
    assert!(twice.starts_with(user));
    assert_eq!(twice.matches(pi_rs_host::config::MANAGED_BEGIN).count(), 1);
    assert!(!agent.join("settings.json").exists());
    let effective = pi_rs_host::config::evaluate(&twice, "config.lua").unwrap();
    assert_eq!(effective.settings["quietStartup"], true);
    assert_eq!(effective.settings["theme"], "light");
    assert_eq!(
        effective.settings["enabledModels"],
        serde_json::json!(["a", "b"])
    );
    let managed = pi_rs_host::config::managed_settings(&twice).unwrap();
    assert!(!managed.contains_key("quietStartup"));
}

#[test]
fn file_backed_extension_uses_the_same_declaration_surface() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent) };
    let host = Host::new(HostConfig {
        cwd: Some(root.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let path = format!(
        "{}/../../examples/extensions/config-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).unwrap();
    let result = host.call_command("config-demo", "").unwrap().unwrap();
    assert_eq!(result["exit"], serde_json::json!(["ctrl+d", "ctrl+q"]));
    assert_eq!(result["model"], "demo-model");
    assert_eq!(result["dark"], true);
    assert_eq!(result["enabled"], serde_json::json!(["demo.lua"]));
}

#[test]
fn public_reload_reports_attributed_error_and_keeps_live_snapshot() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let project = root.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(project.join(".pi")).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent) };
    std::fs::write(agent.join("config.lua"), config("{ theme = 'dark' }")).unwrap();
    std::fs::write(
        project.join(".pi/config.lua"),
        config("{ hideThinkingBlock = false }"),
    )
    .unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(project.to_string_lossy().into_owned()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .unwrap();
    host.load(
        "reload-probe.lua",
        r#"
local pi = ...
pi.register_command("config-reload-probe", { handler = function()
  local before = pi.config.snapshot()
  local ok, err = pcall(pi.config.reload)
  local after = pi.config.snapshot()
  return { ok = ok, error = tostring(err), before = before, after = after, theme = pi.settings.theme() }
end })
"#,
    )
    .unwrap();

    std::fs::write(agent.join("config.lua"), config("{ theme = 'light' }")).unwrap();
    std::fs::write(
        project.join(".pi/config.lua"),
        "local pi = ...\npi.config.settings({ hideThinkingBlock = true })\nerror('broken project config')\n",
    )
    .unwrap();
    let result = host
        .call_command("config-reload-probe", "")
        .unwrap()
        .unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains(".pi/config.lua"));
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("broken project config")
    );
    assert_eq!(result["before"], result["after"]);
    assert_eq!(result["theme"], "dark");
}
