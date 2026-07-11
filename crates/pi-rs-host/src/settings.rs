//! `pi.settings` — Pi's settings manager exposed as a per-VM mechanism.
//! Product policy (when settings are read/changed and all UI) stays in Lua.

use std::sync::{Arc, Mutex};

use mlua::{Lua, Table};

use crate::{
    convert::{json_to_lua, lua_to_json},
    settings_manager::{SettingsManager, SettingsManagerCreateOptions, parse_thinking_level},
};

pub(crate) type SharedSettings = Arc<Mutex<SettingsManager>>;

fn lock(settings: &SharedSettings) -> mlua::Result<std::sync::MutexGuard<'_, SettingsManager>> {
    settings
        .lock()
        .map_err(|_| mlua::Error::runtime("settings store poisoned"))
}

pub(crate) fn install(lua: &Lua, pi: &Table, cwd: &str, project_trusted: bool) -> mlua::Result<()> {
    let settings: SharedSettings = Arc::new(Mutex::new(SettingsManager::create(
        std::path::Path::new(cwd),
        None,
        SettingsManagerCreateOptions {
            project_trusted: Some(project_trusted),
            ..Default::default()
        },
    )));
    let table = lua.create_table()?;

    macro_rules! getter {
        ($lua_name:literal, $method:ident) => {{
            let store = Arc::clone(&settings);
            table.set(
                $lua_name,
                lua.create_function(move |_, ()| Ok(lock(&store)?.$method()))?,
            )?;
        }};
    }
    macro_rules! bool_setter {
        ($lua_name:literal, $method:ident) => {{
            let store = Arc::clone(&settings);
            table.set(
                $lua_name,
                lua.create_function(move |_, value: bool| {
                    lock(&store)?.$method(value);
                    Ok(())
                })?,
            )?;
        }};
    }
    macro_rules! string_setter {
        ($lua_name:literal, $method:ident) => {{
            let store = Arc::clone(&settings);
            table.set(
                $lua_name,
                lua.create_function(move |_, value: String| {
                    lock(&store)?.$method(&value);
                    Ok(())
                })?,
            )?;
        }};
    }
    macro_rules! integer_setter {
        ($lua_name:literal, $method:ident) => {{
            let store = Arc::clone(&settings);
            table.set(
                $lua_name,
                lua.create_function(move |_, value: u64| {
                    lock(&store)?.$method(value);
                    Ok(())
                })?,
            )?;
        }};
    }

    getter!("block_images", get_block_images);
    bool_setter!("set_block_images", set_block_images);
    getter!("compaction_enabled", get_compaction_enabled);
    bool_setter!("set_compaction_enabled", set_compaction_enabled);
    getter!("show_images", get_show_images);
    bool_setter!("set_show_images", set_show_images);
    getter!("image_width_cells", get_image_width_cells);
    integer_setter!("set_image_width_cells", set_image_width_cells);
    getter!("image_auto_resize", get_image_auto_resize);
    bool_setter!("set_image_auto_resize", set_image_auto_resize);
    getter!("enable_skill_commands", get_enable_skill_commands);
    bool_setter!("set_enable_skill_commands", set_enable_skill_commands);
    getter!("steering_mode", get_steering_mode);
    string_setter!("set_steering_mode", set_steering_mode);
    getter!("follow_up_mode", get_follow_up_mode);
    string_setter!("set_follow_up_mode", set_follow_up_mode);
    getter!("transport", get_transport);
    string_setter!("set_transport", set_transport);
    getter!("hide_thinking_block", get_hide_thinking_block);
    bool_setter!("set_hide_thinking_block", set_hide_thinking_block);
    getter!("collapse_changelog", get_collapse_changelog);
    bool_setter!("set_collapse_changelog", set_collapse_changelog);
    getter!("last_changelog_version", get_last_changelog_version);
    string_setter!("set_last_changelog_version", set_last_changelog_version);
    getter!("enable_install_telemetry", get_enable_install_telemetry);
    bool_setter!("set_enable_install_telemetry", set_enable_install_telemetry);
    getter!("quiet_startup", get_quiet_startup);
    bool_setter!("set_quiet_startup", set_quiet_startup);
    getter!("default_project_trust", get_default_project_trust);
    string_setter!("set_default_project_trust", set_default_project_trust);
    getter!("double_escape_action", get_double_escape_action);
    string_setter!("set_double_escape_action", set_double_escape_action);
    getter!("tree_filter_mode", get_tree_filter_mode);
    string_setter!("set_tree_filter_mode", set_tree_filter_mode);
    getter!("show_hardware_cursor", get_show_hardware_cursor);
    bool_setter!("set_show_hardware_cursor", set_show_hardware_cursor);
    getter!("editor_padding_x", get_editor_padding_x);
    integer_setter!("set_editor_padding_x", set_editor_padding_x);
    getter!("autocomplete_max_visible", get_autocomplete_max_visible);
    integer_setter!("set_autocomplete_max_visible", set_autocomplete_max_visible);
    getter!("clear_on_shrink", get_clear_on_shrink);
    bool_setter!("set_clear_on_shrink", set_clear_on_shrink);
    getter!("show_terminal_progress", get_show_terminal_progress);
    bool_setter!("set_show_terminal_progress", set_show_terminal_progress);
    getter!("theme", get_theme);
    string_setter!("set_theme", set_theme);
    getter!("enabled_models", get_enabled_models);

    let store = Arc::clone(&settings);
    table.set(
        "set_enabled_models",
        lua.create_function(move |_, patterns: Option<Vec<String>>| {
            lock(&store)?.set_enabled_models(patterns.as_deref());
            Ok(())
        })?,
    )?;

    let store = Arc::clone(&settings);
    table.set(
        "http_idle_timeout_ms",
        lua.create_function(move |_, ()| {
            lock(&store)?
                .get_http_idle_timeout_ms()
                .map_err(mlua::Error::external)
        })?,
    )?;
    integer_setter!("set_http_idle_timeout_ms", set_http_idle_timeout_ms);

    let store = Arc::clone(&settings);
    table.set(
        "set_default_model_and_provider",
        lua.create_function(move |_, (provider, model): (String, String)| {
            lock(&store)?.set_default_model_and_provider(&provider, &model);
            Ok(())
        })?,
    )?;

    let store = Arc::clone(&settings);
    table.set(
        "warnings",
        lua.create_function(move |lua, ()| {
            json_to_lua(
                lua,
                &serde_json::Value::Object(lock(&store)?.get_warnings()),
            )
        })?,
    )?;
    let store = Arc::clone(&settings);
    table.set(
        "set_warnings",
        lua.create_function(move |_, warnings: mlua::Value| {
            let warnings = match lua_to_json(warnings)? {
                serde_json::Value::Object(warnings) => warnings,
                _ => return Err(mlua::Error::runtime("warnings must be an object")),
            };
            lock(&store)?.set_warnings(warnings);
            Ok(())
        })?,
    )?;

    let store = Arc::clone(&settings);
    table.set(
        "compaction_settings",
        lua.create_function(move |lua, ()| {
            let compaction = lock(&store)?.get_compaction_settings();
            let result = lua.create_table()?;
            result.set("enabled", compaction.enabled)?;
            result.set("reserveTokens", compaction.reserve_tokens)?;
            result.set("keepRecentTokens", compaction.keep_recent_tokens)?;
            Ok(result)
        })?,
    )?;

    let store = Arc::clone(&settings);
    table.set(
        "retry_settings",
        lua.create_function(move |lua, ()| {
            let retry = lock(&store)?.get_retry_settings();
            let result = lua.create_table()?;
            result.set("enabled", retry.enabled)?;
            result.set("maxRetries", retry.max_retries)?;
            result.set("baseDelayMs", retry.base_delay_ms)?;
            Ok(result)
        })?,
    )?;

    getter!("shell_command_prefix", get_shell_command_prefix);
    getter!("shell_path", get_shell_path);

    let store = Arc::clone(&settings);
    table.set(
        "default_thinking_level",
        lua.create_function(move |_, ()| {
            Ok(lock(&store)?
                .get_default_thinking_level()
                .and_then(|level| serde_json::to_value(level).ok())
                .and_then(|value| value.as_str().map(str::to_owned)))
        })?,
    )?;
    let store = Arc::clone(&settings);
    table.set(
        "set_default_thinking_level",
        lua.create_function(move |_, level: String| {
            let level = parse_thinking_level(&level)
                .ok_or_else(|| mlua::Error::runtime(format!("invalid thinking level: {level}")))?;
            lock(&store)?.set_default_thinking_level(level);
            Ok(())
        })?,
    )?;

    let store = Arc::clone(&settings);
    table.set(
        "branch_summary",
        lua.create_function(move |lua, ()| {
            let branch_summary = lock(&store)?.get_branch_summary_settings();
            let result = lua.create_table()?;
            result.set("reserveTokens", branch_summary.reserve_tokens)?;
            result.set("skipPrompt", branch_summary.skip_prompt)?;
            Ok(result)
        })?,
    )?;

    let store = Arc::clone(&settings);
    table.set(
        "reload",
        lua.create_function(move |_, ()| {
            lock(&store)?.reload();
            Ok(())
        })?,
    )?;

    pi.set("settings", table)?;
    Ok(())
}
