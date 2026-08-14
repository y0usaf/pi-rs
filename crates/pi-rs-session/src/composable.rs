//! Spatiotemporal composability for session persistence.
//!
//! Sessions are host-owned state with an explicit lifecycle. This module
//! puts that ownership on the `pi-rs-kernel` substrate and adds the
//! lifecycle the minimal kernel deliberately omits:
//!
//! - **Temporal (effect scope)** — a [`SessionScope`] commits one live
//!   [`SessionManager`] handle on the shared [`kernel::Context`] and records
//!   an [`kernel::Inverse`] that detaches it on unmount. Unmounting replays
//!   the inverse, so a mounted session leaves no residue (no leaked manager
//!   handle, no orphaned persistence handle).
//! - **Spatial (reactive dependency graph)** — a consumer declares the
//!   context keys it *reads* (`session:active`, `session:leaf`, ...). A
//!   committed [`kernel::Context::set`] on exactly one declared key notifies
//!   only the consumers that declared it; undeclared changes never fire a
//!   reaction.
//! - **Atomic reload** — [`SessionReloader::publish`] follows
//!   `build → activate → publish → drain`: the next generation is built
//!   (preflight) *before* anything replaces the current one, so a build
//!   failure leaves the old session untouched; publish is a single
//!   [`kernel::Context::set`] swap; drain drops the old handle only after
//!   the new one is published, so in-flight work on the old manager finishes
//!   before it is freed.
//!
//! Effect-scope inverse semantics only cover reverse-LIFO rollback, which is
//! the right contract for *detaching* a resource. Replacing a published
//! generation is not a rollback, so [`SessionReloader::publish`] performs an
//! explicit swap rather than stacking a second inverse-effect of the same
//! key.

use std::sync::Arc;
use std::sync::Mutex;

use pi_rs_kernel::{Component, Context, Inverse};

pub use pi_rs_kernel;

/// Canonical context key identifying the active [`SessionManager`]. Readers
/// of this key react to session (re)loads.
pub const KEY_ACTIVE: &str = "session:active";
/// Context key for the branch / leaf a consumer navigated to.
pub const KEY_LEAF: &str = "session:leaf";
/// Context key for the reconstructed LLM context (`buildSessionContext`).
pub const KEY_CONTEXT: &str = "session:context";

/// A mounted, host-owned `SessionManager` handle.
///
/// The inner manager is wrapped in a `Mutex` because the session manager's
/// API is `&mut self`; the outer `Arc` lets one live handle be shared by
/// several consumers (frontend subscription, persistence writer) without
/// double ownership.
pub type SessionManagerHandle = Arc<Mutex<super::SessionManager>>;

/// A composition unit that owns one live session handle on the context.
///
/// Mounting commits the handle under [`KEY_ACTIVE`] and returns an inverse;
/// unmounting (via the kernel) replays that inverse and detaches it, so the
/// session leaves no residue in the context.
pub struct SessionScope {
    inner: Component,
}

impl SessionScope {
    /// Build a scope that owns `manager` under [`KEY_ACTIVE`]. `reads`
    /// declares the context keys this consumer reacts to (a `set` on one of
    /// them fires its `on_change`, if any).
    pub fn new(manager: SessionManagerHandle, reads: Vec<&'static str>) -> Self {
        let mut inner = Component::new(reads);
        inner
            .effects
            .push(Box::new(move |ctx| commit_active(ctx, manager.clone())));
        Self { inner }
    }

