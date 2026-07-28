//! Dedicated Lua VM and watchdog-bounded package/root dispatch.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::time::Instant;

use crate::kernel::{CancellationToken, Control, DispatchBatch, DispatchRequest, ScopeId};
use crate::{HostConfig, HostError};

const NTH_INSTRUCTION: u32 = 1000;

pub(crate) enum Msg {
    Load {
        source_key: String,
        source: String,
        scope: ScopeId,
        reply: SyncSender<Result<(), HostError>>,
    },
    Dispatch {
        request: DispatchRequest,
        reply: SyncSender<Result<DispatchBatch, HostError>>,
    },
    DisposePackage {
        scope: ScopeId,
        reply: SyncSender<Result<(), HostError>>,
    },
    Shutdown {
        scopes: Vec<(ScopeId, String)>,
        reply: SyncSender<()>,
    },
}

pub(crate) fn spawn(
    config: HostConfig,
    control: Arc<Control>,
    effects: crate::effects::EffectHub,
    effect_runner: crate::effects::EffectRunner,
) -> Result<Sender<Msg>, HostError> {
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let (init_tx, init_rx) = sync_channel::<Result<(), String>>(1);
    std::thread::Builder::new()
        .name("pi-rs-host-lua".to_owned())
        .spawn(move || vm_main(config, control, effects, effect_runner, rx, init_tx))
        .map_err(|_| HostError::VmUnavailable)?;
    match init_rx.recv() {
        Ok(Ok(())) => Ok(tx),
        Ok(Err(message)) => Err(HostError::Lua(message)),
        Err(_) => Err(HostError::VmUnavailable),
    }
}

fn vm_main(
    config: HostConfig,
    control: Arc<Control>,
    effects: crate::effects::EffectHub,
    effect_runner: crate::effects::EffectRunner,
    rx: Receiver<Msg>,
    init_tx: SyncSender<Result<(), String>>,
) {
    let init = || -> Result<(mlua::Lua, mlua::Table, tokio::runtime::Runtime), String> {
        let lua = mlua::Lua::new();
        let cwd = config.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_owned())
        });
        let pi = crate::bindings::build(
            &lua,
            &cwd,
            config.clone(),
            Arc::clone(&control),
            effects.clone(),
        )
        .map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        Ok((lua, pi, runtime))
    };
    let (lua, pi, runtime) = match init() {
        Ok(parts) => {
            let _ = init_tx.send(Ok(()));
            parts
        }
        Err(message) => {
            let _ = init_tx.send(Err(message));
            return;
        }
    };
    effect_runner.start(&runtime);

    while let Ok(message) = rx.recv() {
        match message {
            Msg::Load {
                source_key,
                source,
                scope,
                reply,
            } => {
                let result = crate::kernel_api::set_scope(&lua, Some(scope))
                    .map_err(|error| HostError::Lua(error.to_string()))
                    .and_then(|()| load_chunk(&lua, &runtime, &config, &pi, &source_key, &source));
                if result.is_err() {
                    let _ = crate::kernel_api::dispose_callbacks(
                        &lua, &runtime, &config, &control, scope,
                    );
                    let _ = remove_scope(&lua, scope);
                }
                let _ = crate::kernel_api::set_scope(&lua, None);
                let _ = reply.send(result);
            }
            Msg::Dispatch { request, reply } => {
                let _ = reply.send(dispatch_root(&lua, &runtime, &config, &control, &request));
            }
            Msg::DisposePackage { scope, reply } => {
                let _ = reply.send(dispose_scope(
                    &lua, &runtime, &config, &control, &effects, scope,
                ));
            }
            Msg::Shutdown { scopes, reply } => {
                for (scope, _) in scopes {
                    let _ = dispose_scope(&lua, &runtime, &config, &control, &effects, scope);
                }
                let _ = reply.send(());
                return;
            }
        }
    }
}

