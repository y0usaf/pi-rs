//! Differential for PLAN 9.6 (coding.lua-config): the effective settings
//! read-model. The oracle in tests/config-settings-parity/oracle.json is
//! generated from Pi's real `SettingsManager` (merge + migration + typed
//! getters) by scripts/config-settings-oracle. This test replays each
//! scenario's global + project settings through pi-rs's SettingsManager and
//! asserts the same typed getter outcomes.
//!
//! DESIGN difference 2 authorizes pi-rs's config.lua declaration *format* in
//! place of Pi's settings.json; the differential therefore compares effective
//! outcomes, not file bytes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::settings_manager::{
    SettingsManager, SettingsManagerCreateOptions, SettingsScope, SettingsStorage,
};
use serde_json::{Map, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/config-settings-parity/oracle.json")).unwrap()
}

/// Build a pi-rs SettingsManager with the given global and project maps stored
/// in-memory, through the same config.lua declaration format pi-rs uses.
fn manager_for(global: &Value, project: &Value, project_trusted: bool) -> SettingsManager {
    let global_decl =
        create_config_lua(global);
    let project_decl = create_config_lua(project);
    let mut storage = SettingsStorage::in_memory();
    storage
        .with_lock(SettingsScope::Global, |_| Ok(Some(global_decl)))
        .unwrap();
    if project_trusted {
        storage
            .with_lock(SettingsScope::Project, |_| Ok(Some(project_decl)))
            .unwrap();
    }
    SettingsManager::from_storage(storage, SettingsManagerCreateOptions {
        project_trusted: Some(project_trusted),
    })
}

