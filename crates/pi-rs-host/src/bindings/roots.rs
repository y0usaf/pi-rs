//! Focused versioned root/action facade over the canonical kernel transaction.

pub(crate) fn install(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    control: std::sync::Arc<crate::kernel::Control>,
    config: crate::HostConfig,
) -> mlua::Result<()> {
    let kernel: mlua::Table = pi.get("kernel")?;
    let kernel_v1: mlua::Table = kernel.get("v1")?;
    let register: mlua::Function = kernel_v1.get("root")?;

    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set(
        "register",
        lua.create_function(move |_, definition: mlua::Table| {
            let kind: String = definition.get("kind")?;
            if !matches!(kind.as_str(), "application" | "agent" | "frontend") {
                return Err(mlua::Error::runtime(
                    "roots.v1 supports application, agent, and frontend roots",
                ));
            }
            register.call::<()>(definition)
        })?,
    )?;
    v1.set("action", kernel_v1.get::<mlua::Function>("action")?)?;
    v1.set(
        "cancellation",
        kernel_v1.get::<mlua::Function>("cancellation")?,
    )?;
    v1.set("module", kernel_v1.get::<mlua::Table>("module")?)?;

    // Cross-root coordination: dispatch another root kind from inside an
    // active dispatch and receive its settled batch as data. The caller keeps
    // its own transaction; nothing publishes implicitly. The nested dispatch
    // shares the caller's runtime and watchdog budget, is depth-capped, and
    // rejects direct recursion into a root kind already on the nest stack.
    let dispatch_control = control;
    let dispatch_config = config;
    v1.set(
        "dispatch",
        lua.create_async_function(
            move |lua, (kind, event, context): (String, mlua::Value, Option<mlua::Value>)| {
                let control = std::sync::Arc::clone(&dispatch_control);
                let config = dispatch_config.clone();
                async move {
                    let kind =
                        crate::kernel::RootKind::parse(&kind).map_err(mlua::Error::external)?;
                    if !matches!(
                        kind,
                        crate::kernel::RootKind::Application
                            | crate::kernel::RootKind::Agent
                            | crate::kernel::RootKind::Frontend
                    ) {
                        return Err(mlua::Error::runtime(
                            "roots.v1.dispatch supports application, agent, and frontend roots",
                        ));
                    }
                    let event = crate::convert::lua_to_json_strict(event)?;
                    let context = match context {
                        Some(value) => crate::convert::lua_to_json_strict(value)?,
                        None => serde_json::Value::Null,
                    };
                    let batch =
                        crate::vm::dispatch_nested(&lua, &config, &control, kind, event, context)
                            .await
                            .map_err(mlua::Error::external)?;
                    // Batches cross back as ordinary mutable tables: they are
                    // data the caller may republish through roots.action.
                    let actions = lua.create_table()?;
                    for action in &batch.actions {
                        let entry = lua.create_table()?;
                        entry.set("kind", action.kind.as_str())?;
                        entry.set(
                            "payload",
                            crate::convert::json_to_lua(&lua, &action.payload)?,
                        )?;
                        actions.push(entry)?;
                    }
                    let effects = lua.create_table()?;
                    for effect in &batch.effects {
                        let entry = lua.create_table()?;
                        entry.set("kind", effect.kind.as_str())?;
                        entry.set(
                            "payload",
                            crate::convert::json_to_lua(&lua, &effect.payload)?,
                        )?;
                        effects.push(entry)?;
                    }
                    let result = lua.create_table()?;
                    result.set("generation", batch.generation.get())?;
                    result.set("source", batch.source)?;
                    result.set("actions", actions)?;
                    result.set("effects", effects)?;
                    Ok(result)
                }
            },
        )?,
    )?;

    let roots = lua.create_table()?;
    roots.set("v1", v1)?;
    pi.set("roots", roots)
}
