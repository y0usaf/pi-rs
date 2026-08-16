//! The daemon/client boundary for pi-rs, named here so a future process split
//! is a **boundary move, not a rewrite**.
//!
//! ## What the boundary is
//!
//! [`DaemonBoundary`] owns one [`pi_rs_kernel::Context`] — the *host-owned keyed
//! state* the DESIGN "Spatiotemporal composability" decision names as the
//! composition unit. Everything that must **outlive a viewer** (the Lua VM
//! thread, process trees, the active session manager handle, agent resources) is
//! committed into this Context by mounting a kernel [`Component`]; every mount
//! returns an [`Inverse`] that restores pre-mount state, so an unmount leaves no
//! residue.
//!
//! The rule that makes the future daemon/client split trivial:
//! - **The daemon side** owns the [`DaemonBoundary`] (and thus the kernel
//!   Context). It creates the VM, mounts the host lifecycle, the session
//!   manager, and any long-lived agent resource as kernel components.
//! - **A client / viewer** never owns host state. It *attaches* by mounting a
//!   read-scope [`Component`] that declares the keys it reads (spatial axis) and,
//!   when done, *detaches* by unmounting that same scope. Rendered cells come
//!   from the committed context, so a client can attach, render, and detach
//!   without ever holding a `&mut` to the host.
//!
//! Splitting the process later is then: keep the [`DaemonBoundary`] in the
//! daemon, hand each client a way to mount read-scopes over the same kernel
//! semantics, and move the transport — the lifecycle, triage of in/out, and
//! the residue guarantee are unchanged.
//!
//! ## The three axes, wired to the real host
//!
//! - **Temporal** — [`DaemonBoundary::mount_host`] mounts a component whose
//!   effect records the live VM and whose inverse calls [`Host::stop`]; mounting
//!   the session commits the real [`SessionManager`] handle and its inverse
//!   detaches it. [`DaemonBoundary::drain`] replays inverses in reverse
//!   registration order, so an unmount returns the context to its pre-mount
//!   state with no residue.
//! - **Spatial** — [`DaemonBoundary::attach`] mounts a client/viewer scope that
//!   declares the keys it reads; a committed `set` on a declared key notifies
//!   exactly that scope (and no others). A client that renders from a key
//!   re-renders *only* when that key changes — the parity-safe invalidation
//!   path the TUI lifecycle already uses.
//! - **Atomic reload** — [`DaemonBoundary`] mounts new generations (host or
//!   session) via the kernel's single write path on the same Context, so a
//!   reload is `build → activate → publish → drain`, not a rollback.

use pi_rs_kernel::{Component, Context};
use pi_rs_session::composable::{SessionScope, SessionManagerHandle, KEY_ACTIVE};

use crate::Host;

/// Context key under which the host VM's liveness is committed. Mounting the
/// host sets it; unmounting (its inverse) removes it.
pub const KEY_HOST_VM: &str = "daemon:host:vm";
/// Context key a client/viewer may read to observe the active session handle
/// (re-exported from the session scope module for a single naming surface).
pub const KEY_SESSION: &str = KEY_ACTIVE;

/// The host-owned spatiotemporal boundary. Owns the kernel [`Context`]; hosts
/// and sessions are mounted as [`Component`]s so unmount/drain leaves no
/// residue, and clients attach/detach read-scopes over the same Context.
#[derive(Default)]
pub struct DaemonBoundary {
    ctx: Context,
    /// Mounted scope ids, in registration order, so `drain` replays inverses in
    /// reverse. The kernel tracks its own readers/values; this list only orders
    /// the drain (and lets `drain` be idempotent).
    scopes: Vec<usize>,
}

impl DaemonBoundary {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mount the real host VM as a kernel component. The effect commits
    /// [`KEY_HOST_VM`]; the returned inverse stops the VM thread (`Host::stop`)
    /// and removes the key, so `unmount` tears the host lifecycle down with no
    /// residue.
    pub fn mount_host(&mut self, host: &Host) -> usize {
        let host = host.clone();
        self.mount(
            Component::new(vec![])
                .effect(move |ctx| {
                    ctx.set(KEY_HOST_VM, ());
                    // The outer closure is `Fn` (mount may invoke the effect
                    // more than once), but each invocation must return its own
                    // `FnOnce` inverse owning its own host clone.
                    let host = host.clone();
                    Box::new(move |ctx| {
                        // Deterministic teardown: stop the VM thread, drop its
                        // Lua state/runtime, then remove the liveness key.
                        let _ = host.stop();
                        ctx.remove(KEY_HOST_VM);
                    })
                })
        )
    }

    /// Mount the real session manager handle as a kernel component. The session
    /// scope commits [`KEY_SESSION`]; its inverse detaches the handle on
    /// unmount, so a mounted session leaves no residue in the context.
    pub fn mount_session(&mut self, handle: SessionManagerHandle) -> usize {
        let scope = SessionScope::new(handle, vec![]).into_component();
        self.mount(scope)
    }

    /// Mount a generic kernel component (host-owned resource, watcher, timer…).
    /// Effects run at mount and return inverses replayed on unmount.
    pub fn mount(&mut self, component: Component) -> usize {
        let id = self.ctx.mount(component);
        self.scopes.push(id);
        id
    }

    /// A client/viewer attaches by mounting a read-scope component declaring the
    /// keys it renders from. Unmounting that scope detaches the viewer: its read
    /// keys are unregistered and no host state is touched.
    pub fn attach(&mut self, reads: Vec<&'static str>) -> usize {
        self.mount(Component::new(reads))
    }

