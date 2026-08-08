//! pi.net — Lua-native TCP socket client and URL parsing (PLAN 9.9
//! socket.Socket / socket.createConnection, node_ambient URL).
//!
//! Sockets are opaque handles owning a tokio TcpStream; every handle is a
//! tracked resource (resource.tcp_socket) so dispose_all / VM shutdown
//! cannot leak a connection. Read methods take an optional timeout_ms so
//! a silent peer cannot hang a coroutine forever.
//!
//! Lua surface (pi.net):
//! - create_connection(host, port, options?) -> socket (async)
//!   options: { timeout_ms = connect timeout }
//!   socket methods: write(data) async, read_some(timeout_ms?) async,
//!   read_line(max_bytes, timeout_ms?) async, read_exact(n, timeout_ms?)
//!   async, close(), peer()
//! - pi.url.parse(url, base?) -> { protocol, host, hostname, port,
//!   pathname, search, hash, href }

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mlua::{Lua, Table, UserData, UserDataMethods};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct SocketState {
    stream: Option<TcpStream>,
}

struct LuaSocket {
    state: Arc<Mutex<SocketState>>,
    peer: String,
}

#[allow(clippy::await_holding_lock)] // per-socket serialization: the state lock must cover the async read/write
impl UserData for LuaSocket {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method_mut("write", |_, this, data: mlua::String| async move {
            let mut stream = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(stream) = stream.stream.as_mut() else {
                return Err(mlua::Error::runtime("socket closed"));
            };
            stream
                .write_all(data.as_bytes().as_ref())
                .await
                .map_err(mlua::Error::external)?;
            Ok(())
        });
        methods.add_async_method_mut("read_some", |lua, this, timeout_ms: Option<u64>| async move {
            let mut stream = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(stream) = stream.stream.as_mut() else {
                return Ok(mlua::Value::Nil);
            };
            let mut buf = [0u8; 4096];
            let read = read_with_timeout(stream, &mut buf, timeout_ms).await?;
            match read {
                Some(0) => Ok(mlua::Value::Nil),
                Some(n) => Ok(mlua::Value::String(lua.create_string(&buf[..n])?)),
                None => Ok(mlua::Value::Nil),
            }
        });
        methods.add_async_method_mut(
            "read_line",
            |lua, this, (max_bytes, timeout_ms): (usize, Option<u64>)| async move {
                let mut stream = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(stream) = stream.stream.as_mut() else {
                    return Ok(mlua::Value::Nil);
                };
                let mut line = Vec::new();
                let mut byte = [0u8; 1];
                let deadline = timeout_ms.map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));
                loop {
                    if line.len() >= max_bytes {
                        break;
                    }
                    let read = match deadline {
                        Some(d) => match tokio::time::timeout_at(d, stream.read(&mut byte)).await {
                            Ok(r) => r.map_err(mlua::Error::external)?,
                            Err(_) => return Err(mlua::Error::runtime("read_line timeout")),
                        },
                        None => stream.read(&mut byte).await.map_err(mlua::Error::external)?,
                    };
                    match read {
                        0 => {
                            if line.is_empty() {
                                return Ok(mlua::Value::Nil);
                            }
                            break;
                        }
                        _ => {
                            if byte[0] == b'\n' {
                                break;
                            }
                            if byte[0] != b'\r' {
                                line.push(byte[0]);
                            }
                        }
                    }
                }
                Ok(mlua::Value::String(lua.create_string(&line)?))
            },
        );
        methods.add_async_method_mut(
            "read_exact",
            |lua, this, (n, timeout_ms): (usize, Option<u64>)| async move {
                let mut stream = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(stream) = stream.stream.as_mut() else {
                    return Ok(mlua::Value::Nil);
                };
                let mut buf = vec![0u8; n];
                let deadline = timeout_ms.map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));
                let mut filled = 0usize;
                while filled < n {
                    let read = match deadline {
                        Some(d) => match tokio::time::timeout_at(d, stream.read(&mut buf[filled..])).await {
                            Ok(r) => r.map_err(mlua::Error::external)?,
                            Err(_) => return Err(mlua::Error::runtime("read_exact timeout")),
                        },
                        None => stream.read(&mut buf[filled..]).await.map_err(mlua::Error::external)?,
                    };
                    if read == 0 {
                        break;
                    }
                    filled += read;
                }
                Ok(mlua::Value::String(lua.create_string(&buf[..filled])?))
            },
        );
        methods.add_method_mut("close", |_, this, ()| {
            let mut state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.stream.take();
            Ok(())
        });
        methods.add_method("peer", |_, this, ()| Ok(this.peer.clone()));
    }
}

