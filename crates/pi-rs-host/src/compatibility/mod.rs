//! Thin retained public names over canonical kernel roots/declarations.

mod resolve;

pub(crate) use resolve::*;

pub(crate) use crate::runtime_registry::{current_source, registry_table, set_current_source};
fn adapter_id(lua: &mlua::Lua, family: &str) -> mlua::Result<String> {
    let registry = registry_table(lua)?;
    let sequence = registry.get::<u64>("adapter_sequence")?;
    registry.set("adapter_sequence", sequence + 1)?;
    Ok(format!("{family}\0{sequence}"))
}

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    // Mechanisms used by the Lua-authored agent loop. Definitions stay in
    // Lua (including execute/prepare functions); this returns the resolved
    // first-registration-wins view rather than a JSON metadata mirror.
    pi.set(
        "registered_tools",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for (_, _, def) in all_tools(lua)? {
                result.push(def)?;
            }
            Ok(result)
        })?,
    )?;
    // Product activation is declaration data on each tool, never inferred
    // from whether its source key happens to be synthetic. Ordinary tools
    // default active for Pi compatibility; shipped tools state the field
    // explicitly in their package.
    let registered_active_tools = lua.create_function(|lua, ()| {
        let result = lua.create_table()?;
        for (_, _, def) in all_tools(lua)? {
            if def
                .get::<Option<bool>>("active_by_default")?
                .unwrap_or(true)
            {
                result.push(def)?;
            }
        }
        Ok(result)
    })?;
    pi.set("registered_active_tools", registered_active_tools.clone())?;
    // Compatibility alias for the first product-loading slice. Its behavior
    // is now declaration-driven and source-identity-neutral.
    pi.set("registered_extension_tools", registered_active_tools)?;
    pi.set(
        "registered_extension_commands",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for command in resolved_commands(lua)? {
                if !command_is_public(lua, &command.source, &command.entry)? {
                    continue;
                }
                let entry = lua.create_table()?;
                entry.set("name", command.name)?;
                entry.set("invocation_name", command.invocation_name)?;
                entry.set("source", command.source.as_str())?;
                entry.set("description", command.description)?;
                let source_info = lua.create_table()?;
                source_info.set("path", command.source.as_str())?;
                source_info.set("source", "cli")?;
                source_info.set("scope", "temporary")?;
                source_info.set("origin", "top-level")?;
                entry.set("source_info", source_info)?;
                if let Some(completions) = command.get_argument_completions {
                    let source = command.source.clone();
                    entry.set(
                        "get_argument_completions",
                        lua.create_function(move |lua, prefix: String| {
                            let previous = current_source(lua);
                            set_current_source(lua, &source);
                            let outcome: mlua::Result<mlua::Value> = completions.call(prefix);
                            set_current_source(lua, &previous);
                            outcome
                        })?,
                    )?;
                }
                let source = command.source;
                let handler = command.handler;
                entry.set(
                    "handler",
                    lua.create_async_function(move |lua, (args, ctx): (String, mlua::Table)| {
                        let source = source.clone();
                        let handler = handler.clone();
                        async move {
                            let previous = current_source(&lua);
                            set_current_source(&lua, &source);
                            let outcome: mlua::Result<mlua::Value> =
                                handler.call_async((args, ctx)).await;
                            set_current_source(&lua, &previous);
                            outcome
                        }
                    })?,
                )?;
                result.push(entry)?;
            }
            Ok(result)
        })?,
    )?;
    // ExtensionAPI.getCommands. Prompt-template and skill rows join when their
    // Lua resource registries land; extension commands use the same resolved
    // invocation names and source snapshots as the product router.
    pi.set(
        "get_commands",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for command in resolved_commands(lua)? {
                if !command_is_public(lua, &command.source, &command.entry)? {
                    continue;
                }
                let entry = lua.create_table()?;
                entry.set("name", command.invocation_name)?;
                entry.set("description", command.description)?;
                entry.set("source", "extension")?;
                let source_info = lua.create_table()?;
                source_info.set("path", command.source.as_str())?;
                source_info.set("source", "cli")?;
                source_info.set("scope", "temporary")?;
                source_info.set("origin", "top-level")?;
                entry.set("sourceInfo", source_info)?;
                result.push(entry)?;
            }
            Ok(result)
        })?,
    )?;
    // Snapshot extension handlers for Lua-authored product fold policy. Each
    // wrapper restores source attribution around the async callback; Rust does
    // not interpret event results or choose stop/merge semantics.
    pi.set(
        "extension_handlers",
        lua.create_function(|lua, event: String| {
            let result = lua.create_table()?;
            for (source, handler) in event_handlers(lua, &event)? {
                let entry = lua.create_table()?;
                entry.set("source", source.as_str())?;
                let wrapper_source = source.clone();
                entry.set(
                    "handler",
                    lua.create_async_function(
                        move |lua, (event, ctx): (mlua::Table, mlua::Table)| {
                            let handler = handler.clone();
                            let source = wrapper_source.clone();
                            async move {
                                let previous = current_source(&lua);
                                set_current_source(&lua, &source);
                                let outcome: mlua::Result<mlua::Value> =
                                    handler.call_async((event, ctx)).await;
                                set_current_source(&lua, &previous);
                                outcome
                            }
                        },
                    )?,
                )?;
                result.push(entry)?;
            }
            Ok(result)
        })?,
    )?;
    pi.set(
        "validate_tool_arguments",
        lua.create_function(
            |lua, (name, schema, arguments): (String, mlua::Value, mlua::Value)| {
                let schema = crate::convert::lua_to_json(schema)?;
                let arguments = crate::convert::lua_to_json(arguments)?;
                let validated = crate::schema::validate_tool_arguments(&name, &schema, &arguments)
                    .map_err(|error| mlua::Error::runtime(error.to_string()))?;
                crate::convert::json_to_lua(lua, &validated)
            },
        )?,
    )?;

    // Thin CLI output mechanism; formatting and event policy remain Lua.
    pi.set(
        "output",
        lua.create_function(|_, text: String| {
            use std::io::Write as _;
            print!("{text}");
            std::io::stdout().flush().map_err(mlua::Error::external)
        })?,
    )?;

    // Package-level declaration defaults. Embedded and file-backed chunks
    // call the same function; source keys remain attribution only.
    pi.set(
        "declare_package",
        lua.create_function(|lua, options: mlua::Table| {
            let visibility = options
                .get::<Option<String>>("command_visibility")?
                .unwrap_or_else(|| "public".to_owned());
            if visibility != "public" && visibility != "internal" {
                return Err(mlua::Error::runtime(
                    "declare_package: command_visibility must be 'public' or 'internal'",
                ));
            }
            let entry = lua.create_table()?;
            entry.set("source", current_source(lua))?;
            entry.set(
                "scope",
                crate::kernel_api::scope_for_current_entry(lua)?.get(),
            )?;
            entry.set("command_visibility", visibility)?;
            let packages: mlua::Table = registry_table(lua)?.get("package_options")?;
            packages.push(entry)
        })?,
    )?;

    let on = lua.create_function(|lua, (event, handler): (String, mlua::Function)| {
        if event.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "on: event name must be a non-empty string",
            ));
        }
        let entry = lua.create_table()?;
        entry.set("event", event)?;
        entry.set("fn", handler)?;
        entry.set("bus", false)?;
        entry.set("active", true)?;
        let id = adapter_id(lua, "event")?;
        crate::kernel_api::register_adapter_declaration(lua, "event", &id, entry)
    })?;
    pi.set("on", on)?;

    // Shared inter-extension event bus (`pi.events`). Unlike lifecycle events,
    // channels are open strings, listeners see the same payload, and failures
    // are isolated. Snapshotting active listeners preserves EventEmitter's
    // add/remove-during-emit behavior.
    let events_api = lua.create_table()?;
    events_api.set(
        "on",
        lua.create_function(|lua, (channel, handler): (String, mlua::Function)| {
            let entry = lua.create_table()?;
            entry.set("event", channel)?;
            entry.set("fn", handler)?;
            entry.set("bus", true)?;
            entry.set("active", true)?;
            let id = adapter_id(lua, "bus")?;
            crate::kernel_api::register_adapter_declaration(lua, "event", &id, entry.clone())?;
            lua.create_function(move |_, ()| entry.set("active", false))
        })?,
    )?;
    events_api.set(
        "emit",
        lua.create_function(|lua, (channel, data): (String, mlua::Value)| {
            let mut snapshot = Vec::new();
            for entry in crate::kernel_api::declaration_entries(lua, "event")? {
                if entry.get::<Option<bool>>("bus")?.unwrap_or(false)
                    && entry.get::<String>("event")? == channel
                    && entry.get::<Option<bool>>("active")?.unwrap_or(true)
                {
                    snapshot.push((
                        entry.get::<String>("source")?,
                        entry.get::<mlua::Function>("fn")?,
                    ));
                }
            }
            for (source, handler) in snapshot {
                let previous = current_source(lua);
                set_current_source(lua, &source);
                let outcome = handler.call::<()>(data.clone());
                set_current_source(lua, &previous);
                if let Err(error) = outcome {
                    eprintln!("Event handler error ({channel}): {error}");
                }
            }
            Ok(())
        })?,
    )?;
    pi.set("events", events_api)?;

    // Spec `registerTool` (loader.ts): validate the mechanism-required
    // fields, then per-extension `Map.set` — re-registration of the same
    // name replaces the definition but keeps its position.
    let register_tool = lua.create_function(|lua, def: mlua::Table| {
        let name = def
            .get::<Option<String>>("name")?
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| {
                mlua::Error::runtime("register_tool: tool.name must be a non-empty string")
            })?;
        if def.get::<Option<mlua::Function>>("execute")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_tool: tool.execute must be a function",
            ));
        }
        crate::kernel_api::register_adapter_declaration(lua, "tool", &name, def)
    })?;
    pi.set("register_tool", register_tool)?;

    // Spec `registerCommand` (loader.ts): `{ name, ...options }` into the
    // per-extension map. Options are shallow-copied so later mutation of
    // the caller's table doesn't alias the registry.
    let register_command = lua.create_function(|lua, (name, options): (String, mlua::Table)| {
        if name.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_command: name must be a non-empty string",
            ));
        }
        if options.get::<Option<mlua::Function>>("handler")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_command: options.handler must be a function",
            ));
        }
        let entry = lua.create_table()?;
        for pair in options.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            entry.set(k, v)?;
        }
        entry.set("name", name.as_str())?;
        crate::kernel_api::register_adapter_declaration(lua, "command", &name, entry)
    })?;
    pi.set("register_command", register_command)?;

    // Public application/frontend declarations. Selection is by generic role
    // plus explicit active/priority data; extension load order and source-key
    // syntax never affect the winner.
    pi.set(
        "register_role",
        lua.create_function(|lua, definition: mlua::Table| {
            let id = definition
                .get::<Option<String>>("id")?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    mlua::Error::runtime("register_role: id must be a non-empty string")
                })?;
            let role = definition
                .get::<Option<String>>("role")?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    mlua::Error::runtime("register_role: role must be a non-empty string")
                })?;
            if definition.get::<Option<bool>>("active")?.is_none() {
                return Err(mlua::Error::runtime(
                    "register_role: active must be declared explicitly",
                ));
            }
            if definition.get::<Option<i64>>("priority")?.is_none() {
                return Err(mlua::Error::runtime(
                    "register_role: priority must be declared explicitly",
                ));
            }
            if definition
                .get::<Option<mlua::Function>>("handler")?
                .is_none()
            {
                return Err(mlua::Error::runtime(
                    "register_role: handler must be a function",
                ));
            }
            let entry = lua.create_table()?;
            for pair in definition.pairs::<mlua::Value, mlua::Value>() {
                let (key, value) = pair?;
                entry.set(key, value)?;
            }
            entry.set("id", id.as_str())?;
            entry.set("role", role)?;
            entry.set("dispatch", entry.get::<mlua::Function>("handler")?)?;
            crate::kernel_api::register_adapter_root(lua, "role", entry)
        })?,
    )?;
    pi.set(
        "registered_roles",
        lua.create_function(|lua, role: Option<String>| {
            let result = lua.create_table()?;
            for (_, _, declaration) in all_roles(lua)? {
                if role
                    .as_ref()
                    .is_none_or(|role| declaration.get::<String>("role").is_ok_and(|v| &v == role))
                {
                    result.push(declaration)?;
                }
            }
            Ok(result)
        })?,
    )?;

    // Spec `registerShortcut` (loader.ts): `{ shortcut, ...options }` into
    // the per-extension map, keyed by the lowercased key id. Conflict policy
    // against built-in keybindings stays with the frontend (runner.ts
    // getShortcuts); the host registry is mechanism only.
    let register_shortcut = lua.create_function(|lua, (key, options): (String, mlua::Table)| {
        if key.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_shortcut: shortcut must be a non-empty string",
            ));
        }
        if options.get::<Option<mlua::Function>>("handler")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_shortcut: options.handler must be a function",
            ));
        }
        let key = key.to_lowercase();
        let entry = lua.create_table()?;
        for pair in options.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            entry.set(k, v)?;
        }
        entry.set("shortcut", key.as_str())?;
        crate::kernel_api::register_adapter_declaration(lua, "keymap", &key, entry)
    })?;
    pi.set("register_shortcut", register_shortcut)?;

    // Resolved first-registration-wins view for the frontend (spec
    // runner.ts getShortcuts, minus the keybinding-conflict diagnostics
    // that land with user keybinding config).
    pi.set(
        "registered_shortcuts",
        lua.create_function(|lua, ()| {
            let mut seen = std::collections::HashSet::new();
            let result = lua.create_table()?;
            for (_, key, entry) in registrations(lua, "shortcuts", "shortcut_order")? {
                if seen.insert(key) {
                    result.push(entry)?;
                }
            }
            Ok(result)
        })?,
    )?;

    // Additive pi-rs composition surface: transcript renderers and shell slots
    // use one ordered declaration mechanism for embedded and file-backed Lua.
    // Definitions remain Lua functions; wrappers restore source attribution.
    let register_render_middleware =
        lua.create_function(|lua, (kind, options): (String, mlua::Table)| {
            if kind.trim().is_empty() {
                return Err(mlua::Error::runtime(
                    "register_render_middleware: kind must be a non-empty string",
                ));
            }
            if options.get::<Option<mlua::Function>>("render")?.is_none() {
                return Err(mlua::Error::runtime(
                    "register_render_middleware: options.render must be a function",
                ));
            }
            let name = options
                .get::<Option<String>>("name")?
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| kind.clone());
            let key = format!("{kind}\u{0}{name}");
            let entry = lua.create_table()?;
            entry.set("family", kind)?;
            entry.set("name", name)?;
            entry.set("order", options.get::<Option<i64>>("order")?.unwrap_or(0))?;
            entry.set("render", options.get::<mlua::Function>("render")?)?;
            crate::kernel_api::register_adapter_declaration(lua, "renderer", &key, entry)
        })?;
    pi.set("register_render_middleware", register_render_middleware)?;
    pi.set(
        "registered_render_middlewares",
        lua.create_function(|lua, kind: Option<String>| {
            let mut entries = registrations(lua, "render_middleware", "render_middleware_order")?
                .into_iter()
                .enumerate()
                .filter_map(|(index, (source, _, entry))| {
                    let entry_kind = entry.get::<String>("family").ok()?;
                    if kind
                        .as_ref()
                        .is_some_and(|kind| kind != &entry_kind && entry_kind != "*")
                    {
                        return None;
                    }
                    let order = entry
                        .get::<Option<i64>>("order")
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    Some((order, index, source, entry))
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|(order, index, _, _)| (*order, *index));
            let result = lua.create_table()?;
            for (order, _, source, entry) in entries {
                let value = lua.create_table()?;
                value.set("source", source.as_str())?;
                value.set("kind", entry.get::<String>("family")?)?;
                value.set("name", entry.get::<String>("name")?)?;
                value.set("order", order)?;
                let render = entry.get::<mlua::Function>("render")?;
                value.set(
                    "render",
                    lua.create_function(move |lua, args: mlua::MultiValue| {
                        let previous = current_source(lua);
                        set_current_source(lua, &source);
                        let outcome = render.call::<mlua::Value>(args);
                        set_current_source(lua, &previous);
                        outcome
                    })?,
                )?;
                result.push(value)?;
            }
            Ok(result)
        })?,
    )?;

    let register_ui_slot = lua.create_function(|lua, (slot, options): (String, mlua::Table)| {
        if slot.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_ui_slot: slot must be a non-empty string",
            ));
        }
        if options.get::<Option<mlua::Function>>("render")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_ui_slot: options.render must be a function",
            ));
        }
        let name = options
            .get::<Option<String>>("name")?
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| slot.clone());
        let key = format!("{slot}\u{0}{name}");
        let entry = lua.create_table()?;
        entry.set("slot", slot)?;
        entry.set("name", name)?;
        entry.set("order", options.get::<Option<i64>>("order")?.unwrap_or(0))?;
        entry.set("render", options.get::<mlua::Function>("render")?)?;
        crate::kernel_api::register_adapter_declaration(lua, "ui_slot", &key, entry)
    })?;
    pi.set("register_ui_slot", register_ui_slot)?;
    pi.set(
        "registered_ui_slots",
        lua.create_function(|lua, slot: Option<String>| {
            let mut entries = registrations(lua, "ui_slots", "ui_slot_order")?
                .into_iter()
                .enumerate()
                .filter_map(|(index, (source, _, entry))| {
                    let entry_slot = entry.get::<String>("slot").ok()?;
                    if slot
                        .as_ref()
                        .is_some_and(|slot| slot != &entry_slot && entry_slot != "*")
                    {
                        return None;
                    }
                    let order = entry
                        .get::<Option<i64>>("order")
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    Some((order, index, source, entry))
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|(order, index, _, _)| (*order, *index));
            let result = lua.create_table()?;
            for (order, _, source, entry) in entries {
                let value = lua.create_table()?;
                value.set("source", source.as_str())?;
                value.set("slot", entry.get::<String>("slot")?)?;
                value.set("name", entry.get::<String>("name")?)?;
                value.set("order", order)?;
                let render = entry.get::<mlua::Function>("render")?;
                value.set(
                    "render",
                    lua.create_function(move |lua, args: mlua::MultiValue| {
                        let previous = current_source(lua);
                        set_current_source(lua, &source);
                        let outcome = render.call::<mlua::Value>(args);
                        set_current_source(lua, &previous);
                        outcome
                    })?,
                )?;
                result.push(value)?;
            }
            Ok(result)
        })?,
    )?;

    // Spec `registerFlag`/`getFlag`: definitions stay per extension while
    // parsed/default values live in one shared runtime map. Defaults are
    // first-wins, matching `runtime.flagValues.has(name)`.
    let register_flag = lua.create_function(|lua, (name, options): (String, mlua::Table)| {
        if name.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_flag: name must be a non-empty string",
            ));
        }
        let flag_type: String = options.get("type")?;
        if flag_type != "boolean" && flag_type != "string" {
            return Err(mlua::Error::runtime(
                "register_flag: options.type must be 'boolean' or 'string'",
            ));
        }
        let source = current_source(lua);
        let entry = lua.create_table()?;
        for pair in options.pairs::<mlua::Value, mlua::Value>() {
            let (key, value) = pair?;
            entry.set(key, value)?;
        }
        entry.set("name", name.as_str())?;
        entry.set("extension_path", source.as_str())?;
        let default: mlua::Value = entry.get("default")?;
        crate::kernel_api::register_adapter_declaration(lua, "flag", &name, entry)?;
        if !default.is_nil() {
            let registry = registry_table(lua)?;
            let values: mlua::Table = registry.get("flag_values")?;
            if values.get::<Option<mlua::Value>>(name.as_str())?.is_none() {
                values.set(name.as_str(), default)?;
                let owners: mlua::Table = registry.get("flag_value_owners")?;
                owners.set(
                    name.as_str(),
                    crate::kernel_api::scope_for_current_entry(lua)?.get(),
                )?;
            }
        }
        Ok(())
    })?;
    pi.set("register_flag", register_flag)?;
    pi.set(
        "get_flag",
        lua.create_function(|lua, name: String| {
            let source = current_source(lua);
            if !registrations(lua, "flags", "flag_order")?
                .iter()
                .any(|(owner, registered, _)| owner == &source && registered == &name)
            {
                return Ok(mlua::Value::Nil);
            }
            let values: mlua::Table = registry_table(lua)?.get("flag_values")?;
            values.get(name.as_str())
        })?,
    )?;

    // Spec `registerProvider` (loader.ts / model-registry.ts): the
    // host-side half — store the config per extension, merging defined
    // keys over an existing registration of the same name (spec
    // `upsertRegisteredProvider`). Function values (`streamSimple`,
    // `oauth.*`) stay Lua-side, invocable when their mechanisms land
    // (WS2.5 auth, WS5 custom streams); the JSON mirror strips them.
    let register_provider = lua.create_function(|lua, (name, config): (String, mlua::Table)| {
        if name.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_provider: name must be a non-empty string",
            ));
        }
        let source = current_source(lua);
        let entry = lua.create_table()?;
        if let Some((_, _, previous)) = all_providers(lua)?
            .into_iter()
            .find(|(owner, registered, _)| owner == &source && registered == &name)
        {
            for pair in previous.pairs::<mlua::Value, mlua::Value>() {
                let (key, value) = pair?;
                if !matches!(key, mlua::Value::String(ref key) if key.to_str().is_ok_and(|key| matches!(key.as_ref(), "kind" | "id" | "source" | "scope" | "sequence" | "order"))) {
                    entry.set(key, value)?;
                }
            }
        }
        for pair in config.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            entry.set(k, v)?;
        }
        crate::kernel_api::register_adapter_declaration(lua, "provider", &name, entry)
    })?;
    pi.set("register_provider", register_provider)?;

    // Spec `unregisterProvider`: removal by name regardless of which
    // extension registered it (the spec's registry is keyed globally);
    // no effect if the name was never registered.
    let unregister_provider = lua.create_function(|lua, name: String| {
        let entry = lua.create_table()?;
        entry.set("removed", true)?;
        crate::kernel_api::register_adapter_declaration(lua, "provider", &name, entry)
    })?;
    pi.set("unregister_provider", unregister_provider)?;

    Ok(())
}