    /// Build a scope that owns `manager` and reacts (`on_change`) to each
    /// declared read key.
    pub fn with_reaction(
        manager: SessionManagerHandle,
        reads: Vec<&'static str>,
        on_change: impl FnMut(&mut Context, &'static str) + Send + 'static,
    ) -> Self {
        let mut inner = Component::new(reads);
        inner
            .effects
            .push(Box::new(move |ctx| commit_active(ctx, manager.clone())));
        inner.on_change = Some(Box::new(on_change));
        Self { inner }
    }

    /// Consume the scope and yield the kernel [`Component`], ready for
    /// [`kernel::Context::mount`].
    pub fn into_component(self) -> Component {
        self.inner
    }
}

fn commit_active(ctx: &mut Context, handle: SessionManagerHandle) -> Inverse {
    let prev_has = ctx.has(KEY_ACTIVE);
    let prev = ctx.get::<SessionManagerHandle>(KEY_ACTIVE).cloned();
    let handle = handle.clone();
    ctx.set(KEY_ACTIVE, handle.clone());
    Box::new(move |ctx| {
        match prev_has {
            true => match prev {
                // Restore exactly the prior live handle.
                Some(prev) => ctx.set(KEY_ACTIVE, prev),
                None => ctx.remove(KEY_ACTIVE),
            },
            false => {
                ctx.remove(KEY_ACTIVE);
            }
        }
    })
}

/// Error from an atomic session reload.
#[derive(Debug, thiserror::Error)]
pub enum ReloadError {
    /// The next generation failed to build/preflight. The old session is
    /// untouched.
    #[error("session reload preflight failed: {0}")]
    Preflight(String),
}

/// Owner of the published session generation.
///
/// Holds no tolerable rollback for a swap: [`publish`](Self::publish) builds
/// the next generation first, then atomically replaces the current handle on
/// the context and drops the old one.
#[derive(Default)]
pub struct SessionReloader {
    current: Option<SessionManagerHandle>,
}

impl SessionReloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently published handle, if any.
    pub fn current(&self) -> Option<&SessionManagerHandle> {
        self.current.as_ref()
    }

    /// Atomic reload: build → activate → publish → drain.
    ///
    /// 1. **Build (preflight)** — `build` must fully construct (and validate)
    ///    the next generation. If it errors, nothing has changed.
    /// 2. **Activate** — the built handle is ready on return.
    /// 3. **Publish** — the context's [`KEY_ACTIVE`] value is swapped with a
    ///    single [`Context::set`], notifying exactly the readers that
    ///    declared [`KEY_ACTIVE`].
    /// 4. **Drain** — the previous handle is retained in the returned
    ///    [`DrainOutcome`] so in-flight work on the old manager can finish;
    ///    only after it completes does the caller release it.
    ///
    /// The caller's `build` may choose to reuse the current handle to perform
    /// an in-place reconstruction; comparing `Arc` pointers makes that
    /// explicit.
    pub fn publish<F>(
        &mut self,
        ctx: &mut Context,
        build: F,
    ) -> std::result::Result<DrainOutcome, ReloadError>
    where
        F: FnOnce() -> std::result::Result<SessionManagerHandle, String>,
    {
        // 1. Build (preflight failure → old session untouched).
        let next = build().map_err(ReloadError::Preflight)?;
        // 3. Publish: single atomic swap of the active handle.
        ctx.set(KEY_ACTIVE, next.clone());
        // 2 / 4. Activate + drain: retire the previously published handle.
        let retired = self.current.replace(next);
        let current = match &self.current {
            Some(c) => c.clone(),
            None => {
                return Err(ReloadError::Preflight(
                    "reloader did not retain the published handle".to_owned(),
                ));
            }
        };
        Ok(DrainOutcome { retired, current })
    }

    /// Detach the published handle. Runs *without* replaying any inverse:
    /// the session is intentionally being retired, which is not a rollback.
    pub fn clear(&mut self, ctx: &mut Context) {
        let retired = self.current.take();
        ctx.remove(KEY_ACTIVE);
        // Drain: release the retired handle; if it was the last owner, its
        // resources are disposed via their scopes' inverses.
        drop(retired);
    }
}