/// Render a settings map as a canonical `pi.config.settings({...})` Lua call.
fn create_config_lua(settings: &Value) -> String {
    // Reuse the embedded Lua-value renderer exposed by the config module for
    // managed blocks; fall back to a compact JSON→Lua conversion for maps.
    fn render(value: &Value) -> String {
        match value {
            Value::Null => "nil".to_owned(),
            Value::Bool(true) => "true".to_owned(),
            Value::Bool(false) => "false".to_owned(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => {
                // Lua single-quoted string with escaping for quotes/newlines.
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('\'', "\\'")
                    .replace('\n', "\\n");
                format!("'{escaped}'")
            }
            Value::Array(a) => {
                let inner = a.iter().map(render).collect::<Vec<_>>().join(", ");
                format!("{{ {inner} }}")
            }
            Value::Object(map) => {
                let inner = map
                    .iter()
                    .map(|(k, v)| format!("['{k}'] = {}", render(v)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {inner} }}")
            }
        }
    }
    format!("local pi = ...\npi.config.settings({})\n", render(settings))
}

/// Run a named getter on the manager, returning the same JSON shape the oracle
/// recorded (null for undefined/None).
fn run_getters(m: &SettingsManager) -> Map<String, Value> {
    let mut out = Map::new();
    let insert = |out: &mut Map<String, Value>, k: &str, v: Value| {
        out.insert(k.to_owned(), v);
    };

    // Option<string> getters.
    insert(&mut out, "getTheme", opt_str(m.get_theme()));
    insert(&mut out, "getDefaultProvider", opt_str(m.get_default_provider()));
    insert(&mut out, "getDefaultModel", opt_str(m.get_default_model()));
    insert(&mut out, "getShellPath", opt_str(m.get_shell_path()));
    insert(&mut out, "getShellCommandPrefix", opt_str(m.get_shell_command_prefix()));
    insert(&mut out, "getDefaultThinkingLevel", {
        match m.get_default_thinking_level() {
            Some(l) => serde_json::to_value(l).unwrap_or(Value::Null),
            None => Value::Null,
        }
    });

    // Enumerated/string getters.
    insert(&mut out, "getSteeringMode", Value::String(m.get_steering_mode()));
    insert(&mut out, "getFollowUpMode", Value::String(m.get_follow_up_mode()));
    insert(&mut out, "getTransport", Value::String(m.get_transport()));
    insert(&mut out, "getDefaultProjectTrust", Value::String(m.get_default_project_trust().to_owned()));
    insert(&mut out, "getDoubleEscapeAction", Value::String(m.get_double_escape_action()));
    insert(&mut out, "getTreeFilterMode", Value::String(m.get_tree_filter_mode()));
    insert(&mut out, "getCodeBlockIndent", Value::String(m.get_code_block_indent()));

    // bool getters.
    insert(&mut out, "getCompactionEnabled", Value::Bool(m.get_compaction_enabled()));
    insert(&mut out, "getBranchSummarySkipPrompt", Value::Bool(m.get_branch_summary_skip_prompt()));
    insert(&mut out, "getRetryEnabled", Value::Bool(m.get_retry_enabled()));
    insert(&mut out, "getHideThinkingBlock", Value::Bool(m.get_hide_thinking_block()));
    insert(&mut out, "getQuietStartup", Value::Bool(m.get_quiet_startup()));
    insert(&mut out, "getCollapseChangelog", Value::Bool(m.get_collapse_changelog()));
    insert(&mut out, "getEnableInstallTelemetry", Value::Bool(m.get_enable_install_telemetry()));
    insert(&mut out, "getEnableSkillCommands", Value::Bool(m.get_enable_skill_commands()));
    insert(&mut out, "getShowImages", Value::Bool(m.get_show_images()));
    insert(&mut out, "getClearOnShrink", Value::Bool(m.get_clear_on_shrink()));
    insert(&mut out, "getShowTerminalProgress", Value::Bool(m.get_show_terminal_progress()));
    insert(&mut out, "getImageAutoResize", Value::Bool(m.get_image_auto_resize()));
    insert(&mut out, "getBlockImages", Value::Bool(m.get_block_images()));
    insert(&mut out, "getShowHardwareCursor", Value::Bool(m.get_show_hardware_cursor()));

    // numeric getters.
    insert(&mut out, "getCompactionReserveTokens", json_u64(m.get_compaction_reserve_tokens()));
    insert(&mut out, "getCompactionKeepRecentTokens", json_u64(m.get_compaction_keep_recent_tokens()));
    insert(&mut out, "getImageWidthCells", json_u64(m.get_image_width_cells()));
    insert(&mut out, "getEditorPaddingX", json_u64(m.get_editor_padding_x()));
    insert(&mut out, "getAutocompleteMaxVisible", json_u64(m.get_autocomplete_max_visible()));
    insert(&mut out, "getHttpIdleTimeoutMs", json_u64(
        m.get_http_idle_timeout_ms().unwrap_or(u64::MAX),
    ));
    insert(&mut out, "getWebSocketConnectTimeoutMs", m.get_websocket_connect_timeout_ms().ok().flatten().map(json_u64).unwrap_or(Value::Null));

    // object getters.
    insert(&mut out, "getCompactionSettings", Value::Object({
        let c = m.get_compaction_settings();
        Map::from_iter([
            ("enabled".into(), Value::Bool(c.enabled)),
            ("reserveTokens".into(), json_u64(c.reserve_tokens)),
            ("keepRecentTokens".into(), json_u64(c.keep_recent_tokens)),
        ])
    }));
    insert(&mut out, "getBranchSummarySettings", Value::Object({
        let b = m.get_branch_summary_settings();
        Map::from_iter([
            ("reserveTokens".into(), json_u64(b.reserve_tokens)),
            ("skipPrompt".into(), Value::Bool(b.skip_prompt)),
        ])
    }));
    insert(&mut out, "getRetrySettings", Value::Object({
        let r = m.get_retry_settings();
        Map::from_iter([
            ("enabled".into(), Value::Bool(r.enabled)),
            ("maxRetries".into(), json_u64(r.max_retries)),
            ("baseDelayMs".into(), json_u64(r.base_delay_ms)),
        ])
    }));
    insert(&mut out, "getProviderRetrySettings", Value::Object({
        let r = m.get_provider_retry_settings();
        let mut map = Map::new();
        if let Some(t) = r.timeout_ms {
            map.insert("timeoutMs".into(), json_u64(t));
        }
        map.insert("maxRetryDelayMs".into(), json_u64(r.max_retry_delay_ms));
        map
    }));
    insert(&mut out, "getWarnings", Value::Object(m.get_warnings()));

    // list getters.
    insert(&mut out, "getEnabledModels", m.get_enabled_models().map(|v| Value::Array(v.into_iter().map(Value::String).collect())).unwrap_or(Value::Null));
    insert(&mut out, "getPackages", Value::Array(m.get_packages()));
    insert(&mut out, "getExtensionPaths", Value::Array(m.get_extension_paths().into_iter().map(Value::String).collect()));
    insert(&mut out, "getSkillPaths", Value::Array(m.get_skill_paths().into_iter().map(Value::String).collect()));
    insert(&mut out, "getPromptTemplatePaths", Value::Array(m.get_prompt_template_paths().into_iter().map(Value::String).collect()));
    insert(&mut out, "getThemePaths", Value::Array(m.get_theme_paths().into_iter().map(Value::String).collect()));
    insert(&mut out, "getNpmCommand", m.get_npm_command().map(|v| Value::Array(v.into_iter().map(Value::String).collect())).unwrap_or(Value::Null));

    // optional object.
    insert(&mut out, "getThinkingBudgets", m.get_thinking_budgets().unwrap_or(Value::Null));

    out
}

fn opt_str(v: Option<String>) -> Value {
    match v {
        Some(s) => Value::String(s),
        None => Value::Null,
    }
}

fn json_u64(v: u64) -> Value {
    serde_json::json!(v)
}

#[test]
fn effective_settings_reads_match_pi_typed_getters() {
    let oracle = fixture();
    let expected_getters: Vec<String> = oracle["getters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g.as_str().unwrap().to_owned())
        .collect();
    for scenario in oracle["scenarios"].as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let global = &scenario["global"];
        let project = &scenario["project"];
        let project_trusted = scenario["projectTrusted"].as_bool().unwrap_or(true);
        let m = manager_for(global, project, project_trusted);
        let got = run_getters(&m);
        for g in &expected_getters {
            let expected = &scenario["getters"][g];
            let actual = got.get(g).unwrap_or(&Value::Null);
            assert_eq!(
                actual, expected,
                "{name}: getter {g} mismatch\n  expected: {expected}\n  actual:   {actual}"
            );
        }
    }
}

/// Convert a keybinding declaration value (string or array) into a Lua literal.
fn key_literal(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Array(a) => {
            let items = a
                .iter()
                .map(|k| format!("'{}'", k.as_str().unwrap()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {items} }}")
        }
        _ => "''".to_owned(),
    }
}

