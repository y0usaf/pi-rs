//! Upstream's `settings-manager.test.ts`, `settings-manager-compaction.test.ts`,
//! `settings-manager-bug.test.ts` and `settings-diagnostics.test.ts`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pi_coding_agent::settings::{
    DiagnosticKind, InMemorySettingsStorage, QuietStartup, Settings, SettingsDiagnostic,
    SettingsManager, SettingsScope, SettingsStorage, Update, WheelScrollLines, collect_diagnostics,
    deduplicate_diagnostics,
};
use pi_durable::ai::types::ModelThinkingLevel;
use serde_json::{Map, Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A throwaway cwd and agent dir, removed when the test ends.
struct Tree {
    root: String,
}

impl Tree {
    fn new(name: &str) -> Tree {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = format!("/tmp/pi-settings-{name}-{}-{unique}", std::process::id());
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(format!("{root}/agent")).expect("agent dir");
        std::fs::create_dir_all(format!("{root}/project/.pi")).expect("project dir");
        Tree { root }
    }

    fn agent(&self) -> String {
        format!("{}/agent", self.root)
    }

    fn project(&self) -> String {
        format!("{}/project", self.root)
    }

    fn agent_settings(&self) -> String {
        format!("{}/agent/settings.json", self.root)
    }

    fn project_settings(&self) -> String {
        format!("{}/project/.pi/settings.json", self.root)
    }

    fn write(&self, path: &str, text: &str) {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, text).expect("write settings");
    }

    fn write_json(&self, path: &str, value: Value) {
        self.write(path, &serde_json::to_string_pretty(&value).expect("json"));
    }

    fn read_json(&self, path: &str) -> Value {
        let text = std::fs::read_to_string(path).expect("read settings");
        serde_json::from_str(&text).expect("settings json")
    }

    fn manager(&self) -> SettingsManager {
        SettingsManager::create(&self.project(), &self.agent(), true)
    }

    fn untrusted(&self) -> SettingsManager {
        SettingsManager::create(&self.project(), &self.agent(), false)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected an object, got {other}"),
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn values(items: &[&str]) -> Vec<Value> {
    items.iter().map(|item| Value::from(*item)).collect()
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    let lengths: Vec<usize> = groups.iter().map(|group| group.len()).collect();
    lengths == [8, 4, 4, 4, 12]
        && groups.iter().all(|group| {
            group
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
}

mod manager {
    use super::*;

    mod preserves_externally_added_settings {
        use super::*;

        #[test]
        fn keeps_enabled_models_when_changing_the_thinking_level() {
            let tree = Tree::new("preserve-enabled-models");
            tree.write_json(
                &tree.agent_settings(),
                json!({"theme": "dark", "defaultModel": "claude-sonnet"}),
            );
            let mut settings = tree.manager();

            let mut file = tree.read_json(&tree.agent_settings());
            file["enabledModels"] = json!(["claude-opus-4-5", "gpt-5.2-codex"]);
            tree.write_json(&tree.agent_settings(), file);

            settings.set_default_thinking_level(ModelThinkingLevel::High);

            assert_eq!(
                tree.read_json(&tree.agent_settings()),
                json!({
                    "enabledModels": ["claude-opus-4-5", "gpt-5.2-codex"],
                    "defaultThinkingLevel": "high",
                    "theme": "dark",
                    "defaultModel": "claude-sonnet",
                })
            );
        }

        #[test]
        fn keeps_custom_settings_when_changing_the_theme() {
            let tree = Tree::new("preserve-custom");
            tree.write_json(
                &tree.agent_settings(),
                json!({"defaultModel": "claude-sonnet"}),
            );
            let mut settings = tree.manager();

            let mut file = tree.read_json(&tree.agent_settings());
            file["shellPath"] = json!("/bin/zsh");
            file["extensions"] = json!(["/path/to/extension.ts"]);
            tree.write_json(&tree.agent_settings(), file);

            settings.set_theme("light");

            assert_eq!(
                tree.read_json(&tree.agent_settings()),
                json!({
                    "shellPath": "/bin/zsh",
                    "extensions": ["/path/to/extension.ts"],
                    "theme": "light",
                    "defaultModel": "claude-sonnet",
                })
            );
        }

        #[test]
        fn lets_an_in_memory_change_override_the_file_for_the_same_key() {
            let tree = Tree::new("in-memory-wins");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));
            let mut settings = tree.manager();

            let mut file = tree.read_json(&tree.agent_settings());
            file["defaultThinkingLevel"] = json!("low");
            tree.write_json(&tree.agent_settings(), file);

            settings.set_default_thinking_level(ModelThinkingLevel::High);

            assert_eq!(
                tree.read_json(&tree.agent_settings())["defaultThinkingLevel"],
                json!("high")
            );
        }
    }

    #[test]
    fn creates_one_global_device_id_and_reuses_it() {
        let tree = Tree::new("device-id");
        tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));
        tree.write_json(
            &tree.project_settings(),
            json!({"deviceId": "project-device"}),
        );
        let mut settings = tree.manager();

        let device_id = settings.get_or_create_device_id();
        assert!(is_uuid(&device_id), "{device_id} is not a uuid");
        assert_eq!(settings.get_or_create_device_id(), device_id);
        assert_eq!(tree.manager().get_or_create_device_id(), device_id);
        assert_eq!(
            tree.read_json(&tree.agent_settings()),
            json!({"theme": "dark", "deviceId": device_id})
        );
    }

    mod packages_migration {
        use super::*;

        #[test]
        fn keeps_local_only_extensions_in_the_extensions_array() {
            let tree = Tree::new("extensions-only");
            tree.write_json(
                &tree.agent_settings(),
                json!({"extensions": ["/local/ext.ts", "./relative/ext.ts"]}),
            );
            let settings = tree.manager();

            assert_eq!(settings.packages(), Vec::<Value>::new());
            assert_eq!(
                settings.extension_paths(),
                strings(&["/local/ext.ts", "./relative/ext.ts"])
            );
        }

        #[test]
        fn keeps_packages_with_filtering_objects() {
            let tree = Tree::new("packages-objects");
            let packages = json!([
                "npm:simple-pkg",
                {
                    "source": "npm:shitty-extensions",
                    "extensions": ["extensions/oracle.ts"],
                    "skills": [],
                },
            ]);
            tree.write_json(&tree.agent_settings(), json!({ "packages": packages }));
            let settings = tree.manager();

            assert_eq!(settings.packages().len(), 2);
            assert_eq!(settings.packages()[0], json!("npm:simple-pkg"));
            assert_eq!(
                settings.packages()[1],
                json!({
                    "source": "npm:shitty-extensions",
                    "extensions": ["extensions/oracle.ts"],
                    "skills": [],
                })
            );
        }
    }

    mod reload {
        use super::*;

        #[test]
        fn reloads_global_settings_from_disk() {
            let tree = Tree::new("reload");
            tree.write_json(
                &tree.agent_settings(),
                json!({"theme": "dark", "extensions": ["/before.ts"]}),
            );
            let mut settings = tree.manager();

            tree.write_json(
                &tree.agent_settings(),
                json!({
                    "theme": "light",
                    "extensions": ["/after.ts"],
                    "defaultModel": "claude-sonnet",
                }),
            );
            settings.reload();

            assert_eq!(settings.theme(), Some("light"));
            assert_eq!(settings.extension_paths(), strings(&["/after.ts"]));
            assert_eq!(settings.default_model(), Some("claude-sonnet"));
        }

        #[test]
        fn keeps_previous_settings_and_reports_the_path_when_the_file_is_invalid() {
            let tree = Tree::new("reload-invalid");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));
            let mut settings = tree.manager();

            tree.write(&tree.agent_settings(), "{ invalid json");
            settings.reload();

            assert_eq!(settings.theme(), Some("dark"));
            let errors = settings.drain_errors();
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].scope, SettingsScope::Global);
            assert_eq!(
                errors[0].path.as_deref(),
                Some(tree.agent_settings().as_str())
            );
        }
    }

    #[test]
    fn stores_slash_separated_themes_apart_from_fixed_theme_names() {
        let tree = Tree::new("theme-setting");
        tree.write_json(&tree.agent_settings(), json!({"theme": "light/dark"}));
        let mut settings = tree.manager();

        assert_eq!(settings.theme(), None);
        assert_eq!(settings.theme_setting(), Some("light/dark"));

        settings.set_theme("solarized-light/tokyo-night");
        assert_eq!(
            tree.read_json(&tree.agent_settings())["theme"],
            json!("solarized-light/tokyo-night")
        );
    }

    #[test]
    fn collects_and_clears_load_errors() {
        let tree = Tree::new("errors");
        tree.write(&tree.agent_settings(), "{ invalid global json");
        tree.write(&tree.project_settings(), "{ invalid project json");

        let mut settings = tree.manager();
        let errors = settings.drain_errors();

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].scope, SettingsScope::Global);
        assert_eq!(
            errors[0].path.as_deref(),
            Some(tree.agent_settings().as_str())
        );
        assert_eq!(errors[1].scope, SettingsScope::Project);
        assert_eq!(
            errors[1].path.as_deref(),
            Some(tree.project_settings().as_str())
        );
        assert!(settings.drain_errors().is_empty());
    }

    mod project_trust {
        use super::*;

        #[test]
        fn skips_project_settings_when_the_project_is_not_trusted() {
            let tree = Tree::new("trust-skip");
            tree.write_json(&tree.agent_settings(), json!({"theme": "global"}));
            tree.write_json(&tree.project_settings(), json!({"theme": "project"}));

            let settings = tree.untrusted();

            assert!(!settings.is_project_trusted());
            assert_eq!(settings.theme(), Some("global"));
            assert_eq!(settings.project_settings(), &Settings::default());
        }

        #[test]
        fn loads_project_settings_once_trusted() {
            let tree = Tree::new("trust-reload");
            tree.write_json(&tree.agent_settings(), json!({"theme": "global"}));
            tree.write_json(&tree.project_settings(), json!({"theme": "project"}));
            let mut settings = tree.untrusted();

            settings.set_project_trusted(true);

            assert!(settings.is_project_trusted());
            assert_eq!(settings.theme(), Some("project"));
        }

        #[test]
        fn refuses_project_writes_when_the_project_is_not_trusted() {
            let tree = Tree::new("trust-write");
            tree.write_json(
                &tree.project_settings(),
                json!({"packages": ["npm:existing"]}),
            );
            let mut settings = tree.untrusted();

            assert_eq!(
                settings.set_project_packages(&values(&["npm:new"])),
                Err("Project is not trusted; refusing to write project settings".to_owned())
            );

            assert_eq!(settings.project_settings(), &Settings::default());
            assert_eq!(
                tree.read_json(&tree.project_settings()),
                json!({"packages": ["npm:existing"]})
            );
        }

        #[test]
        fn reads_the_default_project_trust_from_global_settings_only() {
            let tree = Tree::new("trust-default");
            tree.write_json(
                &tree.agent_settings(),
                json!({"defaultProjectTrust": "always"}),
            );
            tree.write_json(
                &tree.project_settings(),
                json!({"defaultProjectTrust": "never"}),
            );

            assert_eq!(tree.manager().default_project_trust(), "always");
        }

        #[test]
        fn defaults_an_invalid_project_trust_to_ask() {
            let tree = Tree::new("trust-invalid");
            tree.write_json(
                &tree.agent_settings(),
                json!({"defaultProjectTrust": "sometimes"}),
            );

            assert_eq!(tree.manager().default_project_trust(), "ask");
        }
    }

    mod project_settings_directory {
        use super::*;

        #[test]
        fn is_not_created_by_reading() {
            let tree = Tree::new("no-pi-dir-read");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));
            std::fs::remove_dir_all(format!("{}/project/.pi", tree.root)).expect("remove .pi");

            let settings = tree.manager();

            assert!(!std::path::Path::new(&format!("{}/project/.pi", tree.root)).exists());
            assert_eq!(settings.theme(), Some("dark"));
        }

        #[test]
        fn is_created_by_writing_project_settings() {
            let tree = Tree::new("pi-dir-write");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));
            std::fs::remove_dir_all(format!("{}/project/.pi", tree.root)).expect("remove .pi");
            let mut settings = tree.manager();

            assert!(!std::path::Path::new(&format!("{}/project/.pi", tree.root)).exists());

            settings
                .set_project_packages(&[json!({"source": "npm:test-pkg"})])
                .expect("write project settings");

            assert!(std::path::Path::new(&tree.project_settings()).exists());
            assert_eq!(
                tree.read_json(&tree.project_settings()),
                json!({"packages": [{"source": "npm:test-pkg"}]})
            );
        }
    }

    #[test]
    fn defaults_and_overrides_the_agent_retry_delay_cap() {
        assert_eq!(
            SettingsManager::in_memory(Map::new(), true).retry_settings(),
            pi_coding_agent::settings::RetrySettings {
                enabled: true,
                max_retries: 3.0,
                base_delay_ms: 2000.0,
                max_agent_delay_ms: 60000.0,
            }
        );
        let settings = SettingsManager::in_memory(
            object(json!({
                "retry": {"enabled": true, "maxRetries": 10, "baseDelayMs": 500, "maxAgentDelayMs": 5000},
            })),
            true,
        );
        assert_eq!(
            settings.retry_settings(),
            pi_coding_agent::settings::RetrySettings {
                enabled: true,
                max_retries: 10.0,
                base_delay_ms: 500.0,
                max_agent_delay_ms: 5000.0,
            }
        );
    }

    mod http_idle_timeout {
        use super::*;

        #[test]
        fn defaults_to_five_minutes() {
            let tree = Tree::new("http-default");
            assert_eq!(tree.manager().http_idle_timeout_ms(), Ok(300_000.0));
        }

        #[test]
        fn uses_the_merged_global_and_project_settings() {
            let tree = Tree::new("http-merge");
            tree.write_json(&tree.agent_settings(), json!({"httpIdleTimeoutMs": 300000}));
            tree.write_json(&tree.project_settings(), json!({"httpIdleTimeoutMs": 0}));

            assert_eq!(tree.manager().http_idle_timeout_ms(), Ok(0.0));
        }

        #[test]
        fn rejects_invalid_values() {
            let tree = Tree::new("http-invalid");
            tree.write_json(&tree.agent_settings(), json!({"httpIdleTimeoutMs": -1}));

            assert_eq!(
                tree.manager().http_idle_timeout_ms(),
                Err("Invalid httpIdleTimeoutMs setting: -1".to_owned())
            );
        }
    }

    mod cache_warming {
        use super::*;

        #[test]
        fn defaults_to_streaming_and_ignores_project_settings() {
            let tree = Tree::new("cache-warming");

            assert_eq!(tree.manager().cache_warming_mode(), "streaming");

            tree.write_json(&tree.project_settings(), json!({"cacheWarming": "idle"}));
            assert_eq!(tree.manager().cache_warming_mode(), "streaming");

            tree.write_json(&tree.agent_settings(), json!({"cacheWarming": "idle"}));
            assert_eq!(tree.manager().cache_warming_mode(), "idle");

            tree.write_json(&tree.agent_settings(), json!({"cacheWarming": "bogus"}));
            assert_eq!(tree.manager().cache_warming_mode(), "streaming");
        }

        #[test]
        fn persists_the_mode_globally() {
            let tree = Tree::new("cache-warming-save");
            let mut settings = tree.manager();

            settings.set_cache_warming_mode("off");

            assert_eq!(tree.manager().cache_warming_mode(), "off");
            assert_eq!(
                tree.read_json(&tree.agent_settings()),
                json!({"cacheWarming": "off"})
            );
        }
    }

    mod external_editor {
        use super::*;

        /// Set or clear an editor variable. This module is the only reader of VISUAL and EDITOR
        /// in this test binary, so the change is not observed elsewhere.
        fn set(name: &str, value: Option<&str>) {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }

        #[test]
        fn resolves_editor_commands_by_precedence_then_the_platform_default() {
            let visual = std::env::var("VISUAL").ok();
            let editor = std::env::var("EDITOR").ok();

            set("VISUAL", Some("vim"));
            set("EDITOR", Some("nano"));
            assert_eq!(
                SettingsManager::in_memory(object(json!({"externalEditor": "code --wait"})), true)
                    .external_editor_command(),
                "code --wait"
            );
            assert_eq!(
                SettingsManager::in_memory(Map::new(), true).external_editor_command(),
                "vim"
            );

            set("VISUAL", None);
            set("EDITOR", Some("emacs"));
            assert_eq!(
                SettingsManager::in_memory(Map::new(), true).external_editor_command(),
                "emacs"
            );

            set("EDITOR", None);
            let expected = if cfg!(windows) { "notepad" } else { "nano" };
            assert_eq!(
                SettingsManager::in_memory(Map::new(), true).external_editor_command(),
                expected
            );

            set("VISUAL", visual.as_deref());
            set("EDITOR", editor.as_deref());
        }
    }

    mod tui_mode {
        use super::*;

        #[test]
        fn defaults_to_fullscreen_and_persists_regular() {
            let tree = Tree::new("tui-mode");
            let mut settings = tree.manager();

            assert_eq!(settings.tui_mode(), "fullscreen");

            settings.set_tui_mode("regular");

            assert_eq!(settings.tui_mode(), "regular");
            assert_eq!(
                tree.read_json(&tree.agent_settings())["tuiMode"],
                json!("regular")
            );
        }

        #[test]
        fn falls_back_to_fullscreen_for_unsupported_values() {
            let tree = Tree::new("tui-mode-invalid");
            tree.write_json(&tree.agent_settings(), json!({"tuiMode": "other"}));

            assert_eq!(tree.manager().tui_mode(), "fullscreen");
        }

        #[test]
        fn does_not_recognize_the_old_ui_mode_setting() {
            let tree = Tree::new("ui-mode");
            tree.write_json(&tree.agent_settings(), json!({"uiMode": "regular"}));

            assert_eq!(tree.manager().tui_mode(), "fullscreen");
        }
    }

    #[test]
    fn validates_and_persists_fullscreen_settings() {
        let tree = Tree::new("fullscreen");
        let mut settings = tree.manager();
        assert_eq!(settings.fullscreen_exit_output(), "transcript");
        assert_eq!(settings.fullscreen_scrollbar(), "auto");
        assert!(settings.fullscreen_copy_on_select());

        settings.set_fullscreen_exit_output("resume-hint");
        settings.set_fullscreen_scrollbar("hidden");
        settings.set_fullscreen_copy_on_select(false);

        assert_eq!(
            tree.read_json(&tree.agent_settings()),
            json!({
                "fullscreenExitOutput": "resume-hint",
                "fullscreenScrollbar": "hidden",
                "fullscreenCopyOnSelect": false,
            })
        );

        tree.write_json(
            &tree.agent_settings(),
            json!({"fullscreenExitOutput": "nothing", "fullscreenScrollbar": "sometimes"}),
        );
        let reloaded = tree.manager();
        assert_eq!(reloaded.fullscreen_exit_output(), "transcript");
        assert_eq!(reloaded.fullscreen_scrollbar(), "auto");
        assert!(reloaded.fullscreen_copy_on_select());
    }

    #[test]
    fn persists_fullscreen_wheel_scroll_lines() {
        let tree = Tree::new("wheel-lines");
        let mut settings = tree.manager();
        assert_eq!(
            settings.fullscreen_wheel_scroll_lines(),
            WheelScrollLines::Auto
        );

        settings.set_fullscreen_wheel_scroll_lines(WheelScrollLines::Lines(3));
        assert_eq!(
            tree.read_json(&tree.agent_settings())["fullscreenWheelScrollLines"],
            json!(3)
        );

        for (value, expected) in [
            (json!(7.9), WheelScrollLines::Lines(7)),
            (json!(0), WheelScrollLines::Lines(1)),
            (json!(1000), WheelScrollLines::Lines(100)),
            (json!("fast"), WheelScrollLines::Auto),
            (Value::Null, WheelScrollLines::Auto),
        ] {
            tree.write_json(
                &tree.agent_settings(),
                json!({"fullscreenWheelScrollLines": value}),
            );
            assert_eq!(
                tree.manager().fullscreen_wheel_scroll_lines(),
                expected,
                "for {value}"
            );
        }
    }

    mod output_pad {
        use super::*;

        #[test]
        fn defaults_to_one_and_persists_binary_values() {
            let tree = Tree::new("output-pad");
            let mut settings = tree.manager();

            assert_eq!(settings.output_pad(), 1);

            settings.set_output_pad(0);

            assert_eq!(settings.output_pad(), 0);
            assert_eq!(
                tree.read_json(&tree.agent_settings())["outputPad"],
                json!(0)
            );
        }

        #[test]
        fn treats_unsupported_values_as_the_default() {
            let tree = Tree::new("output-pad-invalid");
            tree.write_json(&tree.agent_settings(), json!({"outputPad": 2}));

            assert_eq!(tree.manager().output_pad(), 1);
        }
    }

    mod mermaid {
        use super::*;

        #[test]
        fn defaults_to_streaming_and_persists_modes() {
            let tree = Tree::new("mermaid");
            let mut settings = tree.manager();

            assert_eq!(settings.mermaid_rendering_mode(), "streaming");

            settings.set_mermaid_rendering_mode("final");

            assert_eq!(settings.mermaid_rendering_mode(), "final");
            assert_eq!(
                tree.read_json(&tree.agent_settings()),
                json!({"markdown": {"mermaid": "final"}})
            );
        }

        #[test]
        fn falls_back_to_streaming_for_unsupported_values() {
            let tree = Tree::new("mermaid-invalid");
            tree.write_json(
                &tree.agent_settings(),
                json!({"markdown": {"mermaid": "sometimes"}}),
            );

            assert_eq!(tree.manager().mermaid_rendering_mode(), "streaming");
        }
    }

    mod shell_command_prefix {
        use super::*;

        #[test]
        fn loads_the_prefix() {
            let tree = Tree::new("prefix");
            tree.write_json(
                &tree.agent_settings(),
                json!({"shellCommandPrefix": "shopt -s expand_aliases"}),
            );

            assert_eq!(
                tree.manager().shell_command_prefix(),
                Some("shopt -s expand_aliases")
            );
        }

        #[test]
        fn is_undefined_when_not_set() {
            let tree = Tree::new("prefix-unset");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));

            assert_eq!(tree.manager().shell_command_prefix(), None);
        }

        #[test]
        fn survives_saving_unrelated_settings() {
            let tree = Tree::new("prefix-preserve");
            tree.write_json(
                &tree.agent_settings(),
                json!({"shellCommandPrefix": "shopt -s expand_aliases"}),
            );
            let mut settings = tree.manager();

            settings.set_theme("light");

            assert_eq!(
                tree.read_json(&tree.agent_settings()),
                json!({"shellCommandPrefix": "shopt -s expand_aliases", "theme": "light"})
            );
        }
    }

    mod default_tools {
        use super::*;

        #[test]
        fn loads_global_defaults_and_lets_the_project_replace_them() {
            let tree = Tree::new("tools-replace");
            tree.write_json(
                &tree.agent_settings(),
                json!({"defaultTools": ["read", "bash"]}),
            );
            assert_eq!(
                tree.manager().default_tools(),
                Some(strings(&["read", "bash"]))
            );

            tree.write_json(&tree.project_settings(), json!({"defaultTools": ["grep"]}));
            assert_eq!(tree.manager().default_tools(), Some(strings(&["grep"])));
        }

        #[test]
        fn preserves_an_empty_tool_list() {
            assert_eq!(
                SettingsManager::in_memory(object(json!({"defaultTools": []})), true)
                    .default_tools(),
                Some(Vec::new())
            );
            assert_eq!(
                SettingsManager::in_memory(Map::new(), true).default_tools(),
                None
            );
        }

        #[test]
        fn applies_modifiers_to_the_default_selection() {
            assert_eq!(
                SettingsManager::in_memory(
                    object(json!({"defaultTools": ["+codemode", "-write"]})),
                    true
                )
                .default_tools(),
                Some(strings(&["read", "bash", "edit", "codemode"]))
            );
            assert_eq!(
                SettingsManager::in_memory(
                    object(json!({"defaultTools": ["read", "+grep", "+read"]})),
                    true
                )
                .default_tools(),
                Some(strings(&["read", "grep"]))
            );
        }

        #[test]
        fn layers_project_modifiers_on_the_global_selection() {
            let tree = Tree::new("tools-modifiers");
            tree.write_json(
                &tree.agent_settings(),
                json!({"defaultTools": ["read", "bash", "+codemode"]}),
            );
            tree.write_json(
                &tree.project_settings(),
                json!({"defaultTools": ["-codemode", "+tool_search"]}),
            );
            let mut settings = tree.manager();
            assert_eq!(
                settings.default_tools(),
                Some(strings(&["read", "bash", "tool_search"]))
            );

            settings.apply_overrides(object(json!({"defaultTools": ["+codemode"]})));
            assert_eq!(
                settings.default_tools(),
                Some(strings(&["read", "bash", "tool_search", "codemode"]))
            );
        }

        #[test]
        fn applies_project_modifiers_to_the_built_in_defaults() {
            let tree = Tree::new("tools-project-only");
            tree.write_json(
                &tree.project_settings(),
                json!({"defaultTools": ["+codemode"]}),
            );

            assert_eq!(
                tree.manager().default_tools(),
                Some(strings(&["read", "bash", "edit", "write", "codemode"]))
            );
        }
    }

    mod session_dir {
        use super::*;
        use pi_coding_agent::paths::home_dir;

        #[test]
        fn is_undefined_when_not_set() {
            let tree = Tree::new("session-dir-unset");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));

            assert_eq!(tree.manager().session_dir(), None);
        }

        #[test]
        fn returns_the_global_directory() {
            let tree = Tree::new("session-dir-global");
            tree.write_json(
                &tree.agent_settings(),
                json!({"sessionDir": "/tmp/sessions"}),
            );

            assert_eq!(
                tree.manager().session_dir(),
                Some("/tmp/sessions".to_owned())
            );
        }

        #[test]
        fn returns_the_project_directory_over_the_global_one() {
            let tree = Tree::new("session-dir-project");
            tree.write_json(
                &tree.agent_settings(),
                json!({"sessionDir": "/global/sessions"}),
            );
            tree.write_json(
                &tree.project_settings(),
                json!({"sessionDir": "./sessions"}),
            );

            assert_eq!(tree.manager().session_dir(), Some("./sessions".to_owned()));
        }

        #[test]
        fn expands_a_leading_tilde() {
            let tree = Tree::new("session-dir-tilde");
            tree.write_json(&tree.agent_settings(), json!({"sessionDir": "~/sessions"}));

            assert_eq!(
                tree.manager().session_dir(),
                Some(format!("{}/sessions", home_dir()))
            );
        }
    }

    mod shell_path {
        use super::*;
        use pi_coding_agent::paths::home_dir;

        #[test]
        fn is_undefined_when_not_set() {
            let tree = Tree::new("shell-path-unset");
            tree.write_json(&tree.agent_settings(), json!({"theme": "dark"}));

            assert_eq!(tree.manager().shell_path(), None);
        }

        #[test]
        fn keeps_an_absolute_path_unchanged() {
            let tree = Tree::new("shell-path-absolute");
            tree.write_json(&tree.agent_settings(), json!({"shellPath": "/bin/zsh"}));

            assert_eq!(tree.manager().shell_path(), Some("/bin/zsh".to_owned()));
        }

        #[test]
        fn expands_a_leading_tilde() {
            let tree = Tree::new("shell-path-tilde");
            tree.write_json(
                &tree.agent_settings(),
                json!({"shellPath": "~/.local/bin/agent-shell-sandbox"}),
            );

            assert_eq!(
                tree.manager().shell_path(),
                Some(format!("{}/.local/bin/agent-shell-sandbox", home_dir()))
            );
        }

        #[test]
        fn expands_a_bare_tilde() {
            let tree = Tree::new("shell-path-bare-tilde");
            tree.write_json(&tree.agent_settings(), json!({"shellPath": "~"}));

            assert_eq!(tree.manager().shell_path(), Some(home_dir()));
        }
    }

    #[test]
    fn quiet_startup_reads_true_header_and_off() {
        assert_eq!(
            SettingsManager::in_memory(Map::new(), true).quiet_startup(),
            QuietStartup::Off
        );
        assert_eq!(
            SettingsManager::in_memory(object(json!({"quietStartup": "header"})), true)
                .quiet_startup(),
            QuietStartup::Header
        );
    }
}

