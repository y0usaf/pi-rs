//! Versioned model lookup, catalog inventory, and bounded provider-stream
//! bindings.

const DEFAULT_MAX_EVENTS: usize = 256;
const MAX_EVENTS: usize = 1_024;
const DEFAULT_MAX_MODELS: usize = 64;
const MAX_MODELS: usize = 512;

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let bridge = lua.create_table()?;
    crate::ai::install(lua, &bridge)?;
    let mechanisms: mlua::Table = bridge.get("ai")?;
    let stream: mlua::Function = mechanisms.get("stream_simple")?;

    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set("find", mechanisms.get::<mlua::Function>("find_model")?)?;
    v1.set(
        "providers",
        mechanisms.get::<mlua::Function>("list_providers")?,
    )?;
    let list_models: mlua::Function = mechanisms.get("list_models")?;
    v1.set(
        "catalog",
        lua.create_function(
            move |_, (provider, options): (String, Option<mlua::Table>)| {
                let offset = options
                    .as_ref()
                    .map(|options| options.get::<Option<usize>>("offset"))
                    .transpose()?
                    .flatten()
                    .unwrap_or(0);
                let limit = options
                    .as_ref()
                    .map(|options| options.get::<Option<usize>>("limit"))
                    .transpose()?
                    .flatten()
                    .unwrap_or(DEFAULT_MAX_MODELS);
                if !(1..=MAX_MODELS).contains(&limit) {
                    return Err(mlua::Error::runtime(format!(
                        "models.v1.catalog limit must be in 1..={MAX_MODELS}"
                    )));
                }
                list_models.call::<(mlua::Table, usize)>((provider, offset, limit))
            },
        )?,
    )?;
    v1.set("apis", mechanisms.get::<mlua::Function>("list_apis")?)?;
    v1.set(
        "validate",
        mechanisms.get::<mlua::Function>("validate_model")?,
    )?;
    v1.set("default_max_models", DEFAULT_MAX_MODELS)?;
    v1.set("max_models", MAX_MODELS)?;
    v1.set("default_max_events", DEFAULT_MAX_EVENTS)?;
    v1.set("max_events", MAX_EVENTS)?;
    v1.set(
        "stream",
        lua.create_async_function(
            move |lua,
                  (model, context, options, on_event): (
                mlua::Value,
                mlua::Value,
                Option<mlua::Table>,
                mlua::Function,
            )| {
                let stream = stream.clone();
                async move {
                    let max_events = options
                        .as_ref()
                        .map(|value| value.get::<Option<usize>>("max_events"))
                        .transpose()?
                        .flatten()
                        .unwrap_or(DEFAULT_MAX_EVENTS);
                    if !(1..=MAX_EVENTS).contains(&max_events) {
                        return Err(mlua::Error::runtime(format!(
                            "models.v1.stream max_events must be in 1..={MAX_EVENTS}"
                        )));
                    }
                    let seen = std::rc::Rc::new(std::cell::Cell::new(0_usize));
                    let callback_seen = std::rc::Rc::clone(&seen);
                    let callback = on_event.clone();
                    let bounded = lua.create_async_function(move |_, event: mlua::Value| {
                        let callback = callback.clone();
                        let seen = std::rc::Rc::clone(&callback_seen);
                        async move {
                            let next = seen.get().saturating_add(1);
                            if next > max_events {
                                return Err(mlua::Error::runtime(format!(
                                    "model stream exceeded {max_events} events"
                                )));
                            }
                            seen.set(next);
                            callback.call_async::<()>(event).await
                        }
                    })?;
                    stream
                        .call_async::<mlua::Value>((model, context, options, bounded))
                        .await
                }
            },
        )?,
    )?;

    let models = lua.create_table()?;
    models.set("v1", v1)?;
    pi.set("models", models)
}
