//! Public HTTP adapters over the bounded scope-owned HTTP stream effect.

use std::collections::HashMap;
use std::time::Duration;

use mlua::{Lua, Table, UserData, UserDataMethods};

use crate::effects::{
    EffectOptions, EffectRequest, EffectResult, EffectTimeout, HttpRequest, HttpResponse,
};

struct CollectedResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct LuaHttpStream(tokio::sync::Mutex<crate::effects::HttpStream>);

impl UserData for LuaHttpStream {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("next_chunk", |lua, this, ()| async move {
            match this.0.lock().await.next().await {
                Some(Ok(bytes)) => lua.create_string(&bytes).map(mlua::Value::String),
                Some(Err(error)) => Err(crate::effects::lua_error(error)),
                None => Ok(mlua::Value::Nil),
            }
        });
    }
}

fn parse_options(options: Option<Table>) -> mlua::Result<(HashMap<String, String>, EffectOptions)> {
    let mut headers = HashMap::new();
    let mut effect_options = EffectOptions {
        timeout: EffectTimeout::After(Duration::from_secs(30)),
        stream_capacity: crate::effects::DEFAULT_STREAM_CAPACITY,
        max_output_bytes: crate::effects::DEFAULT_MAX_OUTPUT_BYTES,
    };
    if let Some(options) = options {
        headers = options
            .get::<Option<HashMap<String, String>>>("headers")?
            .unwrap_or_default();
        if let Some(timeout_ms) = options.get::<Option<u64>>("timeout_ms")? {
            effect_options.timeout = EffectTimeout::After(Duration::from_millis(timeout_ms));
        }
        if let Some(capacity) = options.get::<Option<usize>>("stream_capacity")? {
            effect_options.stream_capacity = capacity;
        }
        if let Some(limit) = options.get::<Option<usize>>("max_body_bytes")? {
            effect_options.max_output_bytes = limit;
        }
    }
    Ok((headers, effect_options))
}

async fn start_request(
    hub: &crate::effects::EffectHub,
    scope: crate::kernel::ScopeId,
    url: String,
    options: Option<Table>,
) -> mlua::Result<HttpResponse> {
    let (headers, options) = parse_options(options)?;
    let result = hub
        .request(
            scope,
            EffectRequest::Http(HttpRequest {
                url,
                headers,
                options,
            }),
            crate::effects::cancellation(),
        )
        .await
        .map_err(crate::effects::lua_error)?;
    match result {
        EffectResult::Http(response) => Ok(response),
        _ => Err(mlua::Error::runtime(
            "HTTP effect returned the wrong result",
        )),
    }
}

async fn request(
    hub: &crate::effects::EffectHub,
    scope: crate::kernel::ScopeId,
    url: String,
    options: Option<Table>,
    on_chunk: Option<mlua::Function>,
) -> mlua::Result<CollectedResponse> {
    let mut response = start_request(hub, scope, url, options).await?;
    let mut body = Vec::new();
    while let Some(chunk) = response.stream.next().await {
        let chunk = chunk.map_err(crate::effects::lua_error)?;
        if let Some(callback) = &on_chunk {
            callback
                .call_async::<()>(mlua::String::wrap(&chunk))
                .await?;
        }
        body.extend(chunk);
    }
    Ok(CollectedResponse {
        status: response.status,
        headers: response.headers,
        body,
    })
}

fn response_table(lua: &Lua, response: CollectedResponse) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.set("status", response.status)?;
    result.set("ok", (200..300).contains(&response.status))?;
    result.set("headers", response.headers)?;
    result.set("body", String::from_utf8_lossy(&response.body).into_owned())?;
    result.set("bytes", lua.create_string(&response.body)?)?;
    Ok(result)
}

fn stream_table(lua: &Lua, response: HttpResponse) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.set("status", response.status)?;
    result.set("ok", (200..300).contains(&response.status))?;
    result.set("headers", response.headers)?;
    result.set(
        "stream",
        lua.create_userdata(LuaHttpStream(tokio::sync::Mutex::new(response.stream)))?,
    )?;
    Ok(result)
}

pub(crate) fn install(lua: &Lua, pi: &Table, hub: crate::effects::EffectHub) -> mlua::Result<()> {
    let http = lua.create_table()?;
    let get_hub = hub.clone();
    http.set(
        "get",
        lua.create_async_function(move |lua, (url, options): (String, Option<Table>)| {
            let hub = get_hub.clone();
            async move {
                let scope = hub.scope(&lua)?;
                let response = request(&hub, scope, url, options, None).await?;
                response_table(&lua, response)
            }
        })?,
    )?;
    let stream_hub = hub.clone();
    http.set(
        "stream",
        lua.create_async_function(
            move |lua, (url, options, on_chunk): (String, Option<Table>, mlua::Function)| {
                let hub = stream_hub.clone();
                async move {
                    let scope = hub.scope(&lua)?;
                    let response = request(&hub, scope, url, options, Some(on_chunk)).await?;
                    response_table(&lua, response)
                }
            },
        )?,
    )?;
    http.set(
        "open",
        lua.create_async_function(move |lua, (url, options): (String, Option<Table>)| {
            let hub = hub.clone();
            async move {
                let scope = hub.scope(&lua)?;
                let response = start_request(&hub, scope, url, options).await?;
                stream_table(&lua, response)
            }
        })?,
    )?;
    pi.set("http", http)
}