async fn read_with_timeout(
    stream: &mut TcpStream,
    buf: &mut [u8],
    timeout_ms: Option<u64>,
) -> mlua::Result<Option<usize>> {
    match timeout_ms {
        Some(ms) => match tokio::time::timeout(Duration::from_millis(ms), stream.read(buf)).await {
            Ok(read) => read.map(Some).map_err(mlua::Error::external),
            Err(_) => Err(mlua::Error::runtime("socket read timeout")),
        },
        None => stream.read(buf).await.map(Some).map_err(mlua::Error::external),
    }
}

fn install_net(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let net = lua.create_table()?;
    net.set(
        "create_connection",
        lua.create_async_function(
            |lua, (host, port, options): (String, u16, Option<Table>)| async move {
                let timeout_ms = match &options {
                    Some(options) => options.get::<Option<u64>>("timeout_ms")?,
                    None => None,
                };
                let connect = TcpStream::connect((host.as_str(), port));
                let stream = match timeout_ms {
                    Some(ms) => match tokio::time::timeout(Duration::from_millis(ms), connect).await {
                        Ok(result) => result.map_err(mlua::Error::external)?,
                        Err(_) => {
                            return Err(mlua::Error::runtime(format!(
                                "create_connection {host}:{port} timed out"
                            )));
                        }
                    },
                    None => connect.await.map_err(mlua::Error::external)?,
                };
                let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
                let socket = LuaSocket {
                    state: Arc::new(Mutex::new(SocketState { stream: Some(stream) })),
                    peer,
                };
                let label = format!("tcp:{host}:{port}");
                let state = Arc::clone(&socket.state);
                crate::resources::register("resource.tcp_socket", label, move || {
                    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.stream.take();
                });
                Ok(mlua::Value::UserData(lua.create_userdata(socket)?))
            },
        )?,
    )?;
    pi.set("net", net)?;
    Ok(())
}

fn install_url(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let url = lua.create_table()?;
    url.set(
        "parse",
        lua.create_function(|lua, (input, base): (String, Option<String>)| {
            let parsed = match &base {
                Some(base) => url::Url::parse(base)
                    .map_err(|e| mlua::Error::runtime(format!("url.parse: {e}")))?
                    .join(&input)
                    .map_err(|e| mlua::Error::runtime(format!("url.parse: {e}")))?,
                None => url::Url::parse(&input)
                    .map_err(|e| mlua::Error::runtime(format!("url.parse: {e}")))?,
            };
            let out = lua.create_table()?;
            out.set("protocol", parsed.scheme().to_owned() + ":")?;
            out.set("host", parsed.host_str().unwrap_or("").to_owned())?;
            out.set("hostname", parsed.host_str().unwrap_or("").to_owned())?;
            out.set("port", parsed.port().map(|p| p.to_string()).unwrap_or_default())?;
            out.set("pathname", parsed.path().to_owned())?;
            out.set("search", parsed.query().map(|q| format!("?{q}")).unwrap_or_default())?;
            out.set("hash", parsed.fragment().map(|f| format!("#{f}")).unwrap_or_default())?;
            out.set("href", parsed.as_str().to_owned())?;
            Ok(out)
        })?,
    )?;
    pi.set("url", url)?;
    Ok(())
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    install_net(lua, pi)?;
    install_url(lua, pi)?;
    Ok(())
}
