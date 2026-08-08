//! pi.timer — Lua-native timers (PLAN 9.9 scoped timers/resources).
//!
//! setTimeout/setInterval equivalents on the coroutine seam. Ids are
//! positive integers. Callbacks run on the VM thread; the dispatch drive
//! loop (vm.rs) ticks due timers while a coroutine is suspended awaiting
//! a host future, so pi.sleep and long dispatches still deliver timer
//! callbacks. Each live timer is a tracked resource; clear_timer or VM
//! shutdown disposes it (timer.* lifetime contracts).
//!
//! Lua surface (pi.timer):
//! - set_timeout(ms, fn) -> id
//! - set_interval(ms, fn) -> id
//! - clear_timeout(id) -> bool  (alias clear_timer)
//! - clear_interval(id) -> bool
//! - clear_timer(id) -> bool
//! - dispose_all() -> count (shutdown hook; also via pi.resources)

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, Table};

pub(crate) struct TimerEntry {
    pub(crate) due: Instant,
    /// Some(period_ms) => interval; None => one-shot timeout.
    pub(crate) period_ms: Option<u64>,
    pub(crate) func: Function,
}

thread_local! {
    static TIMERS: RefCell<HashMap<u64, TimerEntry>> = RefCell::new(HashMap::new());
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    })
}

pub(crate) fn set(period_ms: Option<u64>, func: Function) -> u64 {
    let id = next_id();
    let entry = TimerEntry {
        due: Instant::now() + Duration::from_millis(period_ms.unwrap_or(0)),
        period_ms,
        func,
    };
    TIMERS.with(|t| {
        t.borrow_mut().insert(id, entry);
    });
    let label = format!("timer-{id}");
    let kind = if period_ms.is_some() { "timer.interval" } else { "timer.timeout" };
    crate::resources::register(kind, label, move || {
        clear(id);
    });
    id
}

pub(crate) fn clear(id: u64) -> bool {
    TIMERS.with(|t| t.borrow_mut().remove(&id).is_some())
}

/// Number of live timers (leak assertions).
pub(crate) fn count() -> usize {
    TIMERS.with(|t| t.borrow().len())
}

/// Earliest due instant across all timers, if any.
pub(crate) fn next_due() -> Option<Instant> {
    TIMERS.with(|t| t.borrow().values().map(|e| e.due).min())
}

/// Harvest every due timer. One-shots are removed; intervals re-register
/// with their next due instant before the callback runs.
pub(crate) fn take_due() -> Vec<TimerEntry> {
    let now = Instant::now();
    TIMERS.with(|t| {
        let mut timers = t.borrow_mut();
        let mut due = Vec::new();
        let mut keep = HashMap::new();
        for (id, entry) in timers.drain() {
            if entry.due <= now {
                if let Some(period) = entry.period_ms {
                    let mut interval = entry;
                    interval.due = now + Duration::from_millis(period);
                    // Intervals fire this tick too: return a copy with the
                    // re-registered due so the drive loop invokes the callback.
                    due.push(TimerEntry {
                        due: interval.due,
                        period_ms: interval.period_ms,
                        func: interval.func.clone(),
                    });
                    keep.insert(id, interval);
                } else {
                    // one-shot: consumed by this tick
                    due.push(entry);
                }
            } else {
                keep.insert(id, entry);
            }
        }
        *timers = keep;
        due
    })
}

/// Drop every timer (VM shutdown path).
pub(crate) fn dispose_all() -> usize {
    TIMERS.with(|t| {
        let mut timers = t.borrow_mut();
        let count = timers.len();
        timers.clear();
        count
    })
}

fn install_timer(
    lua: &Lua,
    pi: &Table,
    name: &str,
    period: Option<u64>,
) -> mlua::Result<()> {
    let closure_name = name.to_owned();
    let f = lua.create_function(move |_, (ms, func): (u64, Function)| {
        if ms == 0 {
            return Err(mlua::Error::runtime(format!("pi.timer.{closure_name}: ms must be > 0")));
        }
        Ok(set(period.map(|_| ms), func))
    })?;
    pi.set(name, f)
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let timer = lua.create_table()?;
    install_timer(lua, &timer, "set_timeout", None)?;
    install_timer(lua, &timer, "set_interval", Some(0))?;
    timer.set(
        "clear_timeout",
        lua.create_function(|_, id: u64| Ok(clear(id)))?,
    )?;
    timer.set(
        "clear_interval",
        lua.create_function(|_, id: u64| Ok(clear(id)))?,
    )?;
    timer.set(
        "clear_timer",
        lua.create_function(|_, id: u64| Ok(clear(id)))?,
    )?;
    timer.set(
        "count",
        lua.create_function(|_, ()| Ok(count()))?,
    )?;
    timer.set(
        "dispose_all",
        lua.create_function(|_, ()| Ok(dispose_all()))?,
    )?;
    pi.set("timer", timer)?;
    Ok(())
}