/// Render a full keybinding config.lua declaration from a raw map.
fn key_decl_source(raw: &Map<String, Value>) -> String {
    let mut decls = String::new();
    for (k, v) in raw {
        let quoted = format!("'{}'", k.replace('\\', "\\\\").replace('\'', "\\'"));
        decls.push_str(&format!(
            "pi.config.keybindings({{ [{}] = {} }})\n",
            quoted, key_literal(v)
        ));
    }
    format!("local pi = ...\n{decls}")
}

/// Drive Pi's `migrateKeybindingsConfig` oracle through pi-rs's canonical
/// `config.lua` declaration surface: declaring legacy keybinding names must
/// migrate to their modern names exactly as Pi does — including collision
/// handling (a legacy key is dropped when the same modern key is also present),
/// and ordering by `orderKeybindingsConfig` (known KEYBINDINGS first in
/// definition order, then unknown keys sorted).
#[test]
fn keybinding_declarations_migrate_like_pi() {
    let oracle = fixture();
    for case in oracle["keybindings"]["migrate"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let raw: Map<String, Value> = case["raw"].as_object().unwrap().clone();
        let snapshot =
            pi_rs_host::config::evaluate(&key_decl_source(&raw), "config.lua").unwrap();
        let expected: Map<String, Value> = case["config"]
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect();
        assert_eq!(
            snapshot.keybindings, expected,
            "{name}: migrated keybinding map mismatch\n  expected: {expected:?}\n  actual:   {:?}",
            snapshot.keybindings
        );
    }
}
