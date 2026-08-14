//! Spatiotemporal host lifecycle for `pi-rs-host`, built on `pi-rs-kernel`.
//!
//! The kernel ([`pi_rs_kernel::Context`]) supplies the shared mechanism:
//! - **Spatial** — a mounted [`Scope`] declares the keys it reads; a committed
//!   [`HostLifecycle::set`] on a declared key fires that scope's reaction (and
//!   only those); an undeclared change fires nothing.
//! - **Temporal** — [`HostLifecycle::dispose`] / [`HostLifecycle::drain`]
//!   replay each scope's resource disposes, so the host returns to its
//!   pre-mount state with no residue.
//!
//! The kernel's `OnChange`/`Inverse` closures are tied to `&mut Context`, so
//! this host layer delegates the value store, the reader graph, and the
//! declared-key notification to the kernel, and keeps the host's *real*
//! resource disposes (killing a process tree, flushing and dropping a session,
//! shutting down a watcher, stopping the VM thread) in a parallel map keyed by
//! the same scope id. Within a unit disposes run in reverse registration order;
//! across units [`HostLifecycle::drain`] runs them in reverse topological
//! order. Each dispose runs at most once.
//!
//! Epoch/identity ([`Epoch`], [`Generation`]) implements the generation axis:
//! a session/settings generation is keyed by
//! `[key, provider id, version, schema hash]`, and only an epoch change forces
//! a dependent reload (DESIGN axiom 06: runtime/session resources have explicit
//! cancellation and disposal; doctrine 02: no live `&mut` host references — a
//! reaction sees only the changed key).

use std::collections::HashMap;

use pi_rs_kernel::Context as KernelContext;

/// A host resource's dispose — reversible, cancelable, or compensatable work
/// that undoes the resource's effect. Runs at most once.
pub type Dispose = Box<dyn FnOnce() + Send>;