fn remove_scope(lua: &mlua::Lua, scope: ScopeId) -> Result<(), HostError> {
    crate::kernel_api::remove_scope(lua, scope)
        .and_then(|()| crate::module_api::remove_scope(lua, scope))
        .and_then(|()| crate::middleware::remove_scope(lua, scope))
        .map_err(|error| HostError::Lua(error.to_string()))
}

fn dispose_scope(
    lua: &mlua::Lua,
    runtime: &tokio::runtime::Runtime,
    config: &HostConfig,
    control: &Control,
    effects: &crate::effects::EffectHub,
    scope: ScopeId,
) -> Result<(), HostError> {
    runtime.block_on(effects.settle_scope(scope));
    let cleanup_source = format!("<cleanup:{}>", scope.get());
    let (cleanup_scope, _) = control.create_scope(cleanup_source)?;
    crate::kernel_api::set_scope(lua, Some(cleanup_scope))
        .map_err(|error| HostError::Lua(error.to_string()))?;
    let callbacks = crate::kernel_api::dispose_callbacks(lua, runtime, config, control, scope);
    let removal = remove_scope(lua, scope);
    let _ = control.dispose(cleanup_scope);
    let _ = crate::kernel_api::set_scope(lua, None);
    callbacks.and(removal)
}

fn load_chunk(
    lua: &mlua::Lua,
    runtime: &tokio::runtime::Runtime,
    config: &HostConfig,
    pi: &mlua::Table,
    source_key: &str,
    source: &str,
) -> Result<(), HostError> {
    let function = lua
        .load(source)
        .set_name(format!("@{source_key}"))
        .into_function()
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::api::set_current_source(lua, source_key);
    let result = dispatch_function(
        lua,
        runtime,
        config,
        function,
        mlua::Value::Table(pi.clone()),
        None,
    );
    crate::api::set_current_source(lua, "<host>");
    result.map(|_| ())
}

fn dispatch_root(
    lua: &mlua::Lua,
    runtime: &tokio::runtime::Runtime,
    config: &HostConfig,
    control: &Control,
    request: &DispatchRequest,
) -> Result<DispatchBatch, HostError> {
    let root = crate::kernel_api::resolve_root(lua, request.root)?;
    let cancellation = control.token(root.scope)?;
    if cancellation.is_cancelled() {
        return Err(HostError::Cancelled);
    }
    // Event middleware composes before the root handler; a stopped chain
    // publishes the stages' queued actions as the whole batch.
    let pipeline = runtime.block_on(crate::middleware::run_event_pipeline(
        lua,
        config,
        control,
        request.root,
        request.event.clone(),
        request.context.clone(),
    ))?;
    if pipeline.stopped {
        let batch = DispatchBatch {
            version: crate::kernel::KERNEL_API_VERSION,
            generation: control.generation(),
            scope: root.scope,
            source: root.source,
            actions: pipeline.actions,
            effects: Vec::new(),
        };
        return runtime.block_on(crate::middleware::apply_render_pipeline(
            lua,
            config,
            control,
            request.root,
            &request.event,
            batch,
        ));
    }
    let request = DispatchRequest::new(request.root, pipeline.event, request.context.clone());
    let generation = control.generation();
    crate::kernel_api::set_scope(lua, Some(root.scope))
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::kernel_api::begin_transaction(lua, generation, root.scope, cancellation.clone())
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::api::set_current_source(lua, &root.source);
    let result = crate::kernel_api::snapshot(lua, &request, generation, root.scope)
        .map_err(|error| HostError::Lua(error.to_string()))
        .and_then(|snapshot| {
            dispatch_function(
                lua,
                runtime,
                config,
                root.handler,
                snapshot,
                Some(cancellation),
            )
        });
    let batch = match result {
        Ok(_) => crate::kernel_api::finish_transaction(lua, root.source),
        Err(error) => {
            crate::kernel_api::clear_transaction(lua);
            Err(error)
        }
    };
    crate::api::set_current_source(lua, "<host>");
    let _ = crate::kernel_api::set_scope(lua, None);
    // Render middleware transforms the settled action list; a failing
    // transform rolls back the whole dispatch so nothing publishes.
    batch.and_then(|batch| {
        runtime.block_on(crate::middleware::apply_render_pipeline(
            lua,
            config,
            control,
            request.root,
            &request.event,
            batch,
        ))
    })
}

