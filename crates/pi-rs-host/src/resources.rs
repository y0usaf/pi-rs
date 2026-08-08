//! Session-scoped managed resources (PLAN 9.9 lifetimes contracts).
//!
//! Every mechanism that owns an external resource — a subprocess, a TCP
//! socket, a file watcher, a timer, a background task — registers a
//! dispose closure here when its handle is created. pi.resources.dispose_all()
//! runs the closures in reverse registration order (children before
//! parents), and the host runs the same disposal at VM shutdown, so no
//! process/task/socket/watcher survives its owner. The registry is
//! thread-local because the VM is single-threaded; a handle registers
//! exactly once and its dispose is idempotent.
//!
//! Lua surface (pi.resources):
//! - list() -> array of { kind = ..., label = ... } for leak assertions
//! - dispose_all() -> count of disposed resources
//! - count() -> number of live tracked resources

use std::cell::RefCell;

pub(crate) struct Resource {
    pub(crate) kind: &'static str,
    pub(crate) label: String,
    pub(crate) dispose: Box<dyn FnMut()>,
}

thread_local! {
    static RESOURCES: RefCell<Vec<Resource>> = const { RefCell::new(Vec::new()) };
}

/// Register a resource. dispose must be idempotent (it may run twice:
/// once from dispose_all, once from the handle's own drop path).
pub(crate) fn register(kind: &'static str, label: String, dispose: impl FnMut() + 'static) {
    RESOURCES.with(|r| r.borrow_mut().push(Resource { kind, label, dispose: Box::new(dispose) }));
}

pub(crate) fn list() -> Vec<(String, String)> {
    RESOURCES.with(|r| {
        r.borrow()
            .iter()
            .map(|res| (res.kind.to_owned(), res.label.clone()))
            .collect()
    })
}

pub(crate) fn len() -> usize {
    RESOURCES.with(|r| r.borrow().len())
}

// Dispose every tracked resource, children first. Returns the count.

/// Remove one tracked resource by kind + label (explicit dispose path).
pub(crate) fn unregister(kind: &'static str, label: &str) -> bool {
    RESOURCES.with(|r| {
        let mut resources = r.borrow_mut();
        let before = resources.len();
        resources.retain(|res| !(res.kind == kind && res.label == label));
        resources.len() != before
    })
}

pub(crate) fn dispose_all() -> usize {
    RESOURCES.with(|r| {
        let mut resources = r.borrow_mut();
        let count = resources.len();
        for mut res in resources.drain(..).rev() {
            (res.dispose)();
        }
        count
    })
}

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let resources = lua.create_table()?;
    resources.set(
        "list",
        lua.create_function(|lua, ()| {
            let out = lua.create_table()?;
            for (kind, label) in list() {
                let entry = lua.create_table()?;
                entry.set("kind", kind)?;
                entry.set("label", label)?;
                out.push(entry)?;
            }
            Ok(out)
        })?,
    )?;
    resources.set(
        "dispose_all",
        lua.create_function(|_, ()| Ok(dispose_all()))?,
    )?;
    resources.set(
        "count",
        lua.create_function(|_, ()| Ok(len()))?,
    )?;
    pi.set("resources", resources)?;
    Ok(())
}