/// Reaction to a committed change of a declared read key. A reaction is
/// passive — it receives only the key of what changed (immutable snapshot in),
/// matching doctrine 02: it never borrows mutable host state.
pub type OnChange = Box<dyn FnMut(&'static str) + Send>;

/// Stable identity of one mounted host unit (effect scope).
pub type ScopeId = usize;

/// One host-owned effect scope: the keys it reads (spatial) plus the live
/// resources it owns (temporal) and the reaction fired when a read key changes.
#[derive(Default)]
pub struct Scope {
    reads: Vec<&'static str>,
    on_change: Option<OnChange>,
    dispose: Vec<Dispose>,
}

impl Scope {
    /// Declare a context key this unit reads. Change it → `on_change` fires.
    #[must_use]
    pub fn with_read(mut self, key: &'static str) -> Self {
        self.reads.push(key);
        self
    }

    /// Register one live resource dispose. Disposes run in reverse registration
    /// order within the unit.
    pub fn track(mut self, dispose: Dispose) -> Self {
        self.dispose.push(dispose);
        self
    }

    /// Declare the keys this scope reads (spatial dependencies).
    #[must_use]
    pub fn reads(self, keys: impl IntoIterator<Item = &'static str>) -> Self {
        let mut s = self;
        for key in keys {
            s.reads.push(key);
        }
        s
    }

    /// Set the reaction fired when a declared read key changes.
    #[must_use]
    pub fn on_change(mut self, cb: impl FnMut(&'static str) + Send + 'static) -> Self {
        self.on_change = Some(Box::new(cb));
        self
    }
}

/// An epoch — the identity of a session/settings generation:
/// `[key, provider id, provider version, schema hash]`. A dependent reloads
/// only when the whole tuple changes ([`Generation`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Epoch {
    /// The reactive key this generation shadows.
    pub key: &'static str,
    /// Provider generating the value (e.g. a settings scope id or session id).
    pub provider: String,
    /// Provider version.
    pub version: String,
    /// Canonical schema hash of the value's shape.
    pub schema_hash: String,
}

impl Epoch {
    #[must_use]
    pub fn new(
        key: &'static str,
        provider: impl Into<String>,
        version: impl Into<String>,
        schema_hash: impl Into<String>,
    ) -> Self {
        Self {
            key,
            provider: provider.into(),
            version: version.into(),
            schema_hash: schema_hash.into(),
        }
    }
}

/// A generation-gated value. Consumers reload only when the epoch changes.
#[derive(Debug)]
pub struct Generation<T> {
    pub value: T,
    pub epoch: Epoch,
}

impl<T> Generation<T> {
    /// True when `epoch` differs from this generation's — only then is a
    /// dependent reload forced.
    #[must_use]
    pub fn reloads_on(&self, epoch: &Epoch) -> bool {
        self.epoch != *epoch
    }
}

/// Spatiotemporal controller for the host's live resources, on top of the
/// kernel's [`KernelContext`].
///
/// The kernel owns the value store ([`KernelContext::set`]), the reader graph,
/// and the declared-key notification; this controller owns the per-scope
/// resource disposes and the reverse-topological drain order.
#[derive(Default)]
pub struct HostLifecycle {
    kernel: KernelContext,
    disposes: HashMap<ScopeId, Vec<Dispose>>,
    deps: HashMap<ScopeId, Vec<ScopeId>>,
    order: Vec<ScopeId>,
}

impl HostLifecycle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mount a host unit: declare the keys it reads (spatial, via the kernel)
    /// and register its resource disposes (temporal). Returns the scope id.
    pub fn mount(&mut self, scope: Scope) -> ScopeId {
        let mut component = pi_rs_kernel::Component::new(scope.reads);
        if let Some(mut cb) = scope.on_change {
            component = component.on_change(move |_ctx, key| cb(key));
        }
        let id = self.kernel.mount(component);
        self.disposes.insert(id, scope.dispose);
        self.order.push(id);
        id
    }

    /// Declare that `scope` depends on `dep`: drain disposes `dep` before `scope`.
    pub fn depends_on(&mut self, scope: ScopeId, dep: ScopeId) {
        self.deps.entry(scope).or_default().push(dep);
    }

    /// Read a committed value by key (typed; `T` must match the writer).
    #[must_use]
    pub fn get<T: std::any::Any + Send>(&self, key: &'static str) -> Option<&T> {
        self.kernel.get::<T>(key)
    }

    /// True if a value exists for `key`.
    #[must_use]
    pub fn has(&self, key: &'static str) -> bool {
        self.kernel.has(key)
    }

    /// The single committed write path (kernel semantics): stores the value,
    /// then notifies **only** the scopes that declared `key` in `reads`.
    pub fn set<T: std::any::Any + Send>(&mut self, key: &'static str, value: T) {
        self.kernel.set(key, value);
    }

    /// Dispose one scope: run its resource disposes in reverse registration
    /// order, then unregister its reads. At most once per scope.
    pub fn dispose(&mut self, id: ScopeId) {
        if let Some(mut d) = self.disposes.remove(&id) {
            self.order.retain(|&i| i != id);
            self.kernel.unmount(id);
            for dispose in d.drain(..).rev() {
                dispose();
            }
        }
    }

    /// Drain every mounted scope in reverse topological order (a scope's
    /// declared dependencies first), falling back to reverse registration
    /// order. After a drain no host resource remains mounted. Safe to call
    /// repeatedly; each dispose runs at most once.
    pub fn drain(&mut self) {
        let mut order_out = Vec::with_capacity(self.order.len());
        let mut remaining: std::collections::HashSet<ScopeId> =
            self.order.iter().copied().collect();
        while !remaining.is_empty() {
            let mut progressed = false;
            for &id in &self.order {
                if !remaining.contains(&id) {
                    continue;
                }
                let deps = self.deps.get(&id).cloned().unwrap_or_default();
                if deps.iter().all(|d| !remaining.contains(d)) {
                    order_out.push(id);
                    remaining.remove(&id);
                    progressed = true;
                }
            }
            if !progressed {
                // Cycle (a strict graph would hard-error): drain the remainder
                // in reverse registration order so each dispose still runs once.
                for &id in self.order.iter().rev() {
                    if remaining.contains(&id) {
                        order_out.push(id);
                    }
                }
                remaining.clear();
            }
        }
        for id in order_out {
            self.dispose(id);
        }
    }

    /// Number of live mounted scopes (0 after a full drain). Pre-image (before
    /// mount) plus live disposes — a true residue check also asserts
    /// [`Self::is_empty`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.disposes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.disposes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    // Tests exercise the deny-lints' forbidden macros with real panics.
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const THEME: &str = "theme";
    const MODE: &str = "mode";
    const MODEL: &str = "model";
    const UNDECLARED: &str = "undeclared";

    /// Temporal: snapshot → mount resources (vm/process/session) → dispose →
    /// diff empty (no live scope residue).
    #[test]
    fn drain_leaves_no_residue() {
        let mut lc = HostLifecycle::new();
        lc.set(THEME, "dark");

        let disposed = Arc::new(Mutex::new(Vec::new()));
        let d = Arc::clone(&disposed);
        let d2 = Arc::clone(&disposed);
        let d3 = Arc::clone(&disposed);
        lc.mount(
            Scope::default()
                .track(Box::new(move || d.lock().unwrap().push("vm")))
                .track(Box::new(move || d2.lock().unwrap().push("process")))
                .track(Box::new(move || d3.lock().unwrap().push("session"))),
        );
        lc.drain();
        // Reverse registration order within the unit.
        assert_eq!(disposed.lock().unwrap()[..], ["session", "process", "vm"]);
        assert!(lc.is_empty(), "residue after drain");
        assert_eq!(lc.get::<&str>(THEME), Some(&"dark"));
    }

    /// Temporal: dispose runs at most once, and repeated dispose/drain are no-ops.
    #[test]
    fn dispose_runs_at_most_once() {
        let mut lc = HostLifecycle::new();
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let scope = lc.mount(Scope::default().track(Box::new(move || {
            c.fetch_add(1, Ordering::Relaxed);
        })));
        lc.dispose(scope);
        lc.dispose(scope); // already removed -> no-op
        lc.drain(); // nothing left
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    /// Temporal: dependencies dispose before dependents (reverse topological).
    #[test]
    fn drain_respects_topological_order() {
        let mut lc = HostLifecycle::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        let b = Arc::clone(&order);
        let base =
            lc.mount(Scope::default().track(Box::new(move || b.lock().unwrap().push("base"))));
        let o = Arc::clone(&order);
        let unit =
            lc.mount(Scope::default().track(Box::new(move || o.lock().unwrap().push("unit"))));
        lc.depends_on(unit, base);
        lc.drain();
        assert_eq!(order.lock().unwrap()[..], ["base", "unit"]);
    }

    /// Spatial: change a declared key → exactly its readers react; undeclared
    /// → none.
    #[test]
    fn set_notifies_only_declared_readers() {
        let mut lc = HostLifecycle::new();
        let theme_hits = Arc::new(AtomicUsize::new(0));
        let mode_hits = Arc::new(AtomicUsize::new(0));

        let th = Arc::clone(&theme_hits);
        lc.mount(
            Scope::default()
                .on_change(move |k| {
                    assert_eq!(k, THEME, "theme reader got non-theme key");
                    th.fetch_add(1, Ordering::Relaxed);
                })
                .with_read(THEME),
        );
        let mh = Arc::clone(&mode_hits);
        lc.mount(
            Scope::default()
                .on_change(move |k| {
                    assert_eq!(k, MODE, "mode reader got non-mode key");
                    mh.fetch_add(1, Ordering::Relaxed);
                })
                .with_read(MODE),
        );

        lc.set(MODE, "command");
        assert_eq!(theme_hits.load(Ordering::Relaxed), 0);
        assert_eq!(mode_hits.load(Ordering::Relaxed), 1);

        lc.set(THEME, "light");
        assert_eq!(theme_hits.load(Ordering::Relaxed), 1);
        assert_eq!(mode_hits.load(Ordering::Relaxed), 1);

        // Undeclared changes never fire anything:
        lc.set(UNDECLARED, 1);
        assert_eq!(theme_hits.load(Ordering::Relaxed), 1);
        assert_eq!(mode_hits.load(Ordering::Relaxed), 1);
    }

    /// A scope is removed on dispose, so its future reaction is silent.
    #[test]
    fn disposed_scope_stops_reacting() {
        let mut lc = HostLifecycle::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let scope = lc.mount(Scope::default().with_read(MODEL).on_change(move |_| {
            h.fetch_add(1, Ordering::Relaxed);
        }));
        lc.set(MODEL, "v1");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        lc.dispose(scope);
        lc.set(MODEL, "v2");
        assert_eq!(hits.load(Ordering::Relaxed), 1, "disposed scope raced");
    }

    /// Generation/epoch: only an epoch change forces a reload.
    #[test]
    fn epoch_gates_reload() {
        let e1 = Epoch::new(MODEL, "app", "1.0", "sha-abc");
        let same = Epoch::new(MODEL, "app", "1.0", "sha-abc");
        let version_bump = Epoch::new(MODEL, "app", "1.1", "sha-abc");
        let schema_bump = Epoch::new(MODEL, "app", "1.0", "sha-def");

        let generation = Generation {
            value: 7,
            epoch: e1,
        };
        assert!(!generation.reloads_on(&same), "same epoch must not reload");
        assert!(generation.reloads_on(&version_bump), "version bump reloads");
        assert!(
            generation.reloads_on(&schema_bump),
            "schema hash bump reloads"
        );

        // Committing the new generation replaces the old identity:
        let gen2 = Generation {
            value: 8,
            epoch: schema_bump.clone(),
        };
        assert!(!gen2.reloads_on(&schema_bump));
        assert!(gen2.reloads_on(&same));
    }
}