    /// Attach a client viewer that reacts to a change on one of its read keys
    /// (spatial axis). `on_change` receives the changed key.
    pub fn attach_with(
        &mut self,
        reads: Vec<&'static str>,
        mut on_change: impl FnMut(&str) + Send + 'static,
    ) -> usize {
        let component = Component::new(reads).on_change(move |_ctx, key| on_change(key));
        self.mount(component)
    }

    /// Unmount one mounted scope by id: replay its inverses in reverse order and
    /// unregister its reads. The context returns to its pre-mount state.
    pub fn unmount(&mut self, id: usize) {
        self.ctx.unmount(id);
        self.scopes.retain(|&existing| existing != id);
    }

    /// Drain every mounted scope in reverse registration order. After this
    /// returns the context holds only pre-mount values — no host VM, no session
    /// handle, no mounted client scope survives. Idempotent.
    pub fn drain(&mut self) {
        while let Some(id) = self.scopes.pop() {
            self.ctx.unmount(id);
        }
    }

    /// True if any scope is currently mounted (0 after a full drain).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    /// Commit a value under `key` (kernel single write path). Only components
    /// that declared `key` as a read are notified.
    pub fn set<T: std::any::Any + Send>(&mut self, key: &'static str, value: T) {
        self.ctx.set(key, value);
    }

    /// True if `key` currently has a committed value (`None` after its owning
    /// mount's inverse removed it). Used to prove unmount leaves no residue.
    #[must_use]
    pub fn has(&self, key: &'static str) -> bool {
        self.ctx.has(key)
    }

    /// Read a committed value by key (typed; `T` must match the writer).
    #[must_use]
    pub fn get<T: std::any::Any + Send>(&self, key: &'static str) -> Option<&T> {
        self.ctx.get::<T>(key)
    }
}

#[cfg(test)]
mod tests {
    // Deny-lints forbid assert!/unwrap! in library code; this test module
    // exercises them against real panic paths, so it opts out per the kernel's
    // established test-module pattern.
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::HostError;
    use super::*;

    /// Canon runnable residue check against the **real host**: snapshot the
    /// boundary, mount a live VM + a live session handle, exercise every effect
    /// (load a chunk, open a session, commit a value), then unmount/drain —
    /// the diff must be empty (no VM liveness key, no session handle, no client
    /// scope, no leaked value).
    #[test]
    fn mount_exercise_unmount_leaves_no_residue() {
        let mut b = DaemonBoundary::new();
        assert!(b.is_empty(), "pre-mount must be empty");

        // Mount the real host VM.
        let host = Host::new(crate::HostConfig::default()).unwrap();
        let host_scope = b.mount_host(&host);
        assert!(b.has(KEY_HOST_VM), "host liveness not committed");

        // Exercise the VM while mounted: register and emit through a live chunk.
        host.load(
            "daemon-test://ping",
            "local pi = ... pi.on('ping', function() return { pong = true } end)",
        )
        .unwrap();
        let outcome = host.emit("ping", &serde_json::json!({})).unwrap();
        assert_eq!(
            outcome[0].result.as_ref().unwrap().as_ref().unwrap()["pong"],
            serde_json::json!(true)
        );

        // Mount the session manager as host-owned state.
        let manager = Arc::new(Mutex::new(pi_rs_session::SessionManager::in_memory()));
        let session_scope = b.mount_session(manager.clone());
        assert!(b.has(KEY_SESSION), "session handle not committed");
        manager
            .lock()
            .unwrap()
            .append_message(serde_json::json!({
                "role": "user",
                "content": "hi",
                "timestamp": pi_rs_session::time::now_ms(),
            }))
            .unwrap();

        // A client viewer attaches over the same context and reacts spatially.
        let viewer_hits = Arc::new(AtomicUsize::new(0));
        let hits = Arc::clone(&viewer_hits);
        let viewer_scope =
            b.attach_with(vec![KEY_SESSION], move |_k| {
                hits.fetch_add(1, Ordering::Relaxed);
            });
        b.set(KEY_SESSION, b.get::<SessionManagerHandle>(KEY_SESSION).unwrap().clone());
        assert_eq!(
            viewer_hits.load(Ordering::Relaxed),
            1,
            "attached viewer must react to a committed session set"
        );

        // Unmount every mount in reverse order and confirm the diff is empty.
        b.unmount(viewer_scope);
        b.unmount(session_scope);
        b.unmount(host_scope);
        assert!(b.is_empty(), "residue: scope still mounted");
        assert!(!b.has(KEY_HOST_VM), "residue: host liveness key survives");
        assert!(!b.has(KEY_SESSION), "residue: session handle survives");

        // The host VM thread was stopped by the unmount inverse: a further call
        // reports VmUnavailable.
        assert!(matches!(
            host.emit("ping", &serde_json::json!({})),
            Err(HostError::VmUnavailable)
        ));
    }

    /// `drain` unmounts every scope in reverse registration order and is
    /// idempotent; after it the context holds only pre-mount values.
    #[test]
    fn drain_replays_reverse_and_is_idempotent() {
        let mut b = DaemonBoundary::new();
        // Pre-mount committed value that must survive mounts/unmounts.
        b.set("daemon:pre", 7);
        assert!(b.has("daemon:pre"));

        let s1 = b.attach(vec![]);
        let s2 = b.attach(vec![]);
        b.drain();
        assert!(b.is_empty());

        // Pre-mount value survives; mount-committed keys are gone.
        assert!(b.has("daemon:pre"));
        assert!(!b.has(KEY_HOST_VM));
        assert!(!b.has(KEY_SESSION));

        // Drain again is a safe no-op.
        b.drain();
        assert!(b.is_empty());

        let _ = s1;
        let _ = s2;
    }
}
