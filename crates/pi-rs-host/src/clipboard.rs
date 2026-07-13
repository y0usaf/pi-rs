//! Public clipboard adapters over the scope-owned clipboard effect.

use std::collections::HashMap;
use std::time::Duration;

use mlua::{Lua, Table};

use crate::effects::{ClipboardRequest, CryptoRequest, EffectOptions, EffectRequest, EffectResult};

type Env = HashMap<String, String>;

fn node_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        "linux" => "linux",
        _ => "other",
    }
}

fn options(options: Option<Table>) -> mlua::Result<(Env, String)> {
    match options {
        Some(options) => {
            let env = match options.get::<Option<Table>>("env")? {
                Some(table) => {
                    let mut env = Env::new();
                    for pair in table.pairs::<String, String>() {
                        let (key, value) = pair?;
                        env.insert(key, value);
                    }
                    env
                }
                None => std::env::vars().collect(),
            };
            let platform = options
                .get::<Option<String>>("platform")?
                .unwrap_or_else(|| node_platform().to_owned());
            Ok((env, platform))
        }
        None => Ok((std::env::vars().collect(), node_platform().to_owned())),
    }
}

pub(crate) fn install(lua: &Lua, pi: &Table, hub: crate::effects::EffectHub) -> mlua::Result<()> {
    let clipboard = lua.create_table()?;
    let read_hub = hub.clone();
    clipboard.set(
        "read_image",
        lua.create_async_function(move |lua, value: Option<Table>| {
            let hub = read_hub.clone();
            async move {
                let (env, platform) = options(value)?;
                let scope = hub.scope(&lua)?;
                let result = hub
                    .request(
                        scope,
                        EffectRequest::Clipboard(
                            ClipboardRequest::ReadImage { env, platform },
                            EffectOptions::bounded(Duration::from_secs(15)),
                        ),
                        crate::effects::cancellation(),
                    )
                    .await
                    .map_err(crate::effects::lua_error)?;
                match result {
                    EffectResult::Clipboard(crate::effects::ClipboardResult::Image(None)) => {
                        Ok(mlua::Value::Nil)
                    }
                    EffectResult::Clipboard(crate::effects::ClipboardResult::Image(Some(
                        image,
                    ))) => {
                        let result = lua.create_table()?;
                        result.set("bytes", lua.create_string(&image.bytes)?)?;
                        result.set("mimeType", image.mime_type)?;
                        Ok(mlua::Value::Table(result))
                    }
                    _ => Err(mlua::Error::runtime(
                        "clipboard effect returned the wrong result",
                    )),
                }
            }
        })?,
    )?;
    clipboard.set(
        "extension_for_mime_type",
        lua.create_function(|_, mime_type: String| {
            Ok(crate::effects::extension_for_image_mime_type(&mime_type).map(str::to_owned))
        })?,
    )?;
    let write_hub = hub.clone();
    clipboard.set(
        "write_text",
        lua.create_async_function(move |lua, (text, value): (String, Option<Table>)| {
            let hub = write_hub.clone();
            async move {
                let (env, platform) = options(value)?;
                let scope = hub.scope(&lua)?;
                let result = hub
                    .request(
                        scope,
                        EffectRequest::Clipboard(
                            ClipboardRequest::WriteText {
                                text,
                                env,
                                platform,
                            },
                            EffectOptions::bounded(Duration::from_secs(15)),
                        ),
                        crate::effects::cancellation(),
                    )
                    .await
                    .map_err(crate::effects::lua_error)?;
                match result {
                    EffectResult::Clipboard(crate::effects::ClipboardResult::Unit) => Ok(()),
                    _ => Err(mlua::Error::runtime(
                        "clipboard effect returned the wrong result",
                    )),
                }
            }
        })?,
    )?;
    pi.set("clipboard", clipboard)?;

    let uuid_hub = hub.clone();
    pi.set(
        "random_uuid",
        lua.create_async_function(move |lua, ()| {
            let hub = uuid_hub.clone();
            async move {
                let scope = hub.scope(&lua)?;
                match hub
                    .request(
                        scope,
                        EffectRequest::Crypto(
                            CryptoRequest::RandomUuid,
                            EffectOptions::bounded(Duration::from_secs(5)),
                        ),
                        crate::effects::cancellation(),
                    )
                    .await
                    .map_err(crate::effects::lua_error)?
                {
                    EffectResult::Uuid(value) => Ok(value),
                    _ => Err(mlua::Error::runtime(
                        "UUID effect returned the wrong result",
                    )),
                }
            }
        })?,
    )?;
    Ok(())
}
