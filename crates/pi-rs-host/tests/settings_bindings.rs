//! `pi.settings` — the settings-manager port bound per VM. Pins the
//! spec's getBlockImages/setBlockImages semantics through the public
//! Lua surface: the default, project-scope merge (`.pi/settings.json`),
//! and the granular global persistence of setBlockImages.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

/// Both tests point `PI_CODING_AGENT_DIR` at their own temp dir;
/// serialize so the process-global env doesn't race.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RUNNER: &str = r#"
local pi = ...
pi.register_command("settings-read", {
  handler = function() return { blocked = pi.settings.block_images() } end,
})
pi.register_command("settings-write", {
  handler = function(args)
    pi.settings.set_block_images(pi.json.decode(args).blocked)
    return { blocked = pi.settings.block_images() }
  end,
})
"#;

#[test]
fn block_images_reads_merged_settings_and_persists_globally() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let agent_dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    let project = tempfile::tempdir().unwrap();
    let cwd = project.path().to_string_lossy().into_owned();

    // Default: false (spec `getBlockImages`).
    let host = Host::new(HostConfig {
        cwd: Some(cwd.clone()),
        ..HostConfig::default()
    })
    .expect("host");
    host.load("settings-test", RUNNER).expect("runner loads");
    let got = host.call_command("settings-read", "").unwrap().unwrap();
    assert_eq!(got["blocked"], false);

    // setBlockImages(true) answers dynamically and persists to the
    // *global* scope's settings.json under the images nested field.
    let got = host
        .call_command("settings-write", r#"{"blocked":true}"#)
        .unwrap()
        .unwrap();
    assert_eq!(got["blocked"], true);
    let global: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(agent_dir.path().join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(global["images"]["blockImages"], true);

    // Project settings merge over global (spec: one-level deep merge).
    std::fs::create_dir_all(project.path().join(".pi")).unwrap();
    std::fs::write(
        project.path().join(".pi/settings.json"),
        r#"{"images":{"blockImages":true}}"#,
    )
    .unwrap();
    std::fs::write(agent_dir.path().join("settings.json"), "{}").unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(cwd),
        ..HostConfig::default()
    })
    .expect("host");
    host.load("settings-test", RUNNER).expect("runner loads");
    let got = host.call_command("settings-read", "").unwrap().unwrap();
    assert_eq!(got["blocked"], true, "project scope merges: {got}");
}

#[test]
fn settings_demo_example_exercises_the_public_surface() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let agent_dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    let cwd_dir = tempfile::tempdir().unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(cwd_dir.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .expect("host");
    let path = format!(
        "{}/../../examples/extensions/settings-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("settings-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(result["initial"], false);
    assert_eq!(result["blocked"], true);
    assert_eq!(result["unblocked"], false);
    // Tree-navigation reads (PLAN 6.4) surface the spec defaults.
    assert_eq!(result["doubleEscapeAction"], "tree");
    assert_eq!(result["treeFilterMode"], "default");
    assert_eq!(result["branchSummaryReserveTokens"], 16384);
    assert_eq!(result["branchSummarySkipPrompt"], false);
    // Compaction read (PLAN 6.5) surfaces the spec defaults.
    assert_eq!(result["compactionEnabled"], true);
    assert_eq!(result["compactionReserveTokens"], 16384);
    assert_eq!(result["compactionKeepRecentTokens"], 20000);
    // Bash-mode reads (PLAN 7.1): unset by default.
    assert_eq!(result["shellCommandPrefixUnset"], true);
    assert_eq!(result["shellPathUnset"], true);
    // Thinking default (PLAN 7.2): unset by default, persists on set.
    assert_eq!(result["defaultThinkingLevelUnset"], true);
    assert_eq!(result["defaultThinkingLevelSet"], "high");
    assert_eq!(result["theme"], "light");
    assert_eq!(result["steeringMode"], "all");
    assert_eq!(result["followUpMode"], "all");
    assert_eq!(result["httpIdleTimeoutMs"], 60_000);
    assert_eq!(result["editorPaddingX"], 2);
    assert_eq!(result["autocompleteMaxVisible"], 7);
    assert_eq!(result["anthropicWarning"], false);
    assert_eq!(
        result["enabledModels"],
        serde_json::json!(["anthropic/claude-opus-4-6", "openai/gpt-5.4"])
    );
    assert_eq!(result["lastChangelogVersionUnset"], true);
    assert_eq!(result["lastChangelogVersion"], "0.79.0");

    let persisted: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(agent_dir.path().join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["theme"], "light");
    assert_eq!(persisted["defaultProvider"], "anthropic");
    assert_eq!(persisted["defaultModel"], "claude-opus-4-6");
    assert_eq!(persisted["warnings"]["anthropicExtraUsage"], false);
    assert_eq!(
        persisted["enabledModels"],
        serde_json::json!(["anthropic/claude-opus-4-6", "openai/gpt-5.4"])
    );
    assert_eq!(persisted["lastChangelogVersion"], "0.79.0");
}
