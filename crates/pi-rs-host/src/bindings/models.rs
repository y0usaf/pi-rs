//! Versioned model lookup and bounded provider-stream bindings.

const DEFAULT_MAX_EVENTS: usize = 256;
const MAX_EVENTS: usize = 1_024;

pub(crate) fn install(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    storage: crate::auth::SharedStorage,
) -> mlua::Result<()> {
    let bridge = lua.create_table()?;
    crate::ai::install(lua, &bridge, storage)?;
    let mechanisms: mlua::Table = bridge.get("ai")?;
    let stream: mlua::Function = mechanisms.get("stream_simple")?;

    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set("find", mechanisms.get::<mlua::Function>("find_model")?)?;
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
