//! The Lua VM thread and the coroutine dispatch seam.
//!
//! The VM lives on one dedicated OS thread with a current-thread tokio
//! runtime; the host talks to it over a channel. Every dispatch into Lua
//! runs the target function as a Lua coroutine driven as a future, so
//! handlers may await host futures (`pi.sleep`, later: provider streams,
//! subprocesses) — the async seam locked in DESIGN.md.
//!
//! Watchdog semantics (doctrine 02, adapted to the seam): the budget
//! bounds *continuous Lua execution*. Each poll of the coroutine future
//! is a slice; time suspended awaiting a host future is free, and every
//! yield to a host future resets the window — a long-lived handler (the
//! agent loop processing thousands of stream deltas) accumulates unbounded
//! total Lua time as long as no single between-awaits stretch exceeds the
//! budget. A pure-Lua busy loop never awaits, so the instruction hook
//! kills it; awaits are unbounded by design (provider streams run for
//! minutes). Host bindings are async through tokio, so blocking C calls
//! do not exist on this surface.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::time::Instant;

use crate::api;
use crate::bindings;
use crate::convert::{json_to_lua, lua_to_json};
use crate::error::{HostError, WATCHDOG_MARKER};
use crate::kernel::{CancellationToken, Control, DispatchBatch, DispatchRequest, ScopeId};
use crate::{
    CommandInfo, FlagInfo, HostConfig, Outcome, ProviderInfo, RoleInfo, ToolInfo,
    ToolUpdateCallback,
};

/// Check the watchdog deadline every N VM instructions.
const NTH_INSTRUCTION: u32 = 1000;

pub(crate) enum Msg {
    /// Execute an extension chunk (registrations happen as side effects).
    /// The chunk runs on the coroutine path too, so top-level awaits work.
    Load {
        /// Chunk name for error messages and source attribution.
        source_key: String,
        source: String,
        scope: ScopeId,
        reply: SyncSender<Result<(), HostError>>,
    },
    /// Run a generic root through the snapshot/action transaction.
    Dispatch {
        request: DispatchRequest,
        reply: SyncSender<Result<DispatchBatch, HostError>>,
    },
    /// Dispose one package's callbacks and published registrations.
    DisposePackage {
        scope: ScopeId,
        reply: SyncSender<Result<(), HostError>>,
    },
    /// Final host-owner shutdown: cancel/dispose every remaining scope.
    Shutdown {
        scopes: Vec<(ScopeId, String)>,
        reply: SyncSender<()>,
    },
    /// Call every handler subscribed to `event`, sequentially in
    /// registration order, each under its own watchdog window; one hung or
    /// failing handler doesn't stop the rest.
    Emit {
        event: String,
        payload: serde_json::Value,
        reply: SyncSender<Vec<Outcome>>,
    },
    /// Host-side metadata mirror of registered tools (spec:
    /// `runner.getAllRegisteredTools()` — first registration per name wins).
    Tools {
        reply: SyncSender<Result<Vec<ToolInfo>, HostError>>,
    },
    /// Host-side metadata mirror of registered commands (spec:
    /// `runner.resolveRegisteredCommands()` — collisions get `name:N`).
    Commands {
        reply: SyncSender<Result<Vec<CommandInfo>, HostError>>,
    },
    /// Host-side metadata mirror of registered extension CLI flags.
    Flags {
        reply: SyncSender<Result<Vec<FlagInfo>, HostError>>,
    },
    /// Host-side metadata mirror of public application/frontend roles.
    Roles {
        reply: SyncSender<Result<Vec<RoleInfo>, HostError>>,
    },
    /// Update the shared parsed/default flag-value map.
    SetFlagValue {
        name: String,
        value: serde_json::Value,
        reply: SyncSender<Result<(), HostError>>,
    },
    /// Host-side metadata mirror of queued provider registrations (spec:
    /// the loader's queued `registerProvider` calls, drained in order).
    Providers {
        reply: SyncSender<Result<Vec<ProviderInfo>, HostError>>,
    },
    ExtensionConflicts {
        reply: SyncSender<Result<Vec<(String, String)>, HostError>>,
    },
    /// Execute a registered tool on the coroutine path.
    CallTool {
        name: String,
        tool_call_id: String,
        params: serde_json::Value,
        on_update: Option<ToolUpdateCallback>,
        reply: SyncSender<Result<serde_json::Value, HostError>>,
    },
    /// Run a registered command handler on the coroutine path.
    CallCommand {
        invocation_name: String,
        args: String,
        reply: SyncSender<Result<Option<serde_json::Value>, HostError>>,
    },
    /// Run the active declaration for a generic application/frontend role.
    CallRole {
        role: String,
        args: String,
        reply: SyncSender<Result<Option<serde_json::Value>, HostError>>,
    },
}

