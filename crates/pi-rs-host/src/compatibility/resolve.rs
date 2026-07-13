use crate::runtime_registry::registry_table;
use std::collections::{HashMap, HashSet};

/// Snapshot the handler list for `event` before dispatching so a handler
/// that subscribes new handlers mid-emit doesn't alter this dispatch.
pub(crate) fn event_handlers(
    lua: &mlua::Lua,
    event: &str,
) -> mlua::Result<Vec<(String, mlua::Function)>> {
    let mut handlers = Vec::new();
    for entry in crate::kernel_api::declaration_entries(lua, "event")? {
        let is_bus = entry.get::<Option<bool>>("bus")?.unwrap_or(false);
        if !is_bus && entry.get::<String>("event")? == event {
            handlers.push((entry.get("source")?, entry.get("fn")?));
        }
    }
    Ok(handlers)
}

/// Roll back every registration attributed to a source whose top-level chunk
/// failed. Pi constructs an extension off-registry and publishes it only after
/// its async factory resolves; this gives direct Lua chunks the same atomicity.
pub(crate) fn remove_scope(lua: &mlua::Lua, scope: crate::kernel::ScopeId) -> mlua::Result<()> {
    crate::kernel_api::remove_scope(lua, scope)?;
    let registry = registry_table(lua)?;

    let packages: mlua::Table = registry.get("package_options")?;
    let kept_packages = lua.create_table()?;
    for entry in packages.sequence_values::<mlua::Table>() {
        let entry = entry?;
        if entry.get::<u64>("scope")? != scope.get() {
            kept_packages.push(entry)?;
        }
    }
    registry.set("package_options", kept_packages)?;

    let owners: mlua::Table = registry.get("flag_value_owners")?;
    let values: mlua::Table = registry.get("flag_values")?;
    let names = owners
        .clone()
        .pairs::<String, u64>()
        .filter_map(|pair| match pair {
            Ok((name, owner)) if owner == scope.get() => Some(Ok(name)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<mlua::Result<Vec<_>>>()?;
    for name in names {
        owners.set(name.as_str(), mlua::Nil)?;
        values.set(name.as_str(), mlua::Nil)?;
    }

    let modules: mlua::Table = registry.get("modules")?;
    let module_order: mlua::Table = registry.get("module_order")?;
    let kept_modules = lua.create_table()?;
    for key in module_order.sequence_values::<String>() {
        let key = key?;
        let Some(entry) = modules.get::<Option<mlua::Table>>(key.as_str())? else {
            continue;
        };
        if entry.get::<u64>("scope")? == scope.get() {
            modules.set(key.as_str(), mlua::Nil)?;
        } else {
            entry.set("state", "defined")?;
            entry.set("value", mlua::Nil)?;
            kept_modules.push(key)?;
        }
    }
    registry.set("module_order", kept_modules)?;
    registry.set("module_stack", lua.create_table()?)?;
    Ok(())
}

/// Flatten registrations of one kind: extension order (load order), then
/// per-extension insertion order — the iteration order of the spec's
/// nested `Map`s.
pub(crate) fn registrations(
    lua: &mlua::Lua,
    map_key: &str,
    _order_key: &str,
) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let kind = match map_key {
        "tools" => "tool",
        "commands" => "command",
        "providers" => "provider",
        "shortcuts" => "keymap",
        "flags" => "flag",
        "render_middleware" => "renderer",
        "ui_slots" => "ui_slot",
        other => {
            return Err(mlua::Error::runtime(format!(
                "unknown declaration family {other}"
            )));
        }
    };
    crate::kernel_api::effective_declaration_entries(lua, kind)?
        .into_iter()
        .map(|entry| {
            let source = entry.get("source")?;
            let id = entry.get("declaration_id")?;
            Ok((source, id, entry))
        })
        .collect()
}

/// Spec `runner.getAllRegisteredTools()`: first registration per name
/// wins across extensions. Returns `(source, name, definition)`.
pub(crate) fn all_tools(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let mut seen = HashSet::new();
    Ok(registrations(lua, "tools", "tool_order")?
        .into_iter()
        .filter(|(_, name, _)| seen.insert(name.clone()))
        .collect())
}

pub(crate) fn extension_conflicts(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String)>> {
    let mut conflicts = Vec::new();
    for (map_key, order_key, label, decoration) in [
        ("tools", "tool_order", "Tool", ""),
        ("flags", "flag_order", "Flag", "--"),
    ] {
        let mut owners = HashMap::<String, String>::new();
        for (source, name, _) in registrations(lua, map_key, order_key)? {
            if let Some(owner) = owners.get(&name) {
                if owner != &source {
                    conflicts.push((
                        source.clone(),
                        format!("{label} \"{decoration}{name}\" conflicts with {owner}"),
                    ));
                }
            } else {
                owners.insert(name, source);
            }
        }
    }
    Ok(conflicts)
}

/// Spec `ExtensionRunner.getFlags()`: first registration per name wins.
pub(crate) fn all_flags(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let mut seen = HashSet::new();
    Ok(registrations(lua, "flags", "flag_order")?
        .into_iter()
        .filter(|(_, name, _)| seen.insert(name.clone()))
        .collect())
}

pub(crate) fn set_flag_value(
    lua: &mlua::Lua,
    name: &str,
    value: &serde_json::Value,
) -> mlua::Result<()> {
    let values: mlua::Table = registry_table(lua)?.get("flag_values")?;
    values.set(name, crate::convert::json_to_lua(lua, value)?)
}

/// Spec `runner.getToolDefinition()`: the first extension that registered
/// the name.
pub(crate) fn find_tool(
    lua: &mlua::Lua,
    name: &str,
) -> mlua::Result<Option<(String, mlua::Table)>> {
    for (source, n, def) in all_tools(lua)? {
        if n == name {
            return Ok(Some((source, def)));
        }
    }
    Ok(None)
}

/// JSON metadata mirror of a tool definition: every field except
/// functions (`execute`, `prepare_arguments`) — handed to the host
/// uninterpreted.
pub(crate) fn tool_meta(def: &mlua::Table) -> mlua::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for pair in def.pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair?;
        if matches!(v, mlua::Value::Function(_)) {
            continue;
        }
        let key = match k {
            mlua::Value::String(s) => s.to_str()?.to_owned(),
            mlua::Value::Integer(i) => i.to_string(),
            _ => continue,
        };
        if matches!(
            key.as_str(),
            "kind" | "declaration_id" | "source" | "scope" | "sequence" | "order"
        ) {
            continue;
        }
        map.insert(key, crate::convert::lua_to_json(v)?);
    }
    Ok(serde_json::Value::Object(map))
}