mod compaction_model_overrides {
    use super::*;
    use pi_coding_agent::settings::CompactionSettings;

    const MODEL: (&str, &str) = ("provider", "family/model");
    const OTHER: (&str, &str) = ("other", "family/model");
    const MODEL_KEY: &str = "provider/family/model";

    fn defaults() -> CompactionSettings {
        CompactionSettings {
            enabled: true,
            reserve_tokens: 16384,
            keep_recent_tokens: 20000,
        }
    }

    fn entry(field: &str, value: Value) -> Value {
        let mut map = Map::new();
        map.insert(field.to_owned(), value);
        Value::Object(map)
    }

    fn overrides(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert(key.to_owned(), value);
        }
        Value::Object(map)
    }

    fn compaction(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert(key.to_owned(), value);
        }
        json!({ "compaction": Value::Object(map) })
    }

    fn seeded(file: Value) -> SettingsManager {
        let storage = Arc::new(InMemorySettingsStorage::new());
        storage.set(
            SettingsScope::Global,
            &serde_json::to_string(&file).expect("json"),
        );
        SettingsManager::from_storage(storage, true)
    }

    fn invalid_tokens() -> Vec<(Value, &'static str)> {
        vec![
            (Value::Null, "null"),
            (Value::from(-1), "-1"),
            (Value::from(1.5), "1.5"),
            (Value::from("400000"), "400000"),
            (Value::Bool(true), "true"),
            (json!({}), "[object Object]"),
            (json!([]), ""),
            (Value::from(9007199254740992u64), "9007199254740992"),
        ]
    }

    #[test]
    fn uses_defaults_without_compaction_settings() {
        let settings = SettingsManager::in_memory(Map::new(), true);
        assert_eq!(settings.compaction_settings(None), Ok(defaults()));
        assert_eq!(settings.compaction_settings(Some(MODEL)), Ok(defaults()));
    }

    #[test]
    fn resolves_each_field_independently() {
        let settings = SettingsManager::in_memory(
            object(json!({
                "compaction": {
                    "reserveTokens": 8192,
                    "keepRecentTokens": 10000,
                    "modelOverrides": {
                        "provider/family/model": {"reserveTokens": 400000},
                    },
                },
            })),
            true,
        );
        assert_eq!(
            settings.compaction_settings(Some(MODEL)),
            Ok(CompactionSettings {
                enabled: true,
                reserve_tokens: 400000,
                keep_recent_tokens: 10000,
            })
        );
        assert_eq!(settings.compaction_reserve_tokens(Some(MODEL)), Ok(400000));
        assert_eq!(
            settings.compaction_keep_recent_tokens(Some(MODEL)),
            Ok(10000)
        );
        assert_eq!(
            settings.compaction_settings(None),
            Ok(CompactionSettings {
                enabled: true,
                reserve_tokens: 8192,
                keep_recent_tokens: 10000,
            })
        );

        let mut settings = settings;
        settings.apply_overrides(object(json!({
            "compaction": {"modelOverrides": {"provider/family/model": {"keepRecentTokens": 30000}}},
        })));
        assert_eq!(
            settings.compaction_keep_recent_tokens(Some(MODEL)),
            Ok(30000)
        );
        assert_eq!(settings.compaction_reserve_tokens(Some(MODEL)), Ok(400000));
    }

    #[test]
    fn falls_back_to_the_built_in_defaults_for_missing_fields() {
        let settings = SettingsManager::in_memory(
            object(json!({
                "compaction": {"modelOverrides": {"provider/family/model": {"keepRecentTokens": 1024}}},
            })),
            true,
        );
        assert_eq!(
            settings.compaction_settings(Some(MODEL)),
            Ok(CompactionSettings {
                keep_recent_tokens: 1024,
                ..defaults()
            })
        );
    }

    #[test]
    fn matches_exact_provider_and_model_ids_with_slashes() {
        let settings = SettingsManager::in_memory(
            object(json!({
                "compaction": {
                    "modelOverrides": {
                        "provider/family/model": {"reserveTokens": 400000},
                        "provider/*": {"reserveTokens": 1},
                        "family/model": {"reserveTokens": 2},
                    },
                },
            })),
            true,
        );
        assert_eq!(settings.compaction_reserve_tokens(Some(MODEL)), Ok(400000));
        for other in [
            ("other", "family/model"),
            ("provider", "other"),
            ("provider", "family/Model"),
        ] {
            assert_eq!(settings.compaction_settings(Some(other)), Ok(defaults()));
        }
    }

    #[test]
    fn merges_project_model_overrides_per_field() {
        let storage = Arc::new(InMemorySettingsStorage::new());
        storage.set(
            SettingsScope::Global,
            &serde_json::to_string(&compaction(vec![
                ("reserveTokens", json!(8192)),
                (
                    "modelOverrides",
                    overrides(vec![
                        (
                            MODEL_KEY,
                            json!({"reserveTokens": 400000, "keepRecentTokens": 30000}),
                        ),
                        ("provider/other", json!({"keepRecentTokens": 4096})),
                    ]),
                ),
            ]))
            .expect("json"),
        );
        storage.set(
            SettingsScope::Project,
            &serde_json::to_string(&compaction(vec![
                ("reserveTokens", json!(1024)),
                (
                    "modelOverrides",
                    overrides(vec![(MODEL_KEY, json!({"keepRecentTokens": 2000}))]),
                ),
            ]))
            .expect("json"),
        );
        let mut settings = SettingsManager::from_storage(storage, true);

        assert_eq!(
            settings.compaction_settings(Some(MODEL)),
            Ok(CompactionSettings {
                enabled: true,
                reserve_tokens: 400000,
                keep_recent_tokens: 2000,
            })
        );
        assert_eq!(
            settings.compaction_settings(Some(("provider", "other"))),
            Ok(CompactionSettings {
                enabled: true,
                reserve_tokens: 1024,
                keep_recent_tokens: 4096,
            })
        );

        settings.reload();
        assert_eq!(
            settings.compaction_keep_recent_tokens(Some(MODEL)),
            Ok(2000)
        );

        settings.set_project_trusted(false);
        assert_eq!(
            settings.compaction_keep_recent_tokens(Some(MODEL)),
            Ok(30000)
        );
    }

    #[test]
    fn keeps_enabled_global_and_preserves_overrides_when_saving_the_toggle() {
        let mut settings = seeded(compaction(vec![(
            "modelOverrides",
            overrides(vec![(
                MODEL_KEY,
                json!({"enabled": false, "reserveTokens": 400000}),
            )]),
        )]));
        assert_eq!(
            settings.compaction_settings(Some(MODEL)).map(|s| s.enabled),
            Ok(true)
        );

        settings.set_compaction_enabled(false);
        settings.reload();

        assert_eq!(
            settings.compaction_settings(Some(MODEL)),
            Ok(CompactionSettings {
                enabled: false,
                reserve_tokens: 400000,
                ..defaults()
            })
        );
    }

    #[test]
    fn reports_invalid_model_override_tokens() {
        for field in ["reserveTokens", "keepRecentTokens"] {
            for (value, text) in invalid_tokens() {
                let settings = seeded(compaction(vec![(
                    "modelOverrides",
                    overrides(vec![(MODEL_KEY, entry(field, value.clone()))]),
                )]));
                assert_eq!(
                    settings.compaction_settings(Some(MODEL)),
                    Err(format!(
                        "Invalid compaction.modelOverrides[\"{MODEL_KEY}\"].{field} setting: {text}. Expected a non-negative safe integer."
                    )),
                    "for {field} = {value}"
                );
                assert_eq!(settings.compaction_settings(None), Ok(defaults()));
                assert_eq!(settings.compaction_settings(Some(OTHER)), Ok(defaults()));
            }
        }
    }

    #[test]
    fn reports_invalid_ordinary_compaction_tokens_even_with_a_model_override() {
        for field in ["reserveTokens", "keepRecentTokens"] {
            for (value, text) in invalid_tokens() {
                let settings = seeded(compaction(vec![
                    (field, value.clone()),
                    (
                        "modelOverrides",
                        overrides(vec![(MODEL_KEY, entry(field, json!(4096)))]),
                    ),
                ]));
                let error = format!(
                    "Invalid compaction.{field} setting: {text}. Expected a non-negative safe integer."
                );
                assert_eq!(
                    settings.compaction_settings(None),
                    Err(error.clone()),
                    "for {field} = {value}"
                );
                assert_eq!(
                    settings.compaction_settings(Some(MODEL)),
                    Err(error),
                    "for {field} = {value} with a model override"
                );
            }
        }
    }

    #[test]
    fn reports_malformed_model_entries() {
        for (value, text) in [
            (Value::Null, "null"),
            (Value::Bool(false), "false"),
            (json!(42), "42"),
            (json!("invalid"), "invalid"),
            (json!([]), ""),
        ] {
            let settings = seeded(compaction(vec![(
                "modelOverrides",
                overrides(vec![(MODEL_KEY, value.clone())]),
            )]));
            assert_eq!(
                settings.compaction_settings(Some(MODEL)),
                Err(format!(
                    "Invalid compaction.modelOverrides[\"{MODEL_KEY}\"] setting: {text}. Expected an object."
                )),
                "for {value}"
            );
        }
    }

    #[test]
    fn accepts_zero_in_ordinary_settings_and_overrides() {
        let settings = SettingsManager::in_memory(
            object(json!({"compaction": {"reserveTokens": 0, "keepRecentTokens": 0}})),
            true,
        );
        assert_eq!(
            settings.compaction_settings(Some(MODEL)),
            Ok(CompactionSettings {
                enabled: true,
                reserve_tokens: 0,
                keep_recent_tokens: 0,
            })
        );

        let mut settings = settings;
        settings.apply_overrides(object(json!({
            "compaction": {
                "reserveTokens": 1000,
                "keepRecentTokens": 1000,
                "modelOverrides": {
                    "provider/family/model": {"reserveTokens": 0, "keepRecentTokens": 0},
                },
            },
        })));
        assert_eq!(
            settings.compaction_settings(Some(MODEL)),
            Ok(CompactionSettings {
                enabled: true,
                reserve_tokens: 0,
                keep_recent_tokens: 0,
            })
        );
    }
}