/// Spawn the dedicated VM thread. Returns after the VM has initialized
/// (or failed to).
pub(crate) fn spawn(config: HostConfig, control: Arc<Control>) -> Result<Sender<Msg>, HostError> {
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let (init_tx, init_rx) = sync_channel::<Result<(), String>>(1);
    std::thread::Builder::new()
        .name("pi-rs-host-lua".to_owned())
        .spawn(move || vm_main(config, control, rx, init_tx))
        .map_err(|_| HostError::VmUnavailable)?;
    match init_rx.recv() {
        Ok(Ok(())) => Ok(tx),
        Ok(Err(msg)) => Err(HostError::Lua(msg)),
        Err(_) => Err(HostError::VmUnavailable),
    }
}

fn vm_main(
    config: HostConfig,
    control: Arc<Control>,
    rx: Receiver<Msg>,
    init_tx: SyncSender<Result<(), String>>,
) {
    let init = || -> Result<(mlua::Lua, mlua::Table, tokio::runtime::Runtime, String), String> {
        let lua = mlua::Lua::new();
        // Host cwd for the OS bindings (spec: the loader's injected cwd).
        let cwd = config.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_owned())
        });
        let pi = bindings::build(&lua, &cwd, config.project_trusted, Arc::clone(&control))
            .map_err(|e| e.to_string())?;
        // enable_all: the process driver (pi.exec) needs the io/signal
        // drivers in addition to time.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        Ok((lua, pi, rt, cwd))
    };
    let (lua, pi, rt, cwd) = match init() {
        Ok(parts) => {
            let _ = init_tx.send(Ok(()));
            parts
        }
        Err(msg) => {
            let _ = init_tx.send(Err(msg));
            return;
        }
    };

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Load {
                source_key,
                source,
                scope,
                reply,
            } => {
                let result = crate::kernel_api::set_scope(&lua, Some(scope))
                    .map_err(|error| HostError::Lua(error.to_string()))
                    .and_then(|()| load_chunk(&lua, &rt, &config, &pi, &source_key, &source));
                if result.is_err() {
                    let _ =
                        crate::kernel_api::dispose_callbacks(&lua, &rt, &config, &control, scope);
                    let _ = api::remove_scope(&lua, scope);
                }
                let _ = crate::kernel_api::set_scope(&lua, None);
                let _ = reply.send(result);
            }
            Msg::Dispatch { request, reply } => {
                let result = dispatch_root(&lua, &rt, &config, &control, &request);
                let _ = reply.send(result);
            }
            Msg::DisposePackage { scope, reply } => {
                let callbacks =
                    crate::kernel_api::dispose_callbacks(&lua, &rt, &config, &control, scope);
                let removal = api::remove_scope(&lua, scope)
                    .map_err(|error| HostError::Lua(error.to_string()));
                let _ = reply.send(callbacks.and(removal));
            }
            Msg::Shutdown { scopes, reply } => {
                for (scope, _) in scopes {
                    let _ =
                        crate::kernel_api::dispose_callbacks(&lua, &rt, &config, &control, scope);
                    let _ = api::remove_scope(&lua, scope);
                }
                let _ = reply.send(());
                return;
            }
            Msg::Emit {
                event,
                payload,
                reply,
            } => {
                let outcomes = emit(&lua, &rt, &config, &event, &payload);
                let _ = reply.send(outcomes);
            }
            Msg::Tools { reply } => {
                let _ = reply.send(tools_mirror(&lua));
            }
            Msg::Commands { reply } => {
                let _ = reply.send(commands_mirror(&lua));
            }
            Msg::Flags { reply } => {
                let _ = reply.send(flags_mirror(&lua));
            }
            Msg::Roles { reply } => {
                let _ = reply.send(roles_mirror(&lua));
            }
            Msg::SetFlagValue { name, value, reply } => {
                let result = api::set_flag_value(&lua, &name, &value)
                    .map_err(|error| HostError::Lua(error.to_string()));
                let _ = reply.send(result);
            }
            Msg::Providers { reply } => {
                let _ = reply.send(providers_mirror(&lua));
            }
            Msg::ExtensionConflicts { reply } => {
                let result = api::extension_conflicts(&lua)
                    .map_err(|error| HostError::Lua(error.to_string()));
                let _ = reply.send(result);
            }
            Msg::CallTool {
                name,
                tool_call_id,
                params,
                on_update,
                reply,
            } => {
                let _ = reply.send(call_tool(
                    &lua,
                    &rt,
                    &config,
                    ToolInvocation {
                        name: &name,
                        tool_call_id: &tool_call_id,
                        params: &params,
                        on_update,
                        cwd: &cwd,
                    },
                ));
            }
            Msg::CallCommand {
                invocation_name,
                args,
                reply,
            } => {
                let _ = reply.send(call_command(
                    &lua,
                    &rt,
                    &config,
                    &invocation_name,
                    &args,
                    &cwd,
                ));
            }
            Msg::CallRole { role, args, reply } => {
                let _ = reply.send(call_role(&lua, &rt, &config, &role, &args, &cwd));
            }
        }
    }
}

