//! Canonical Lua-only configuration declarations and managed mutations.
//!
//! User configuration is evaluated from `<agentDir>/config.lua` followed by
//! `<cwd>/.pi/config.lua` (when the project is trusted). Both files receive the
//! same `pi.config` declaration table. Evaluation is transactional per file:
//! declarations are published only when the whole chunk succeeds.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::{Lua, Value as LuaValue};
use serde_json::{Map, Value};

use crate::convert::lua_to_json;

pub const MANAGED_BEGIN: &str = "-- pi managed settings: begin";
pub const MANAGED_END: &str = "-- pi managed settings: end";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigSnapshot {
    pub settings: Map<String, Value>,
    pub keybindings: Map<String, Value>,
    pub providers: BTreeMap<String, Value>,
    pub themes: BTreeMap<String, Value>,
    pub selectors: BTreeMap<String, ResourceSelector>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceSelector {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
}

impl ConfigSnapshot {
    /// Global declarations first, trusted project declarations second. Settings
    /// match Pi's one-level nested merge; named declarations merge by name.
    #[must_use]
    pub fn merged(global: &Self, project: &Self) -> Self {
        let mut merged = global.clone();
        merged.settings =
            crate::settings_manager::deep_merge_settings(&global.settings, &project.settings);
        for (key, value) in &project.keybindings {
            merged.keybindings.insert(key.clone(), value.clone());
        }
        merged.providers.extend(project.providers.clone());
        merged.themes.extend(project.themes.clone());
        merged.selectors.extend(project.selectors.clone());
        merged
    }

    /// CLI values are the final layer and use the settings merge rules.
    #[must_use]
    pub fn with_cli_overrides(&self, overrides: &Map<String, Value>) -> Self {
        let mut result = self.clone();
        result.settings = crate::settings_manager::deep_merge_settings(&result.settings, overrides);
        result
    }
}

fn object(value: LuaValue, label: &str) -> mlua::Result<Map<String, Value>> {
    match lua_to_json(value).map_err(|error| mlua::Error::runtime(error.to_string()))? {
        Value::Object(value) => Ok(value),
        _ => Err(mlua::Error::runtime(format!("{label} must be a table"))),
    }
}

fn string_list(value: LuaValue, label: &str) -> mlua::Result<Vec<String>> {
    match lua_to_json(value).map_err(|error| mlua::Error::runtime(error.to_string()))? {
        Value::Array(values) => values
            .into_iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| mlua::Error::runtime(format!("{label} entries must be strings")))
            })
            .collect(),
        Value::Object(values) if values.is_empty() => Ok(Vec::new()),
        _ => Err(mlua::Error::runtime(format!("{label} must be an array"))),
    }
}

fn value_list(value: LuaValue, label: &str) -> mlua::Result<Vec<Value>> {
    match lua_to_json(value).map_err(|error| mlua::Error::runtime(error.to_string()))? {
        Value::Array(values) => Ok(values),
        Value::Object(values) if values.is_empty() => Ok(Vec::new()),
        _ => Err(mlua::Error::runtime(format!("{label} must be an array"))),
    }
}

fn merge_settings(target: &mut Map<String, Value>, incoming: &Map<String, Value>) {
    *target = crate::settings_manager::deep_merge_settings(target, incoming);
}