mod external_edit_preservation {
    use super::*;

    #[test]
    fn keeps_an_externally_emptied_packages_array() {
        let tree = Tree::new("bug-packages");
        tree.write_json(
            &tree.agent_settings(),
            json!({"theme": "dark", "packages": ["npm:pi-mcp-adapter"]}),
        );
        let mut settings = tree.manager();
        assert_eq!(settings.packages(), values(&["npm:pi-mcp-adapter"]));

        let mut file = tree.read_json(&tree.agent_settings());
        file["packages"] = json!([]);
        tree.write_json(&tree.agent_settings(), file);
        assert_eq!(
            tree.read_json(&tree.agent_settings())["packages"],
            json!([])
        );

        settings.set_theme("light");

        assert_eq!(
            tree.read_json(&tree.agent_settings()),
            json!({"packages": [], "theme": "light"})
        );
    }

    #[test]
    fn keeps_an_externally_replaced_extensions_array() {
        let tree = Tree::new("bug-extensions");
        tree.write_json(
            &tree.agent_settings(),
            json!({"theme": "dark", "extensions": ["/old/extension.ts"]}),
        );
        let mut settings = tree.manager();

        let mut file = tree.read_json(&tree.agent_settings());
        file["extensions"] = json!(["/new/extension.ts"]);
        tree.write_json(&tree.agent_settings(), file);

        settings.set_default_thinking_level(ModelThinkingLevel::High);

        assert_eq!(
            tree.read_json(&tree.agent_settings())["extensions"],
            json!(["/new/extension.ts"])
        );
    }