fn tools_mirror(lua: &mlua::Lua) -> Result<Vec<ToolInfo>, HostError> {
    let tools = api::all_tools(lua).map_err(|e| HostError::Lua(e.to_string()))?;
    let mut out = Vec::with_capacity(tools.len());
    for (source, name, def) in tools {
        let meta = api::tool_meta(&def).map_err(|e| HostError::Lua(e.to_string()))?;
        out.push(ToolInfo { name, source, meta });
    }
    Ok(out)
}

fn providers_mirror(lua: &mlua::Lua) -> Result<Vec<ProviderInfo>, HostError> {
    let providers = api::all_providers(lua).map_err(|e| HostError::Lua(e.to_string()))?;
    let mut out = Vec::with_capacity(providers.len());
    for (source, name, config) in providers {
        let config = api::provider_meta(&config).map_err(|e| HostError::Lua(e.to_string()))?;
        out.push(ProviderInfo {
            name,
            source,
            config,
        });
    }
    Ok(out)
}

fn commands_mirror(lua: &mlua::Lua) -> Result<Vec<CommandInfo>, HostError> {
    Ok(api::resolved_commands(lua)
        .map_err(|e| HostError::Lua(e.to_string()))?
        .into_iter()
        .map(|c| CommandInfo {
            name: c.name,
            invocation_name: c.invocation_name,
            source: c.source,
            description: c.description,
        })
        .collect())
}

fn flags_mirror(lua: &mlua::Lua) -> Result<Vec<FlagInfo>, HostError> {
    api::all_flags(lua)
        .map_err(|error| HostError::Lua(error.to_string()))?
        .into_iter()
        .map(|(source, name, flag)| {
            let default = flag
                .get::<Option<mlua::Value>>("default")
                .map_err(|error| HostError::Lua(error.to_string()))?
                .map(crate::convert::lua_to_json)
                .transpose()
                .map_err(|error| HostError::Lua(error.to_string()))?;
            Ok(FlagInfo {
                name,
                source,
                description: flag
                    .get("description")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                flag_type: flag
                    .get("type")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                default,
            })
        })
        .collect()
}

fn roles_mirror(lua: &mlua::Lua) -> Result<Vec<RoleInfo>, HostError> {
    api::all_roles(lua)
        .map_err(|error| HostError::Lua(error.to_string()))?
        .into_iter()
        .map(|(source, _, role)| {
            Ok(RoleInfo {
                id: role
                    .get("id")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                role: role
                    .get("role")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                source,
                active: role
                    .get("active")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
                priority: role
                    .get("priority")
                    .map_err(|error| HostError::Lua(error.to_string()))?,
            })
        })
        .collect()
}

/// Run optional `def.prepare_arguments(params)`, coerce and validate against
/// `def.parameters`, then execute `(tool_call_id, params, signal, on_update,
/// ctx)` under the watchdog. Results and updates cross as uninterpreted JSON.
struct ToolInvocation<'a> {
    name: &'a str,
    tool_call_id: &'a str,
    params: &'a serde_json::Value,
    on_update: Option<ToolUpdateCallback>,
    cwd: &'a str,
}

