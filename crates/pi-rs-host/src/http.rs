//! Minimal HTTP mechanism for Lua-authored product policy.
//!
//! Endpoint choice, request timing, response interpretation, and presentation
//! stay in embedded/user Lua. This module only performs an awaitable request.

use mlua::{Lua, Table};
use std::{collections::HashMap, time::Duration};

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let http = lua.create_table()?;
    http.set(
        "get",
        lua.create_async_function(|lua, (url, options): (String, Option<Table>)| async move {
            let mut request = reqwest::Client::new().get(url);
            if let Some(options) = options {
                if let Some(headers) = options.get::<Option<HashMap<String, String>>>("headers")? {
                    for (name, value) in headers {
                        request = request.header(name, value);
                    }
                }
                if let Some(timeout_ms) = options.get::<Option<u64>>("timeout_ms")? {
                    request = request.timeout(Duration::from_millis(timeout_ms));
                }
            }

            let response = request.send().await.map_err(mlua::Error::external)?;
            let status = response.status();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_owned(), value.to_owned()))
                })
                .collect::<HashMap<_, _>>();
            let body = response.text().await.map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            result.set("status", status.as_u16())?;
            result.set("ok", status.is_success())?;
            result.set("headers", headers)?;
            result.set("body", body)?;
            Ok(result)
        })?,
    )?;
    pi.set("http", http)
}
