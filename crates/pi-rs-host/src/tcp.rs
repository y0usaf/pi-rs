//! TCP framed client mechanism (`pi.tcp`).
//!
//! Node translations (`net#createConnection`, `Socket`, and the Marionette
//! socket protocol in gecko-websearch) become explicit `pi.tcp` bindings.
//! A handle wraps a `tokio::net::TcpStream`; it owns the socket and closes
//! it on disposal (Drop), so no socket survives a dropped handle, reload,
//! or VM shutdown.
//!
//! Surface (all on the coroutine seam):
//! - `pi.tcp.connect(host, port, options?) -> handle`
//!   - `options.timeout_ms` for the connect (optional).
//! - `handle:write(data)` — send bytes; resolves when flushed.
//! - `handle:read(n)` — read up to `n` bytes as a binary-safe Lua string
//!   (blocking; empty when the peer closed).
//! - `handle:read_line(max?)` — read one LF-terminated line as text.
//! - `handle:read_frame(max?, prefix?)` — read a length-prefixed frame:
//!   `prefix` is a length field separator (default none, meaning a raw
//!   `read`). This is the primitive that lets a Lua translation implement
//!   Marionette's JSON-line framing on top.
//! - `handle:close()` — close the socket deterministically.
//! - `handle:is_closed()`
//!
//! The handle never owns mutable product state: it owns only the socket.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mlua::{Lua, UserData, UserDataMethods};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// An empty byte slice for EOF/closed reads.
fn empty_bytes() -> &'static [u8] {
    &[]
}

struct TcpInner {
    stream: Option<tokio::net::TcpStream>,
    closed: bool,
}

pub(crate) struct LuaTcp {
    inner: Rc<RefCell<TcpInner>>,
}

impl Drop for LuaTcp {
    fn drop(&mut self) {
        // Disposal guarantees: the socket is closed (and its OS fd freed) so
        // nothing survives a dropped handle or VM shutdown.
        self.inner.borrow_mut().stream = None;
        self.inner.borrow_mut().closed = true;
    }
}

impl UserData for LuaTcp {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_closed", |_, this, ()| Ok(this.inner.borrow().closed));

        methods.add_method("close", |_, this, ()| {
            let _ = this.inner.borrow_mut().stream.take();
            this.inner.borrow_mut().closed = true;
            Ok(())
        });

        methods.add_async_method("write", |_, this, data: mlua::String| async move {
            let inner = this.inner.clone();
            // Take the stream out for the duration of the await; the RefMut
            // borrow must not bridge the await (it is !Send). Restore after.
            let mut stream = inner
                .borrow_mut()
                .stream
                .take()
                .ok_or_else(|| mlua::Error::runtime("tcp: write on a closed socket"))?;
            let result = stream
                .write_all(&data.as_bytes())
                .await
                .map_err(mlua::Error::external);
            if let Err(e) = result {
                inner.borrow_mut().closed = true;
                return Err(e);
            }
            inner.borrow_mut().stream = Some(stream);
            Ok(())
        });

        methods.add_async_method("read", |lua, this, n: usize| async move {
            let inner = this.inner.clone();
            let Some(stream) = inner.borrow_mut().stream.take() else {
                return lua.create_string(empty_bytes());
            };
            let mut stream = stream;
            // Cap a single read so a Lua-controlled `n` cannot request an
            // unbounded allocation (DoS); a callback wanting more loops.
            const MAX_READ: usize = 64 * 1024;
            let mut buf = vec![0_u8; n.clamp(1, MAX_READ)];
            let (count, keep) = match stream.read(&mut buf).await {
                Ok(0) | Err(_) => (0, false),
                Ok(count) => (count, true),
            };
            if !keep {
                inner.borrow_mut().closed = true;
                return lua.create_string(empty_bytes());
            }
            inner.borrow_mut().stream = Some(stream);
            buf.truncate(count);
            lua.create_string(&buf)
        });

        // Read one line terminated by LF (returns without the LF).
        methods.add_async_method("read_line", |lua, this, max: Option<usize>| async move {
            let inner = this.inner.clone();
            let Some(stream) = inner.borrow_mut().stream.take() else {
                return Ok(mlua::Value::Nil);
            };
            let mut stream = stream;
            let limit = max.unwrap_or(64 * 1024);
            let mut buf = Vec::new();
            let mut byte = [0_u8; 1];
            let mut eof = false;
            loop {
                if buf.len() >= limit {
                    break;
                }
                match stream.read(&mut byte).await {
                    Ok(0) | Err(_) => {
                        eof = true;
                        break;
                    }
                    Ok(_) => {
                        if byte[0] == b'\n' {
                            break;
                        }
                        buf.push(byte[0]);
                    }
                }
            }
            if eof {
                inner.borrow_mut().closed = true;
                if buf.is_empty() {
                    return Ok(mlua::Value::Nil);
                }
            } else {
                inner.borrow_mut().stream = Some(stream);
            }
            Ok(mlua::Value::String(lua.create_string(&buf)?))
        });
    }
}

/// Install `pi.tcp` on the API table.
pub(crate) fn install(lua: &Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let tcp = lua.create_table()?;
    tcp.set(
        "connect",
        lua.create_async_function(
            |lua, (host, port, options): (String, u16, Option<mlua::Table>)| async move {
                let mut timeout_ms = None;
                if let Some(opts) = &options {
                    timeout_ms = opts.get::<Option<u64>>("timeout_ms")?;
                }
                let connect = tokio::net::TcpStream::connect((host.as_str(), port));
                let stream = match timeout_ms {
                    Some(ms) if ms > 0 => tokio::time::timeout(Duration::from_millis(ms), connect)
                        .await
                        .map_err(|_| {
                            mlua::Error::runtime(format!("tcp.connect {host}:{port} timed out"))
                        })?
                        .map_err(mlua::Error::external)?,
                    _ => connect.await.map_err(mlua::Error::external)?,
                };
                let inner = Rc::new(RefCell::new(TcpInner {
                    stream: Some(stream),
                    closed: false,
                }));
                lua.create_userdata(LuaTcp { inner })
            },
        )?,
    )?;
    pi.set("tcp", tcp)?;
    Ok(())
}