    #[test]
    fn keeps_external_project_changes_when_updating_another_project_field() {
        let tree = Tree::new("bug-project");
        tree.write_json(
            &tree.project_settings(),
            json!({"extensions": ["./old-extension.ts"], "prompts": ["./old-prompt.md"]}),
        );
        let mut settings = tree.manager();

        let mut file = tree.read_json(&tree.project_settings());
        file["prompts"] = json!(["./new-prompt.md"]);
        tree.write_json(&tree.project_settings(), file);

        settings
            .set_project_extension_paths(&strings(&["./updated-extension.ts"]))
            .expect("write project settings");

        assert_eq!(
            tree.read_json(&tree.project_settings()),
            json!({
                "prompts": ["./new-prompt.md"],
                "extensions": ["./updated-extension.ts"],
            })
        );
    }

    #[test]
    fn lets_an_in_memory_project_change_override_an_external_one() {
        let tree = Tree::new("bug-project-same-field");
        tree.write_json(
            &tree.project_settings(),
            json!({"extensions": ["./initial-extension.ts"]}),
        );
        let mut settings = tree.manager();

        let mut file = tree.read_json(&tree.project_settings());
        file["extensions"] = json!(["./external-extension.ts"]);
        tree.write_json(&tree.project_settings(), file);

        settings
            .set_project_extension_paths(&strings(&["./in-memory-extension.ts"]))
            .expect("write project settings");

        assert_eq!(
            tree.read_json(&tree.project_settings())["extensions"],
            json!(["./in-memory-extension.ts"])
        );
    }
}