/// Dispatch one root kind from inside an active dispatch.
///
/// Used by the public `roots.v1.dispatch` binding: the calling root keeps its
/// own transaction; the nested root runs with its own transaction and returns
/// its batch to Lua as data. Nested execution shares the caller's runtime and
/// watchdog hook, so the outer dispatch budget bounds the whole nest; depth
/// and direct recursion are capped by the control guard. The caller's
/// scope/current-source are restored afterwards.
pub(crate) async fn dispatch_nested(
    lua: &mlua::Lua,

    config: &HostConfig,
    control: &Control,
    kind: crate::kernel::RootKind,
    event: serde_json::Value,
    context: serde_json::Value,
) -> Result<DispatchBatch, HostError> {
    control.enter_nested_dispatch(kind)?;
    let outcome = dispatch_nested_inner(lua, config, control, kind, event, context).await;
    control.leave_nested_dispatch();
    outcome
}

async fn dispatch_nested_inner(
    lua: &mlua::Lua,
    config: &HostConfig,
    control: &Control,
    kind: crate::kernel::RootKind,
    event: serde_json::Value,
    context: serde_json::Value,
) -> Result<DispatchBatch, HostError> {
    let request = DispatchRequest::new(kind, event, context);
    let root = crate::kernel_api::resolve_root(lua, request.root)?;
    let cancellation = control.token(root.scope)?;
    if cancellation.is_cancelled() {
        return Err(HostError::Cancelled);
    }
    // Nested dispatches compose the same middleware pipelines as top-level
    // dispatches: the stages bound to the nested root kind apply.
    let pipeline = crate::middleware::run_event_pipeline(
        lua,
        config,
        control,
        request.root,
        request.event.clone(),
        request.context.clone(),
    )
    .await?;
    if pipeline.stopped {
        let batch = DispatchBatch {
            version: crate::kernel::KERNEL_API_VERSION,
            generation: control.generation(),
            scope: root.scope,
            source: root.source,
            actions: pipeline.actions,
            effects: Vec::new(),
        };
        return crate::middleware::apply_render_pipeline(
            lua,
            config,
            control,
            request.root,
            &request.event,
            batch,
        )
        .await;
    }
    let request = DispatchRequest::new(kind, pipeline.event, request.context.clone());
    let generation = control.generation();
    let outer_scope = crate::api::registry_table(lua)
        .and_then(|registry| registry.get::<u64>("kernel_scope"))
        .unwrap_or(0);
    let outer_source = crate::api::current_source(lua);
    crate::kernel_api::set_scope(lua, Some(root.scope))
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::kernel_api::begin_transaction(lua, generation, root.scope, cancellation.clone())
        .map_err(|error| HostError::Lua(error.to_string()))?;
    crate::api::set_current_source(lua, &root.source);
    let result = crate::kernel_api::snapshot(lua, &request, generation, root.scope)
        .map_err(|error| HostError::Lua(error.to_string()))
        .and_then(|snapshot| {
            lua.create_thread(root.handler)
                .map_err(|error| HostError::Lua(error.to_string()))
                .map(|thread| (thread, snapshot))
        });
    let batch = match result {
        Ok((thread, snapshot)) => {
            match thread
                .into_async::<mlua::Value>(snapshot)
                .map_err(|error| HostError::Lua(error.to_string()))
            {
                Ok(future) => match future.await {
                    Ok(_) => crate::kernel_api::finish_transaction(lua, root.source),
                    Err(error) => {
                        crate::kernel_api::clear_transaction(lua);
                        Err(HostError::from_lua_message(error.to_string(), 0))
                    }
                },
                Err(error) => {
                    crate::kernel_api::clear_transaction(lua);
                    Err(error)
                }
            }
        }
        Err(error) => {
            crate::kernel_api::clear_transaction(lua);
            Err(error)
        }
    };
    crate::api::set_current_source(lua, &outer_source);
    let restore = if outer_scope == 0 {
        None
    } else {
        Some(ScopeId::from_raw(outer_scope))
    };
    let _ = crate::kernel_api::set_scope(lua, restore);
    match batch {
        Ok(batch) => {
            crate::middleware::apply_render_pipeline(
                lua,
                config,
                control,
                request.root,
                &request.event,
                batch,
            )
            .await
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn dispatch_function(
    lua: &mlua::Lua,
    runtime: &tokio::runtime::Runtime,
    config: &HostConfig,
    function: mlua::Function,
    args: impl mlua::IntoLuaMulti,
    cancellation: Option<CancellationToken>,
) -> Result<mlua::Value, HostError> {
    let (hook, state) = WatchdogHook::install(lua, config, cancellation)?;
    let result = (|| -> Result<mlua::Value, HostError> {
        let thread = lua
            .create_thread(function)
            .map_err(|error| HostError::Lua(error.to_string()))?;
        let future = thread
            .into_async::<mlua::Value>(args)
            .map_err(|error| HostError::Lua(error.to_string()))?;
        runtime
            .block_on(Watched {
                inner: Box::pin(future),
                state: Arc::clone(&state),
            })
            .map_err(|error| HostError::from_lua_message(error.to_string(), hook.budget_ms))
    })();
    hook.remove(lua);
    result
}

/// Async variant for execution already inside the VM runtime (middleware
/// stages run from nested dispatch): same watchdog and cancellation, no
/// nested `block_on`.
pub(crate) async fn dispatch_function_async(
    lua: &mlua::Lua,
    config: &HostConfig,
    function: mlua::Function,
    args: impl mlua::IntoLuaMulti,
    cancellation: Option<CancellationToken>,
) -> Result<mlua::Value, HostError> {
    let (hook, state) = WatchdogHook::install(lua, config, cancellation)?;
    let created = (|| -> Result<_, HostError> {
        let thread = lua
            .create_thread(function)
            .map_err(|error| HostError::Lua(error.to_string()))?;
        thread
            .into_async::<mlua::Value>(args)
            .map_err(|error| HostError::Lua(error.to_string()))
    })();
    let result = match created {
        Ok(future) => Watched {
            inner: Box::pin(future),
            state: Arc::clone(&state),
        }
        .await
        .map_err(|error| HostError::from_lua_message(error.to_string(), hook.budget_ms)),
        Err(error) => Err(error),
    };
    hook.remove(lua);
    result
}

struct WatchdogHook {
    budget_ms: i64,
}

impl WatchdogHook {
    fn install(
        lua: &mlua::Lua,
        config: &HostConfig,
        cancellation: Option<CancellationToken>,
    ) -> Result<(Self, Arc<WatchdogState>), HostError> {
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
                    "{} handler exceeded {budget_ms}ms of continuous Lua execution",
                    crate::error::WATCHDOG_MARKER
                )));
            }
            Ok(mlua::VmState::Continue)
        })
        .map_err(|error| HostError::Lua(error.to_string()))?;
        Ok((Self { budget_ms }, state))
    }

    fn remove(self, lua: &mlua::Lua) {
        lua.remove_global_hook();
    }
}

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

struct Watched<F> {
    inner: Pin<Box<F>>,
    state: Arc<WatchdogState>,
}

impl<F: Future> Future for Watched<F> {
    type Output = F::Output;

    fn poll(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.state.begin_slice();
        let result = self.inner.as_mut().poll(context);
        self.state.end_slice();
        if result.is_pending() {
            self.state.reset();
        }
        result
    }
}