fn call_tool(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    invocation: ToolInvocation<'_>,
) -> Result<serde_json::Value, HostError> {
    let ToolInvocation {
        name,
        tool_call_id,
        params,
        on_update,
        cwd,
    } = invocation;
    let found = api::find_tool(lua, name).map_err(|e| HostError::Lua(e.to_string()))?;
    let Some((source, def)) = found else {
        return Err(HostError::UnknownTool(name.to_owned()));
    };
    let execute: mlua::Function = def
        .get("execute")
        .map_err(|e| HostError::Lua(e.to_string()))?;
    api::set_current_source(lua, &source);
    let res = json_to_lua(lua, params)
        .map_err(|e| HostError::Lua(e.to_string()))
        .and_then(
            |arg| match def.get::<Option<mlua::Function>>("prepare_arguments") {
                Ok(Some(prepare)) => dispatch(lua, rt, config, prepare, arg),
                Ok(None) => Ok(arg),
                Err(error) => Err(HostError::Lua(error.to_string())),
            },
        )
        .and_then(|arg| {
            let raw = lua_to_json(arg).map_err(|error| HostError::Lua(error.to_string()))?;
            let schema = def
                .get::<Option<mlua::Value>>("parameters")
                .map_err(|error| HostError::Lua(error.to_string()))?
                .map(lua_to_json)
                .transpose()
                .map_err(|error| HostError::Lua(error.to_string()))?
                .unwrap_or_else(|| serde_json::json!({}));
            let validated = crate::schema::validate_tool_arguments(name, &schema, &raw)
                .map_err(|error| HostError::Lua(error.to_string()))?;
            let arg =
                json_to_lua(lua, &validated).map_err(|error| HostError::Lua(error.to_string()))?;
            let signal = pi_rs_ai::transport::AbortSignal::new();
            let signal_value = crate::ai::signal_userdata(lua, signal)
                .map_err(|error| HostError::Lua(error.to_string()))?;
            let ctx = invocation_context(lua, cwd, signal_value.clone())?;
            let callback = on_update.map(|sink| {
                lua.create_function(move |_, value: mlua::Value| {
                    let update = lua_to_json(value)
                        .map_err(|error| mlua::Error::runtime(error.to_string()))?;
                    sink(update);
                    Ok(())
                })
            });
            match callback {
                Some(Ok(callback)) => dispatch(
                    lua,
                    rt,
                    config,
                    execute,
                    (tool_call_id.to_owned(), arg, signal_value, callback, ctx),
                ),
                Some(Err(error)) => Err(HostError::Lua(error.to_string())),
                None => dispatch(
                    lua,
                    rt,
                    config,
                    execute,
                    (tool_call_id.to_owned(), arg, signal_value, mlua::Nil, ctx),
                ),
            }
        })
        .and_then(|v| lua_to_json(v).map_err(|e| HostError::Lua(e.to_string())));
    api::set_current_source(lua, "<host>");
    res
}

/// Run a command handler as `handler(args, ctx)` under the watchdog.
fn call_command(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    invocation_name: &str,
    args: &str,
    cwd: &str,
) -> Result<Option<serde_json::Value>, HostError> {
    let commands = api::resolved_commands(lua).map_err(|e| HostError::Lua(e.to_string()))?;
    let Some(cmd) = commands
        .into_iter()
        .find(|c| c.invocation_name == invocation_name)
    else {
        return Err(HostError::UnknownCommand(invocation_name.to_owned()));
    };
    api::set_current_source(lua, &cmd.source);
    let signal = pi_rs_ai::transport::AbortSignal::new();
    let res = crate::ai::signal_userdata(lua, signal)
        .map_err(|error| HostError::Lua(error.to_string()))
        .and_then(|signal| invocation_context(lua, cwd, signal))
        .and_then(|ctx| dispatch(lua, rt, config, cmd.handler, (args.to_owned(), ctx)))
        .and_then(value_to_reply);
    api::set_current_source(lua, "<host>");
    res
}

fn call_role(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    role: &str,
    args: &str,
    cwd: &str,
) -> Result<Option<serde_json::Value>, HostError> {
    let selected =
        api::resolve_role(lua, role).map_err(|error| HostError::Lua(error.to_string()))?;
    let Some(selected) = selected else {
        return Err(HostError::UnknownRole(role.to_owned()));
    };
    api::set_current_source(lua, &selected.source);
    let signal = pi_rs_ai::transport::AbortSignal::new();
    let result = crate::ai::signal_userdata(lua, signal)
        .map_err(|error| HostError::Lua(error.to_string()))
        .and_then(|signal| invocation_context(lua, cwd, signal))
        .and_then(|ctx| dispatch(lua, rt, config, selected.handler, (args.to_owned(), ctx)))
        .and_then(value_to_reply);
    api::set_current_source(lua, "<host>");
    result
}