/// Evaluate one config chunk through the canonical declaration surface.
pub fn evaluate(source: &str, source_name: &str) -> Result<ConfigSnapshot, String> {
    let lua = Lua::new();
    let state = Rc::new(RefCell::new(ConfigSnapshot::default()));
    let pi = lua.create_table().map_err(|error| error.to_string())?;
    let config = lua.create_table().map_err(|error| error.to_string())?;

    {
        let state = Rc::clone(&state);
        config
            .set(
                "settings",
                lua.create_function(move |_, value: LuaValue| {
                    let incoming = object(value, "config.settings")?;
                    merge_settings(&mut state.borrow_mut().settings, &incoming);
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let state = Rc::clone(&state);
        config
            .set(
                "keybindings",
                lua.create_function(move |_, value: LuaValue| {
                    let incoming = object(value, "config.keybindings")?;
                    state.borrow_mut().keybindings.extend(incoming);
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let state = Rc::clone(&state);
        config
            .set(
                "provider",
                lua.create_function(move |_, (name, value): (String, LuaValue)| {
                    if name.is_empty() {
                        return Err(mlua::Error::runtime(
                            "config.provider name must not be empty",
                        ));
                    }
                    let value = Value::Object(object(value, "config.provider")?);
                    state.borrow_mut().providers.insert(name, value);
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let state = Rc::clone(&state);
        config
            .set(
                "model",
                lua.create_function(move |_, (provider, value): (String, LuaValue)| {
                    let model = Value::Object(object(value, "config.model")?);
                    let mut state = state.borrow_mut();
                    let provider_value = state
                        .providers
                        .entry(provider)
                        .or_insert_with(|| Value::Object(Map::new()));
                    let Value::Object(provider) = provider_value else {
                        return Err(mlua::Error::runtime("config.model provider is not a table"));
                    };
                    let models = provider
                        .entry("models")
                        .or_insert_with(|| Value::Array(Vec::new()));
                    let Value::Array(models) = models else {
                        return Err(mlua::Error::runtime(
                            "config.model provider models is not an array",
                        ));
                    };
                    models.push(model);
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let state = Rc::clone(&state);
        config
            .set(
                "theme",
                lua.create_function(move |_, (name, value): (String, LuaValue)| {
                    if name.is_empty() {
                        return Err(mlua::Error::runtime("config.theme name must not be empty"));
                    }
                    let value = Value::Object(object(value, "config.theme")?);
                    state.borrow_mut().themes.insert(name, value);
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let state = Rc::clone(&state);
        config
            .set(
                "active_theme",
                lua.create_function(move |_, name: String| {
                    state
                        .borrow_mut()
                        .settings
                        .insert("theme".to_owned(), Value::String(name));
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }

    for (lua_name, settings_name) in [
        ("extensions", "extensions"),
        ("skills", "skills"),
        ("prompts", "prompts"),
        ("theme_paths", "themes"),
    ] {
        let state = Rc::clone(&state);
        config
            .set(
                lua_name,
                lua.create_function(move |_, value: LuaValue| {
                    let values = string_list(value, &format!("config.{lua_name}"))?;
                    state.borrow_mut().settings.insert(
                        settings_name.to_owned(),
                        Value::Array(values.into_iter().map(Value::String).collect()),
                    );
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let state = Rc::clone(&state);
        config
            .set(
                "packages",
                lua.create_function(move |_, value: LuaValue| {
                    state.borrow_mut().settings.insert(
                        "packages".to_owned(),
                        Value::Array(value_list(value, "config.packages")?),
                    );
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }

    for (lua_name, enabled) in [("enable", true), ("disable", false)] {
        let state = Rc::clone(&state);
        config
            .set(
                lua_name,
                lua.create_function(move |_, (kind, value): (String, LuaValue)| {
                    let values = string_list(value, &format!("config.{lua_name}"))?;
                    let mut state = state.borrow_mut();
                    let selector = state.selectors.entry(kind).or_default();
                    if enabled {
                        selector.enabled = values;
                    } else {
                        selector.disabled = values;
                    }
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }

    pi.set("config", config)
        .map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    let budget_ms = crate::DEFAULT_DISPATCH_TIMEOUT_MS;
    lua.set_global_hook(
        mlua::HookTriggers::new().every_nth_instruction(1000),
        move |_, _| {
            if started.elapsed().as_millis() >= budget_ms as u128 {
                return Err(mlua::Error::runtime(format!(
                    "config exceeded {budget_ms}ms of continuous Lua execution"
                )));
            }
            Ok(mlua::VmState::Continue)
        },
    )
    .map_err(|error| error.to_string())?;
    let result = lua
        .load(source)
        .set_name(format!("@{source_name}"))
        .call::<()>(pi)
        .map_err(|error| error.to_string());
    lua.remove_global_hook();
    result?;
    let snapshot = state.borrow().clone();
    Ok(snapshot)
}

fn lua_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn lua_value(value: &Value, indent: usize) -> String {
    match value {
        Value::Null => "nil".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => lua_quote(value),
        Value::Array(values) => {
            if values.is_empty() {
                return "{}".to_owned();
            }
            let pad = " ".repeat(indent + 2);
            let close = " ".repeat(indent);
            let body = values
                .iter()
                .map(|value| format!("{pad}{},", lua_value(value, indent + 2)))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{{\n{body}\n{close}}}")
        }
        Value::Object(values) => {
            if values.is_empty() {
                return "{}".to_owned();
            }
            let pad = " ".repeat(indent + 2);
            let close = " ".repeat(indent);
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            let body = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{pad}[{}] = {},",
                        lua_quote(key),
                        lua_value(&values[key], indent + 2)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("{{\n{body}\n{close}}}")
        }
    }
}

#[must_use]
pub fn managed_settings_block(settings: &Map<String, Value>) -> String {
    format!(
        "{MANAGED_BEGIN}\ndo\n  local pi = ...\n  pi.config.settings({})\nend\n{MANAGED_END}",
        lua_value(&Value::Object(settings.clone()), 2)
    )
}

fn marker_offsets(source: &str, marker: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if content == marker {
            offsets.push(offset);
        }
        offset += line.len();
    }
    offsets
}

fn managed_range(source: &str) -> Result<Option<(usize, usize)>, String> {
    let begins = marker_offsets(source, MANAGED_BEGIN);
    let ends = marker_offsets(source, MANAGED_END);
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([begin], [end]) if begin < end => Ok(Some((*begin, *end + MANAGED_END.len()))),
        ([], _) => Err("managed config block is missing its begin marker".to_owned()),
        (_, []) => Err("managed config block is missing its end marker".to_owned()),
        ([_], [_]) => Err("managed config block markers are out of order".to_owned()),
        _ => Err("config.lua contains multiple managed settings blocks".to_owned()),
    }
}

/// Settings currently owned by the generated block, excluding user declarations.
pub fn managed_settings(source: &str) -> Result<Map<String, Value>, String> {
    let Some((begin, end)) = managed_range(source)? else {
        return Ok(Map::new());
    };
    Ok(evaluate(&source[begin..end], "managed config block")?.settings)
}

/// Replace only pi's managed block. User bytes outside the block are retained.
/// Malformed/duplicate markers are left untouched; persistence validates them
/// through [`managed_settings`] and reports an attributed error before calling this.
#[must_use]
pub fn update_managed_settings(source: &str, settings: &Map<String, Value>) -> String {
    let block = managed_settings_block(settings);
    match managed_range(source) {
        Ok(Some((begin, end))) => {
            return format!("{}{}{}", &source[..begin], block, &source[end..]);
        }
        Err(_) => return source.to_owned(),
        Ok(None) => {}
    }
    if source.is_empty() {
        return format!("{block}\n");
    }
    let separator = if source.ends_with('\n') { "\n" } else { "\n\n" };
    format!("{source}{separator}{block}\n")
}

fn read_scope(path: &std::path::Path) -> Result<ConfigSnapshot, String> {
    match std::fs::read_to_string(path) {
        Ok(source) => evaluate(&source, &path.to_string_lossy()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigSnapshot::default()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Install the declaration API used by config files onto an ordinary host VM.
/// Initial state is evaluated from the same canonical files as SettingsManager.
pub(crate) fn install_runtime(
    lua: &Lua,
    pi: &mlua::Table,
    cwd: &str,
    project_trusted: bool,
    settings: crate::settings::SharedSettings,
) -> mlua::Result<()> {
    let agent_dir = std::path::PathBuf::from(crate::discover::agent_dir());
    let global_path = agent_dir.join("config.lua");
    let project_path = std::path::Path::new(cwd).join(".pi/config.lua");
    let mut errors = Vec::new();
    let global = match read_scope(&global_path) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            errors.push(error);
            ConfigSnapshot::default()
        }
    };
    let project = if project_trusted {
        match read_scope(&project_path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                errors.push(error);
                ConfigSnapshot::default()
            }
        }
    } else {
        ConfigSnapshot::default()
    };
    let state = Rc::new(RefCell::new(ConfigSnapshot::merged(&global, &project)));
    let table = lua.create_table()?;

    {
        let state = Rc::clone(&state);
        let settings = std::sync::Arc::clone(&settings);
        table.set(
            "settings",
            lua.create_function(move |_, value: LuaValue| {
                let incoming = object(value, "config.settings")?;
                merge_settings(&mut state.borrow_mut().settings, &incoming);
                settings
                    .lock()
                    .map_err(|_| mlua::Error::runtime("settings store poisoned"))?
                    .apply_overrides(&incoming);
                Ok(())
            })?,
        )?;
    }
    {
        let state = Rc::clone(&state);
        table.set(
            "keybindings",
            lua.create_function(move |_, value: LuaValue| {
                state
                    .borrow_mut()
                    .keybindings
                    .extend(object(value, "config.keybindings")?);
                Ok(())
            })?,
        )?;
    }
    {
        let state = Rc::clone(&state);
        table.set(
            "provider",
            lua.create_function(move |_, (name, value): (String, LuaValue)| {
                state
                    .borrow_mut()
                    .providers
                    .insert(name, Value::Object(object(value, "config.provider")?));
                Ok(())
            })?,
        )?;
    }
    {
        let state = Rc::clone(&state);
        table.set(
            "model",
            lua.create_function(move |_, (provider_name, value): (String, LuaValue)| {
                let model = Value::Object(object(value, "config.model")?);
                let mut state = state.borrow_mut();
                let provider = state
                    .providers
                    .entry(provider_name)
                    .or_insert_with(|| Value::Object(Map::new()));
                let Value::Object(provider) = provider else {
                    return Err(mlua::Error::runtime("config.model provider is not a table"));
                };
                let models = provider
                    .entry("models")
                    .or_insert_with(|| Value::Array(Vec::new()));
                let Value::Array(models) = models else {
                    return Err(mlua::Error::runtime(
                        "config.model provider models is not an array",
                    ));
                };
                models.push(model);
                Ok(())
            })?,
        )?;
    }
    {
        let state = Rc::clone(&state);
        table.set(
            "theme",
            lua.create_function(move |_, (name, value): (String, LuaValue)| {
                state
                    .borrow_mut()
                    .themes
                    .insert(name, Value::Object(object(value, "config.theme")?));
                Ok(())
            })?,
        )?;
    }
    {
        let state = Rc::clone(&state);
        let settings = std::sync::Arc::clone(&settings);
        table.set(
            "active_theme",
            lua.create_function(move |_, name: String| {
                let incoming = Map::from_iter([("theme".to_owned(), Value::String(name))]);
                merge_settings(&mut state.borrow_mut().settings, &incoming);
                settings
                    .lock()
                    .map_err(|_| mlua::Error::runtime("settings store poisoned"))?
                    .apply_overrides(&incoming);
                Ok(())
            })?,
        )?;
    }

    for (lua_name, settings_name) in [
        ("extensions", "extensions"),
        ("skills", "skills"),
        ("prompts", "prompts"),
        ("theme_paths", "themes"),
    ] {
        let state = Rc::clone(&state);
        let settings = std::sync::Arc::clone(&settings);
        table.set(
            lua_name,
            lua.create_function(move |_, value: LuaValue| {
                let values = Value::Array(
                    string_list(value, &format!("config.{lua_name}"))?
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                );
                let incoming = Map::from_iter([(settings_name.to_owned(), values)]);
                merge_settings(&mut state.borrow_mut().settings, &incoming);
                settings
                    .lock()
                    .map_err(|_| mlua::Error::runtime("settings store poisoned"))?
                    .apply_overrides(&incoming);
                Ok(())
            })?,
        )?;
    }
    {
        let state = Rc::clone(&state);
        let settings = std::sync::Arc::clone(&settings);
        table.set(
            "packages",
            lua.create_function(move |_, value: LuaValue| {
                let incoming = Map::from_iter([(
                    "packages".to_owned(),
                    Value::Array(value_list(value, "config.packages")?),
                )]);
                merge_settings(&mut state.borrow_mut().settings, &incoming);
                settings
                    .lock()
                    .map_err(|_| mlua::Error::runtime("settings store poisoned"))?
                    .apply_overrides(&incoming);
                Ok(())
            })?,
        )?;
    }

    for (lua_name, enabled) in [("enable", true), ("disable", false)] {
        let state = Rc::clone(&state);
        table.set(
            lua_name,
            lua.create_function(move |_, (kind, value): (String, LuaValue)| {
                let values = string_list(value, &format!("config.{lua_name}"))?;
                let mut state = state.borrow_mut();
                let selector = state.selectors.entry(kind).or_default();
                if enabled {
                    selector.enabled = values;
                } else {
                    selector.disabled = values;
                }
                Ok(())
            })?,
        )?;
    }

    {
        let state = Rc::clone(&state);
        let settings = std::sync::Arc::clone(&settings);
        let global_path = global_path.clone();
        let project_path = project_path.clone();
        table.set(
            "reload",
            lua.create_function(move |_, ()| {
                // Build the complete next declaration graph before touching live state.
                let global = read_scope(&global_path).map_err(mlua::Error::runtime)?;
                let project = if project_trusted {
                    read_scope(&project_path).map_err(mlua::Error::runtime)?
                } else {
                    ConfigSnapshot::default()
                };
                let next = ConfigSnapshot::merged(&global, &project);
                settings
                    .lock()
                    .map_err(|_| mlua::Error::runtime("settings store poisoned"))?
                    .try_reload()
                    .map_err(mlua::Error::external)?;
                *state.borrow_mut() = next;
                Ok(())
            })?,
        )?;
    }

    {
        let state = Rc::clone(&state);
        table.set(
            "snapshot",
            lua.create_function(move |lua, ()| {
                let state = state.borrow();
                let selectors = state
                    .selectors
                    .iter()
                    .map(|(kind, selector)| {
                        (
                            kind.clone(),
                            serde_json::json!({
                                "enabled": selector.enabled,
                                "disabled": selector.disabled
                            }),
                        )
                    })
                    .collect::<Map<String, Value>>();
                let value = serde_json::json!({
                    "settings": state.settings,
                    "keybindings": state.keybindings,
                    "providers": state.providers,
                    "themes": state.themes,
                    "selectors": selectors,
                });
                crate::convert::json_to_lua(lua, &value)
            })?,
        )?;
    }
    table.set(
        "errors",
        lua.create_function(move |lua, ()| {
            crate::convert::json_to_lua(lua, &serde_json::json!(errors))
        })?,
    )?;
    pi.set("config", table)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_merge_and_cover_every_config_kind() {
        let snapshot = evaluate(
            r#"
local pi = ...
pi.config.settings({ theme = "dark", retry = { enabled = true, maxRetries = 2 } })
pi.config.settings({ retry = { enabled = false } })
pi.config.keybindings({ ["app.exit"] = { "ctrl+d", "ctrl+q" } })
pi.config.provider("local", { baseUrl = "http://localhost" })
pi.config.model("local", { id = "one" })
pi.config.theme("paper", { dark = false })
pi.config.active_theme("paper")
pi.config.extensions({ "a.lua" })
pi.config.skills({ "skills" })
pi.config.prompts({ "prompts" })
pi.config.packages({ "pkg", { source = "git:demo", extensions = { "one.lua" } } })
pi.config.theme_paths({ "themes" })
pi.config.enable("extensions", { "a.lua" })
pi.config.disable("skills", { "old" })
"#,
            "config.lua",
        )
        .unwrap();
        assert_eq!(snapshot.settings["theme"], "paper");
        assert_eq!(snapshot.settings["retry"]["enabled"], false);
        assert_eq!(snapshot.settings["retry"]["maxRetries"], 2);
        assert_eq!(snapshot.settings["extensions"][0], "a.lua");
        assert_eq!(snapshot.settings["packages"][1]["source"], "git:demo");
        assert_eq!(snapshot.providers["local"]["models"][0]["id"], "one");
        assert_eq!(snapshot.themes["paper"]["dark"], false);
        assert_eq!(snapshot.selectors["extensions"].enabled, ["a.lua"]);
        assert_eq!(snapshot.selectors["skills"].disabled, ["old"]);
    }

    #[test]
    fn failed_chunk_publishes_nothing() {
        assert!(
            evaluate(
                "local pi = ...; pi.config.settings({ theme = 'light' }); error('broken')",
                "/tmp/config.lua"
            )
            .unwrap_err()
            .contains("/tmp/config.lua")
        );
    }

    #[test]
    fn managed_mutation_is_idempotent_and_preserves_user_code() {
        let source = "local pi = ...\npi.config.settings({ quietStartup = true })\n";
        let settings = Map::from_iter([
            ("theme".to_owned(), Value::String("light".to_owned())),
            ("enabledModels".to_owned(), serde_json::json!(["a", "b"])),
        ]);
        let once = update_managed_settings(source, &settings);
        let twice = update_managed_settings(&once, &settings);
        assert_eq!(once, twice);
        assert!(once.starts_with(source));
        assert_eq!(
            evaluate(&once, "config.lua").unwrap().settings["theme"],
            "light"
        );
    }

    #[test]
    fn managed_markers_do_not_match_user_strings_or_clobber_malformed_blocks() {
        let settings = Map::from_iter([("theme".to_owned(), Value::String("light".to_owned()))]);
        let source = format!("local marker = {MANAGED_BEGIN:?}\n");
        let updated = update_managed_settings(&source, &settings);
        assert!(updated.starts_with(&source));
        assert_eq!(managed_settings(&updated).unwrap()["theme"], "light");

        let malformed = format!("local pi = ...\n{MANAGED_BEGIN}\n");
        assert_eq!(update_managed_settings(&malformed, &settings), malformed);
        assert!(
            managed_settings(&malformed)
                .unwrap_err()
                .contains("end marker")
        );
    }
}
