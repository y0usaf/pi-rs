//! Coroutine, timer, and bounded-concurrency Lua bindings.

use crate::tui_api::runtime::LuaSpawnHandle;
use mlua::AnyUserData;

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    // Awaitable host future: the calling coroutine suspends; the VM thread
    // stays free to run the timer. Await time is excluded from the watchdog
    // budget (see vm.rs). The optional signal ports Pi's sleep(ms, signal)
    // cancellation seam used by AgentSession retry backoff.
    let sleep = lua.create_async_function(
        |lua, (ms, signal): (u64, Option<AnyUserData>)| async move {
            let signal = signal
                .map(|signal| {
                    signal
                        .borrow::<crate::ai::LuaAbortSignal>()
                        .map(|signal| signal.0.clone())
                })
                .transpose()?;
            let scope = crate::kernel_api::current_cancellation(&lua)?;
            match (signal, scope) {
                (Some(signal), Some(scope)) => tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                    () = signal.aborted() => Err(mlua::Error::runtime("sleep aborted")),
                    () = scope.cancelled() => Err(mlua::Error::runtime(crate::error::CANCEL_MARKER)),
                },
                (Some(signal), None) => tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                    () = signal.aborted() => Err(mlua::Error::runtime("sleep aborted")),
                },
                (None, Some(scope)) => tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                    () = scope.cancelled() => Err(mlua::Error::runtime(crate::error::CANCEL_MARKER)),
                },
                (None, None) => {
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    Ok(())
                }
            }
        },
    )?;
    pi.set("sleep", sleep)?;
    // Generic structured concurrency mechanism. Policy (which tasks to group,
    // and how to order their results) remains in Lua. Results are reported in
    // completion order so callers can reproduce Promise completion semantics.
    let parallel = lua.create_async_function(|lua, tasks: mlua::Table| async move {
        use futures_util::stream::{FuturesUnordered, StreamExt};

        let pending = FuturesUnordered::new();
        for (offset, task) in tasks.sequence_values::<mlua::Function>().enumerate() {
            let task = task?;
            pending.push(async move {
                let result = task.call_async::<mlua::Value>(()).await;
                (offset + 1, result)
            });
        }
        let completed = lua.create_table()?;
        let mut pending = pending;
        while let Some((index, result)) = pending.next().await {
            let entry = lua.create_table()?;
            entry.set("index", index)?;
            match result {
                Ok(value) => {
                    entry.set("ok", true)?;
                    entry.set("value", value)?;
                }
                Err(error) => {
                    entry.set("ok", false)?;
                    entry.set("error", error.to_string())?;
                }
            }
            completed.push(entry)?;
        }
        Ok(completed)
    })?;
    pi.set("parallel", parallel)?;
    // Structured background concurrency for long-lived handlers: the
    // function runs as its own coroutine on the dispatch's task set,
    // interleaving with the caller at await points (the interactive
    // frontend runs agent turns this way while its event loop keeps
    // rendering). The task lives within the current dispatch — anything
    // still pending when the dispatch returns is dropped.
    pi.set(
        "spawn",
        lua.create_function(|lua, func: mlua::Function| {
            let handle = tokio::task::spawn_local(func.call_async::<mlua::Value>(()));
            lua.create_userdata(LuaSpawnHandle(std::cell::RefCell::new(Some(handle))))
        })?,
    )?;
    let epoch = std::time::Instant::now();
    pi.set(
        "monotonic_ms",
        lua.create_function(move |_, ()| {
            Ok(u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX))
        })?,
    )?;
    // JS `Date.now()` — epoch milliseconds (the spec's timestamps and
    // timing gates use wall-clock ms, not seconds).
    pi.set(
        "now_ms",
        lua.create_function(|_, ()| {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0))
        })?,
    )?;

    Ok(())
}
