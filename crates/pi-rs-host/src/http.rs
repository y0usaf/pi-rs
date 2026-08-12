//! HTTP mechanism for Lua-authored product policy.
//!
//! Endpoint choice, request timing, response interpretation, and presentation
//! stay in embedded/user Lua. This module only performs an awaitable request.
//!
//! Surface:
//! - `pi.http.get(url, options?)` — one-shot GET returning
//!   `{ status, ok, headers, body }` (body fully buffered as text).
//! - `pi.http.fetch(url, options?)` — one-shot fetch with method/body/headers,
//!   timeout, and abort signal; returns `{ status, ok, headers, body }`.
//! - `pi.http.stream(url, options?, on_chunk)` — abort-aware streaming GET:
//!   `on_chunk(chunk)` is called with each body chunk as a binary-safe Lua
//!   string as it arrives; resolves `{ status, ok, headers }` after the stream
//!   ends, or cancels the request when `options.signal` aborts. This is the
//!   mechanism behind Webfetch's download and Morph's gateway proxy: it
//!   streams rather than buffering, and aborts the in-flight HTTP request when
//!   the tool/session signal fires.

use futures_util::StreamExt;
use mlua::{Function, Lua, Table};
use pi_rs_ai::transport::AbortSignal;
use std::collections::HashMap;
use std::time::Duration;

/// Build a request from a url plus optional `headers`, `method`, `body`, and
/// `timeout_ms` options table.
fn build_request(
    client: &reqwest::Client,
    url: &str,
    options: Option<&Table>,
) -> mlua::Result<reqwest::RequestBuilder> {
    let default_method = reqwest::Method::GET;
    let mut method = default_method;
    if let Some(options) = &options
        && let Some(m) = options.get::<Option<String>>("method")?
    {
        method = reqwest::Method::from_bytes(m.as_bytes())
            .map_err(|e| mlua::Error::runtime(format!("invalid HTTP method {m:?}: {e}")))?;
    }
    let mut request = client.request(method, url);
    if let Some(options) = &options {
        if let Some(headers) = options.get::<Option<HashMap<String, String>>>("headers")? {
            for (name, value) in headers {
                request = request.header(name, value);
            }
        }
        if let Some(timeout_ms) = options.get::<Option<u64>>("timeout_ms")? {
            request = request.timeout(Duration::from_millis(timeout_ms));
        }
        if let Some(body) = options.get::<Option<mlua::String>>("body")? {
            request = request.body(body.as_bytes().to_vec());
        }
    }
    Ok(request)
}

fn response_to_lua(
    lua: &mlua::Lua,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: String,
) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.set("status", status.as_u16())?;
    result.set("ok", status.is_success())?;
    result.set(
        "headers",
        lua.create_table_from(
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_owned(), value.to_owned()))
                })
                .collect::<HashMap<_, _>>(),
        )?,
    )?;
    result.set("body", body)?;
    Ok(result)
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let http = lua.create_table()?;
    http.set(
        "get",
        lua.create_async_function(|lua, (url, options): (String, Option<Table>)| async move {
            let client = reqwest::Client::new();
            let request = build_request(&client, &url, options.as_ref())?;
            let response = request.send().await.map_err(mlua::Error::external)?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.text().await.map_err(mlua::Error::external)?;
            response_to_lua(&lua, status, &headers, body)
        })?,
    )?;

    http.set(
        "fetch",
        lua.create_async_function(|lua, (url, options): (String, Option<Table>)| async move {
            let client = reqwest::Client::new();
            let request = build_request(&client, &url, options.as_ref())?;
            let response = request.send().await.map_err(mlua::Error::external)?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.text().await.map_err(mlua::Error::external)?;
            response_to_lua(&lua, status, &headers, body)
        })?,
    )?;

    // Abort-aware streaming. `on_chunk` is called for each body chunk as it
    // arrives; the request is cancelled when `options.signal` aborts (either
    // before headers or mid-stream).
    http.set(
        "stream",
        lua.create_async_function(
            |lua, (url, options, on_chunk): (String, Option<Table>, Function)| async move {
                let mut signal: Option<AbortSignal> = None;
                let mut timeout_ms: Option<u64> = None;
                if let Some(options) = &options {
                    if let Some(ud) = options.get::<Option<mlua::AnyUserData>>("signal")? {
                        signal = Some(
                            ud.borrow::<crate::ai::LuaAbortSignal>()
                                .map_err(|_| {
                                    mlua::Error::runtime(
                                        "http.stream: signal must be an abort signal",
                                    )
                                })?
                                .0
                                .clone(),
                        );
                    }
                    timeout_ms = options.get::<Option<u64>>("timeout_ms")?;
                }
                let client = reqwest::Client::new();
                let request = build_request(&client, &url, options.as_ref())?;
                let response = request.send().await.map_err(mlua::Error::external)?;
                let status = response.status();
                let headers = response.headers().clone();
                let mut stream = response.bytes_stream();

                // Abort the stream when the signal fires (mid-stream).
                let abort_fut = async {
                    match &signal {
                        Some(signal) => signal.aborted().await,
                        None => std::future::pending().await,
                    }
                };
                tokio::pin!(abort_fut);

                let timeout_fut = async {
                    match timeout_ms {
                        Some(ms) if ms > 0 => tokio::time::sleep(Duration::from_millis(ms)).await,
                        _ => std::future::pending().await,
                    }
                };
                tokio::pin!(timeout_fut);

                'outer: loop {
                    tokio::select! {
                        _ = &mut abort_fut => {
                            break 'outer Err(mlua::Error::runtime("http.stream aborted"));
                        }
                        _ = &mut timeout_fut => {
                            break 'outer Err(mlua::Error::runtime("http.stream timed out"));
                        }
                        chunk = stream.next() => {
                            match chunk {
                                Some(Ok(bytes)) => {
                                    let s = lua.create_string(&bytes)?;
                                    on_chunk.call::<()>(s)?;
                                }
                                Some(Err(e)) => {
                                    break 'outer Err(mlua::Error::external(e));
                                }
                                None => break 'outer Ok(()),
                            }
                        }
                    }
                }?;

                let result_table = lua.create_table()?;
                result_table.set("status", status.as_u16())?;
                result_table.set("ok", status.is_success())?;
                result_table.set(
                    "headers",
                    lua.create_table_from(
                        headers
                            .iter()
                            .filter_map(|(name, value)| {
                                value
                                    .to_str()
                                    .ok()
                                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
                            })
                            .collect::<HashMap<_, _>>(),
                    )?,
                )?;
                Ok(result_table)
            },
        )?,
    )?;
    pi.set("http", http)?;
    Ok(())
}