fn invocation_context(
    lua: &mlua::Lua,
    cwd: &str,
    signal: mlua::AnyUserData,
) -> Result<mlua::Table, HostError> {
    let ctx = lua
        .create_table()
        .map_err(|error| HostError::Lua(error.to_string()))?;
    ctx.set("cwd", cwd)
        .map_err(|error| HostError::Lua(error.to_string()))?;
    ctx.set("signal", signal)
        .map_err(|error| HostError::Lua(error.to_string()))?;
    ctx.set("isIdle", false)
        .map_err(|error| HostError::Lua(error.to_string()))?;
    Ok(ctx)
}

fn load_chunk(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    pi: &mlua::Table,
    source_key: &str,
    source: &str,
) -> Result<(), HostError> {
    let func = lua
        .load(source)
        .set_name(format!("@{source_key}"))
        .into_function()
        .map_err(|e| HostError::Lua(e.to_string()))?;
    api::set_current_source(lua, source_key);
    let res = dispatch(lua, rt, config, func, mlua::Value::Table(pi.clone()));
    api::set_current_source(lua, "<host>");
    res.map(|_| ())
}

fn emit(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    event: &str,
    payload: &serde_json::Value,
) -> Vec<Outcome> {
    let handlers = match api::event_handlers(lua, event) {
        Ok(handlers) => handlers,
        Err(e) => {
            return vec![Outcome {
                source: "<host>".to_owned(),
                result: Err(e.to_string()),
            }];
        }
    };

    let mut outcomes = Vec::with_capacity(handlers.len());
    for (source, handler) in handlers {
        // Registrations made from inside a handler attribute to its extension.
        api::set_current_source(lua, &source);
        let result = json_to_lua(lua, payload)
            .map_err(|e| HostError::Lua(e.to_string()))
            .and_then(|arg| dispatch(lua, rt, config, handler, arg))
            .and_then(value_to_reply)
            .map_err(|e| e.to_string());
        outcomes.push(Outcome { source, result });
    }
    api::set_current_source(lua, "<host>");
    outcomes
}

fn dispatch_root(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    control: &Control,
    request: &DispatchRequest,
) -> Result<DispatchBatch, HostError> {
    let root = crate::kernel_api::resolve_root(lua, request.root)?;
    let cancellation = control.token(root.scope)?;
    if cancellation.is_cancelled() {
        return Err(HostError::Cancelled);
    }
    let generation = control.generation();
    crate::kernel_api::set_scope(lua, Some(root.scope))
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::kernel_api::begin_transaction(lua, generation, root.scope, cancellation.clone())
        .map_err(|error| HostError::Lua(error.to_string()))?;
    api::set_current_source(lua, &root.source);
    let result = crate::kernel_api::snapshot(lua, request, generation, root.scope)
        .map_err(|error| HostError::Lua(error.to_string()))
        .and_then(|snapshot| {
            dispatch_function(lua, rt, config, root.handler, snapshot, Some(cancellation))
        });
    let batch = match result {
        Ok(_) => crate::kernel_api::finish_transaction(lua, root.source),
        Err(error) => {
            crate::kernel_api::clear_transaction(lua);
            Err(error)
        }
    };
    api::set_current_source(lua, "<host>");
    let _ = crate::kernel_api::set_scope(lua, None);
    batch
}

/// Run one Lua function as a coroutine driven to completion on the VM
/// thread's runtime, bounded by the slice-counting watchdog.
fn dispatch(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    func: mlua::Function,
    args: impl mlua::IntoLuaMulti,
) -> Result<mlua::Value, HostError> {
    dispatch_function(lua, rt, config, func, args, None)
}