mod diagnostics {
    use super::*;

    struct FailingStorage;

    impl SettingsStorage for FailingStorage {
        fn read(&self, scope: SettingsScope) -> Result<Option<String>, String> {
            match scope {
                SettingsScope::Global => Err("backend failed".to_owned()),
                SettingsScope::Project => Ok(None),
            }
        }

        fn write(&self, _scope: SettingsScope, _update: &mut Update<'_>) -> Result<(), String> {
            Ok(())
        }
    }

    fn warning(message: &str) -> SettingsDiagnostic {
        SettingsDiagnostic {
            kind: DiagnosticKind::Warning,
            message: message.to_owned(),
        }
    }

    #[test]
    fn includes_the_settings_file_path_for_file_backed_storage() {
        let tree = Tree::new("diagnostics-path");
        tree.write(&tree.agent_settings(), "{");

        let mut settings = SettingsManager::create(&tree.project(), &tree.agent(), true);
        let diagnostics = collect_diagnostics(&mut settings);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Warning);
        assert!(
            diagnostics[0]
                .message
                .starts_with(&format!("Invalid settings file {}:", tree.agent_settings())),
            "{}",
            diagnostics[0].message
        );
    }

    #[test]
    fn falls_back_to_the_settings_scope_without_file_paths() {
        let mut settings = SettingsManager::from_storage(Arc::new(FailingStorage), true);

        assert_eq!(
            collect_diagnostics(&mut settings),
            vec![warning("Invalid global settings: backend failed")]
        );
    }

    #[test]
    fn deduplicates_by_kind_and_message() {
        let duplicate = warning("Invalid settings file /tmp/settings.json");
        let mut other = duplicate.clone();
        other.kind = DiagnosticKind::Error;

        assert_eq!(
            deduplicate_diagnostics(&[duplicate.clone(), duplicate.clone(), other.clone()]),
            vec![duplicate, other]
        );
    }
}