/// Result of an atomic session publish: the newly-active handle plus the
/// retired generation the caller may still be draining.
pub struct DrainOutcome {
    /// The handle replaced by this publish, if any. While a caller holds it,
    /// in-flight turns may finish; drop it to release it.
    pub retired: Option<SessionManagerHandle>,
    /// The newly-active session handle.
    pub current: SessionManagerHandle,
}

#[cfg(test)]
mod tests {
    // Tests exercise deny-linted macros (unwrap!/panic!) to assert failure
    // behavior; opt out per the kernel's established test-module pattern.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::SessionContext;

    fn user_msg(text: &str) -> serde_json::Value {
        serde_json::json!({
            "role": "user", "content": text, "timestamp": crate::time::now_ms()
        })
    }

    /// Canon temporal check: snapshot, mount a session scope, exercise the
    /// live handle, unmount — the context returns to its pre-mount state
    /// with no residue.
    #[test]
    fn mount_unmount_leaves_no_residue() {
        let mut ctx = Context::new();
        assert!(!ctx.has(KEY_ACTIVE), "no active session before mount");

        let manager = Arc::new(Mutex::new(super::super::SessionManager::in_memory()));
        let scope = SessionScope::new(manager.clone(), vec![KEY_LEAF, KEY_CONTEXT]);
        let id = ctx.mount(scope.into_component());
        assert!(ctx.has(KEY_ACTIVE), "active session committed at mount");

        // Exercise the live handle while mounted.
        manager
            .lock()
            .unwrap()
            .append_message(user_msg("hi"))
            .unwrap();
        manager
            .lock()
            .unwrap()
            .append_message(user_msg("yo"))
            .unwrap();

        ctx.unmount(id);

        assert!(
            !ctx.has(KEY_ACTIVE),
            "active session removed on unmount: no residue"
        );
        assert!(
            ctx.get::<SessionManagerHandle>(KEY_ACTIVE).is_none(),
            "no leaked manager handle"
        );
    }