pub(crate) fn dispatch_function(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    func: mlua::Function,
    args: impl mlua::IntoLuaMulti,
    cancellation: Option<CancellationToken>,
) -> Result<mlua::Value, HostError> {
    let budget_ms = config.dispatch_timeout_ms;
    let state = Arc::new(WatchdogState::new(budget_ms));

    let triggers = mlua::HookTriggers::new().every_nth_instruction(NTH_INSTRUCTION);
    let hook_state = Arc::clone(&state);
    lua.set_global_hook(triggers, move |_lua, _debug| {
        if cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(mlua::Error::runtime(crate::error::CANCEL_MARKER));
        }
        if hook_state.exceeded() {
            return Err(mlua::Error::runtime(format!(
                "{WATCHDOG_MARKER} handler exceeded {budget_ms}ms of continuous Lua execution"
            )));
        }
        Ok(mlua::VmState::Continue)
    })
    .map_err(|e| HostError::Lua(e.to_string()))?;

    let res = (|| -> Result<mlua::Value, HostError> {
        let thread = lua
            .create_thread(func)
            .map_err(|e| HostError::Lua(e.to_string()))?;
        let fut = thread
            .into_async::<mlua::Value>(args)
            .map_err(|e| HostError::Lua(e.to_string()))?;
        // The dispatch runs inside a LocalSet so handlers can start
        // background coroutines (`pi.spawn`) that interleave with the
        // dispatched coroutine at await points. Watched wraps the whole
        // run_until drive, so spawned-task Lua execution is metered by the
        // same continuous-execution window as the main coroutine. Tasks
        // still pending when the dispatch returns are dropped with the
        // LocalSet (spawned work lives within its dispatch).
        let local = tokio::task::LocalSet::new();
        rt.block_on(Watched {
            inner: Box::pin(local.run_until(fut)),
            state: Arc::clone(&state),
        })
        .map_err(|e| HostError::from_lua_message(e.to_string(), budget_ms))
    })();

    lua.remove_global_hook();
    res
}

/// Watchdog bookkeeping shared between the poll wrapper and the
/// instruction hook. Times are µs since `epoch`; `slice_start_us < 0`
/// means the coroutine is suspended (awaiting) and the clock is not
/// running against the budget. The budget bounds continuous Lua
/// execution: yielding to a host future resets the consumed counter.
struct WatchdogState {
    epoch: Instant,
    budget_us: i64,
    consumed_us: AtomicI64,
    slice_start_us: AtomicI64,
}

impl WatchdogState {
    fn new(budget_ms: i64) -> Self {
        Self {
            epoch: Instant::now(),
            budget_us: budget_ms.saturating_mul(1000),
            consumed_us: AtomicI64::new(0),
            slice_start_us: AtomicI64::new(-1),
        }
    }

    fn now_us(&self) -> i64 {
        i64::try_from(self.epoch.elapsed().as_micros()).unwrap_or(i64::MAX)
    }

    fn begin_slice(&self) {
        self.slice_start_us.store(self.now_us(), Ordering::Relaxed);
    }

    fn end_slice(&self) {
        let start = self.slice_start_us.swap(-1, Ordering::Relaxed);
        if start >= 0 {
            self.consumed_us
                .fetch_add(self.now_us().saturating_sub(start), Ordering::Relaxed);
        }
    }

    /// New continuous-execution window: the coroutine yielded to a host
    /// future, so time already consumed no longer counts against the
    /// budget.
    fn reset(&self) {
        self.consumed_us.store(0, Ordering::Relaxed);
    }

    fn exceeded(&self) -> bool {
        let consumed = self.consumed_us.load(Ordering::Relaxed);
        let start = self.slice_start_us.load(Ordering::Relaxed);
        let running = if start >= 0 {
            self.now_us().saturating_sub(start)
        } else {
            0
        };
        consumed.saturating_add(running) >= self.budget_us
    }
}

/// Future wrapper that meters Lua execution: every poll of the coroutine
/// is a watchdog slice; the time between polls (host future pending) is
/// free, and a pending poll (the handler awaited) resets the budget
/// window.
struct Watched<F> {
    inner: Pin<Box<F>>,
    state: Arc<WatchdogState>,
}

impl<F: Future> Future for Watched<F> {
    type Output = F::Output;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.state.begin_slice();
        let res = self.inner.as_mut().poll(cx);
        self.state.end_slice();
        if res.is_pending() {
            self.state.reset();
        }
        res
    }
}

/// Serialize a handler return value for the reply. `nil` → `None`; a bare
/// string is shorthand for `{ message = ... }`; anything else is converted
/// as-is and handed to the caller uninterpreted — the host never matches
/// on result variants (DESIGN.md: no closed enums at the seam).
fn value_to_reply(val: mlua::Value) -> Result<Option<serde_json::Value>, HostError> {
    match val {
        mlua::Value::Nil => Ok(None),
        mlua::Value::String(s) => {
            let msg = s.to_str().map_err(|e| HostError::Lua(e.to_string()))?;
            Ok(Some(serde_json::json!({ "message": msg.to_owned() })))
        }
        other => {
            let json = lua_to_json(other).map_err(|e| HostError::Lua(e.to_string()))?;
            Ok(Some(json))
        }
    }
}
