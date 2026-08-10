//! pi.repl — persistent Python/IPython kernel bridge (P1 tier-2 binding).
//!
//! The Lua surface over crates/pi-rs-repl. The kernel is a managed resource
//! (disposed at VM shutdown). host_request bridging follows the host's own
//! coroutine model: the Rust reader task sends the request into a channel
//! and blocks for the reply; Lua policy runs a background pump (pi.spawn)
//! that receives requests and replies through the same channel.
//!
//! Lua surface:
//! - repl.spawn({python?, cwd?, env?, watchdog_ms?, interrupt_grace_ms?})
//!   async -> (kernel, requests)   requests is the host_request channel
//! - kernel:execute(code, {max_chars?}?) async -> ExecuteResult table
//! - kernel:interrupt() async
//! - kernel:snapshot(path, manifest_path, max_bytes?) async -> table
//! - kernel:restore(path) async -> table
//! - kernel:shutdown() async
//! - kernel:is_dead() -> bool
//! - requests:receive() async -> request   (blocks until a host_request)
//! - request:get_kind() -> string
//! - request:get_payload() -> table
//! - request:reply(value)                  (send the reply back to the cell)

use std::path::PathBuf;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};
use pi_rs_repl::{KernelConfig, KernelManager};
use tokio::sync::{mpsc, oneshot};

use crate::convert::{json_to_lua, lua_to_json};



pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let repl = lua.create_table()?;
    repl.set("spawn", lua.create_async_function(|lua, config: Table| async move {
        spawn_inner(&lua, config).await
    })?)?;
    pi.set("repl", repl)
}

async fn spawn_inner(_lua: &Lua, config: Table) -> mlua::Result<(KernelUserData, HostRequestRx)> {
    let python: String = config
        .get("python")
        .unwrap_or_else(|_| std::env::var("PI_RS_REPL_PYTHON").unwrap_or_else(|_| "python3".to_string()));
    let cwd: Option<String> = config.get("cwd").ok();
    let watchdog_ms: u64 = config.get("watchdog_ms").unwrap_or(300_000);
    let interrupt_grace_ms: u64 = config.get("interrupt_grace_ms").unwrap_or(1_000);

    let env: Vec<(String, String)> = if let Ok(env_table) = config.get::<Table>("env") {
        let mut out = Vec::new();
        for pair in env_table.pairs::<String, String>() {
            let (k, v) = pair.map_err(mlua::Error::external)?;
            out.push((k, v));
        }
        out
    } else {
        Vec::new()
    };

    // The host_request outbox: the kernel's reader task delivers requests
    // here and awaits the reply (never blocks a runtime thread); Lua policy
    // pumps the receiver side with a pi.spawn coroutine.
    let (tx, rx) = mpsc::unbounded_channel::<pi_rs_repl::HostRequestMsg>();

    let cfg = KernelConfig {
        python: PathBuf::from(python),
        cwd: cwd.map(PathBuf::from),
        env,
        watchdog_ms,
        interrupt_grace_ms,
        host_outbox: Some(tx),
        on_stream: None,
    };
    let kernel = KernelManager::spawn(cfg).await.map_err(mlua::Error::external)?;

    // Managed resource: the kernel dies with its owner (VM shutdown).
    // Resource disposal runs on the VM thread after the dispatch loop exits,
    // outside any Tokio reactor context, so `tokio::spawn` is not usable
    // here (no reactor -> panic). A fresh current-thread runtime makes the
    // shutdown reactor-independent; it is a no-op when the kernel is already
    // dead (an explicit `kernel:shutdown()` or a prior dispose).
    let resource_kernel = kernel.clone();
    crate::resources::register("kernel", "pi.repl".to_string(), move || {
        let k = resource_kernel.clone();
        if k.is_dead() {
            return;
        }
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            rt.block_on(k.shutdown());
        }
    });

    Ok((KernelUserData(kernel), HostRequestRx(rx)))
}

/// kernel userdata: one per spawned kernel.
struct KernelUserData(KernelManager);

impl UserData for KernelUserData {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method(
            "execute",
            |lua, this, (code, opts): (String, Option<Table>)| async move {
                let max_chars: Option<u64> = opts
                    .and_then(|o| o.get("max_chars").ok())
                    .unwrap_or(None);
                let result = this
                    .0
                    .execute(&code, max_chars)
                    .await
                    .map_err(mlua::Error::external)?;
                json_to_lua(&lua, &serde_json::to_value(result).map_err(mlua::Error::external)?)
            },
        );
        methods.add_async_method("interrupt", |_, this, ()| async move {
            this.0.interrupt().await.map_err(mlua::Error::external)?;
            Ok(())
        });
        methods.add_async_method(
            "snapshot",
            |lua, this, (path, manifest, max): (String, String, Option<u64>)| async move {
                let result = this
                    .0
                    .snapshot(
                        std::path::Path::new(&path),
                        std::path::Path::new(&manifest),
                        max,
                    )
                    .await
                    .map_err(mlua::Error::external)?;
                json_to_lua(&lua, &serde_json::to_value(result).map_err(mlua::Error::external)?)
            },
        );
        methods.add_async_method(
            "restore",
            |lua, this, path: String| async move {
                let result = this
                    .0
                    .restore(std::path::Path::new(&path))
                    .await
                    .map_err(mlua::Error::external)?;
                json_to_lua(&lua, &serde_json::to_value(result).map_err(mlua::Error::external)?)
            },
        );
        methods.add_async_method("shutdown", |_, this, ()| async move {
            this.0.shutdown().await;
            Ok(())
        });
        methods.add_method("is_dead", |_, this, ()| Ok(this.0.is_dead()));
    }
}

/// The host_request channel, pumped from Lua policy (pi.spawn).
struct HostRequestRx(mpsc::UnboundedReceiver<pi_rs_repl::HostRequestMsg>);

impl UserData for HostRequestRx {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method_mut("receive", |_, mut this, ()| async move {
            let req = this
                .0
                .recv()
                .await
                .ok_or_else(|| mlua::Error::external("host request channel closed"))?;
            Ok(HostRequestHandle {
                kind: req.kind,
                payload: req.payload,
                reply: Some(req.reply),
            })
        });
    }
}

/// One in-flight host_request, answered with :reply(value).
struct HostRequestHandle {
    kind: String,
    payload: serde_json::Value,
    reply: Option<oneshot::Sender<serde_json::Value>>,
}

impl UserData for HostRequestHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get_kind", |_, this, ()| Ok(this.kind.clone()));
        methods.add_method("get_payload", |lua, this, ()| {
            json_to_lua(lua, &this.payload)
        });
        methods.add_method_mut("reply", |_lua, this, value: Value| {
            let payload = lua_to_json(value).map_err(mlua::Error::external)?;
            if let Some(tx) = this.reply.take() {
                let _ = tx.send(payload);
            }
            Ok(())
        });
    }
}