    /// Canon spatial check: change a declared key → exactly its readers
    /// react; change an undeclared key → none react.
    #[test]
    fn spatial_notifies_only_declared_readers() {
        let mut ctx = Context::new();
        let leaf_hits = Arc::new(AtomicUsize::new(0));
        let context_hits = Arc::new(AtomicUsize::new(0));

        let l = leaf_hits.clone();
        ctx.mount(
            Component::new(vec![KEY_LEAF])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_LEAF, "leaf reader got non-leaf key");
                    l.fetch_add(1, Ordering::Relaxed);
                }),
        );
        let c = context_hits.clone();
        ctx.mount(
            Component::new(vec![KEY_CONTEXT])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_CONTEXT, "context reader got non-context key");
                    c.fetch_add(1, Ordering::Relaxed);
                }),
        );

        // A session scope that reads neither key must not react.
        let manager = Arc::new(Mutex::new(super::super::SessionManager::in_memory()));
        let _sid = ctx.mount(SessionScope::new(manager, vec![]).into_component());
        let _lid = ctx.mount(Component::new(vec![KEY_LEAF]).effect(|_| Box::new(|_| {})));
        let _cid = ctx.mount(Component::new(vec![KEY_CONTEXT]).effect(|_| Box::new(|_| {})));

        // Undeclared key change → nothing reacts.
        ctx.set("session:unrelated", "x");
        assert_eq!(
            leaf_hits.load(Ordering::Relaxed),
            0,
            "leaf fired on unrelated"
        );
        assert_eq!(
            context_hits.load(Ordering::Relaxed),
            0,
            "context fired on unrelated"
        );

        // Declared key change → exactly that reader reacts.
        ctx.set(KEY_LEAF, "branch-1");
        assert_eq!(leaf_hits.load(Ordering::Relaxed), 1);
        assert_eq!(context_hits.load(Ordering::Relaxed), 0);

        ctx.set(
            KEY_CONTEXT,
            SessionContext {
                messages: Vec::new(),
                thinking_level: "off".to_owned(),
                model: None,
            },
        );
        assert_eq!(leaf_hits.load(Ordering::Relaxed), 1);
        assert_eq!(context_hits.load(Ordering::Relaxed), 1);
    }

    /// Atomic reload: build → activate → publish → drain. A build (preflight)
    /// failure leaves the old session untouched; publish is a single swap;
    /// drain releases the old handle only after the new one is live.
    #[test]
    fn atomic_reload_swap_and_preflight() {
        let mut ctx = Context::new();
        let mut reloader = SessionReloader::new();

        // Publish the "old" generation.
        let old_handle = Arc::new(Mutex::new(super::super::SessionManager::in_memory()));
        reloader
            .publish(&mut ctx, || Ok(old_handle.clone()))
            .unwrap();
        assert!(Arc::ptr_eq(reloader.current().unwrap(), &old_handle));
        assert!(
            Arc::ptr_eq(
                ctx.get::<SessionManagerHandle>(KEY_ACTIVE).unwrap(),
                &old_handle
            ),
            "old generation published"
        );

        // Preflight failure: build errors → old session untouched.
        let build_bomb: std::result::Result<SessionManagerHandle, String> = Err("boom".to_owned());
        let err = reloader.publish(&mut ctx, || build_bomb);
        assert!(err.is_err(), "preflight failure propagates");
        assert!(
            Arc::ptr_eq(
                ctx.get::<SessionManagerHandle>(KEY_ACTIVE).unwrap(),
                &old_handle
            ),
            "old session untouched after preflight failure"
        );
        assert!(Arc::ptr_eq(reloader.current().unwrap(), &old_handle));

        // Publish the new generation: single swap.
        let new_handle = Arc::new(Mutex::new(super::super::SessionManager::in_memory()));
        new_handle
            .lock()
            .unwrap()
            .append_message(user_msg("world"))
            .unwrap();
        let new_writer = new_handle.clone();
        reloader.publish(&mut ctx, move || Ok(new_writer)).unwrap();
        assert!(
            Arc::ptr_eq(
                ctx.get::<SessionManagerHandle>(KEY_ACTIVE).unwrap(),
                &new_handle
            ),
            "publish swapped in the new generation"
        );
        assert!(Arc::ptr_eq(reloader.current().unwrap(), &new_handle));

        // Drain: the old handle is still reachable by the test until the new
        // one is published; once swapped, releasing `old_handle` frees it.
        drop(old_handle);
        assert!(
            Arc::ptr_eq(
                ctx.get::<SessionManagerHandle>(KEY_ACTIVE).unwrap(),
                &new_handle
            ),
            "new generation unaffected by draining the old"
        );
    }

    /// Reloader publish notifies exactly the declared `session:active`
    /// readers (spatial integration of the reload path).
    #[test]
    fn reload_publish_notifies_only_active_readers() {
        let mut ctx = Context::new();
        let mut reloader = SessionReloader::new();

        let active_hits = Arc::new(AtomicUsize::new(0));
        let leaf_hits = Arc::new(AtomicUsize::new(0));

        let a = active_hits.clone();
        ctx.mount(
            Component::new(vec![KEY_ACTIVE])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_ACTIVE);
                    a.fetch_add(1, Ordering::Relaxed);
                }),
        );
        let l = leaf_hits.clone();
        ctx.mount(
            Component::new(vec![KEY_LEAF])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_LEAF);
                    l.fetch_add(1, Ordering::Relaxed);
                }),
        );

        reloader
            .publish(&mut ctx, || {
                Ok(Arc::new(Mutex::new(
                    super::super::SessionManager::in_memory(),
                )))
            })
            .unwrap();

        assert_eq!(active_hits.load(Ordering::Relaxed), 1);
        assert_eq!(
            leaf_hits.load(Ordering::Relaxed),
            0,
            "leaf reader reacted to an active-session reload"
        );
    }
}