/// All provider registrations in extension load order, then
/// per-extension registration order (spec: the runner drains queued
/// `registerProvider` calls in order; the consumer applies the spec's
/// global upsert). Returns `(source, name, config)`.
pub(crate) fn all_providers(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    registrations(lua, "providers", "provider_order")
}

/// JSON mirror of a provider config: function values are stripped at
/// any depth (functions never cross the bridge — `streamSimple` and the
/// `oauth` callbacks stay Lua-side for later invocation; non-function
/// fields like `oauth.name` survive).
pub(crate) fn provider_meta(config: &mlua::Table) -> mlua::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for pair in config.pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair?;
        if matches!(v, mlua::Value::Function(_)) {
            continue;
        }
        let key = match k {
            mlua::Value::String(s) => s.to_str()?.to_owned(),
            mlua::Value::Integer(i) => i.to_string(),
            _ => continue,
        };
        if matches!(
            key.as_str(),
            "kind" | "declaration_id" | "source" | "scope" | "sequence" | "order" | "removed"
        ) {
            continue;
        }
        let value = match v {
            mlua::Value::Table(t) => provider_table_value(&t)?,
            other => crate::convert::lua_to_json(other)?,
        };
        map.insert(key, value);
    }
    Ok(serde_json::Value::Object(map))
}

/// [`provider_meta`] recursion: array tables keep their shape (function
/// entries would shift positions, so they become `null`); map tables
/// drop function values.
fn provider_table_value(t: &mlua::Table) -> mlua::Result<serde_json::Value> {
    let len = t.raw_len();
    let mut is_array = len > 0;
    if is_array {
        let mut count = 0;
        for pair in t.pairs::<mlua::Value, mlua::Value>() {
            let (k, _) = pair?;
            match k {
                mlua::Value::Integer(i) if i >= 1 && (i as usize) <= len => count += 1,
                _ => {
                    is_array = false;
                    break;
                }
            }
        }
        if is_array {
            is_array = count == len;
        }
    }
    if is_array {
        let mut arr = Vec::with_capacity(len);
        for i in 1..=len {
            let v: mlua::Value = t.get(i)?;
            arr.push(match v {
                mlua::Value::Function(_) => serde_json::Value::Null,
                mlua::Value::Table(inner) => provider_table_value(&inner)?,
                other => crate::convert::lua_to_json(other)?,
            });
        }
        return Ok(serde_json::Value::Array(arr));
    }
    provider_meta(t)
}

pub(crate) struct ResolvedCommand {
    pub(crate) source: String,
    pub(crate) name: String,
    pub(crate) invocation_name: String,
    pub(crate) description: Option<String>,
    pub(crate) get_argument_completions: Option<mlua::Function>,
    pub(crate) handler: mlua::Function,
    pub(crate) entry: mlua::Table,
}

/// Spec `runner.resolveRegisteredCommands()`: flatten in extension order;
/// names registered by more than one extension get `name:occurrence`
/// invocation names, bumping the suffix past already-taken names.
pub(crate) fn resolved_commands(lua: &mlua::Lua) -> mlua::Result<Vec<ResolvedCommand>> {
    let regs = registrations(lua, "commands", "command_order")?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, name, _) in &regs {
        *counts.entry(name.clone()).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(regs.len());
    for (source, name, entry) in regs {
        let occurrence = seen.get(&name).copied().unwrap_or(0) + 1;
        seen.insert(name.clone(), occurrence);
        let mut invocation_name = if counts.get(&name).copied().unwrap_or(0) > 1 {
            format!("{name}:{occurrence}")
        } else {
            name.clone()
        };
        if taken.contains(&invocation_name) {
            let mut suffix = occurrence;
            loop {
                suffix += 1;
                invocation_name = format!("{name}:{suffix}");
                if !taken.contains(&invocation_name) {
                    break;
                }
            }
        }
        taken.insert(invocation_name.clone());
        let description = entry.get::<Option<String>>("description")?;
        let get_argument_completions =
            entry.get::<Option<mlua::Function>>("get_argument_completions")?;
        let handler: mlua::Function = entry.get("handler")?;
        out.push(ResolvedCommand {
            source,
            name,
            invocation_name,
            description,
            get_argument_completions,
            handler,
            entry,
        });
    }
    Ok(out)
}

pub(crate) fn command_is_public(
    lua: &mlua::Lua,
    source: &str,
    command: &mlua::Table,
) -> mlua::Result<bool> {
    if let Some(visibility) = command.get::<Option<String>>("visibility")? {
        return Ok(visibility != "internal");
    }
    let packages: mlua::Table = registry_table(lua)?.get("package_options")?;
    let mut visibility = "public".to_owned();
    for package in packages.sequence_values::<mlua::Table>() {
        let package = package?;
        if package.get::<String>("source")? == source {
            visibility = package.get("command_visibility")?;
        }
    }
    Ok(visibility != "internal")
}

pub(crate) fn all_roles(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let mut roles = crate::kernel_api::root_entries(lua, "role")?;
    roles.sort_by_key(|entry| {
        (
            entry.get::<u64>("scope").unwrap_or_default(),
            entry.get::<String>("id").unwrap_or_default(),
        )
    });
    roles
        .into_iter()
        .map(|entry| {
            let source = entry.get("source")?;
            let id = entry.get("id")?;
            Ok((source, id, entry))
        })
        .collect()
}

pub(crate) struct ResolvedRole {
    pub(crate) source: String,
    pub(crate) handler: mlua::Function,
}

pub(crate) fn resolve_role(lua: &mlua::Lua, requested: &str) -> mlua::Result<Option<ResolvedRole>> {
    let mut candidates = Vec::new();
    for (source, _, declaration) in all_roles(lua)? {
        if declaration.get::<String>("role")? == requested && declaration.get::<bool>("active")? {
            let priority = declaration.get::<i64>("priority")?;
            let id = declaration.get::<String>("id")?;
            candidates.push((priority, source, id, declaration));
        }
    }
    let Some(max_priority) = candidates.iter().map(|candidate| candidate.0).max() else {
        return Ok(None);
    };
    let mut selected = candidates
        .into_iter()
        .filter(|candidate| candidate.0 == max_priority);
    let Some((_, source, id, declaration)) = selected.next() else {
        return Ok(None);
    };
    if let Some((_, other_source, other_id, _)) = selected.next() {
        return Err(mlua::Error::runtime(format!(
            "role {requested:?} has conflicting active declarations at priority {max_priority}: {id:?} ({source}) and {other_id:?} ({other_source})"
        )));
    }
    Ok(Some(ResolvedRole {
        source,
        handler: declaration.get("dispatch")?,
    }))
}
