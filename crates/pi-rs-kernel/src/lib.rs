//! Spatiotemporal composability kernel.
//!
//! Zero-dependency leaf crate ported from the ekko reference kernel to
//! pi-rs conventions. Two axes, one mechanism:
//! - **Temporal** — mounting a [`Component`] runs its effects; each effect
//!   returns an [`Inverse`] closure that undoes it. [`Context::unmount`]
//!   replays those inverses in reverse order, so the context returns to its
//!   pre-mount state with no residue.
//! - **Spatial** — each component declares the context keys it *reads*
//!   ([`Component::reads`]). A committed `set` on a key notifies only the
//!   components that declared it.
//!
//! The [`Context`] is host-owned state; effects write it and declarations
//! name its keys. There is a single write path (`Context::set`), matching the
//! functional-core boundary: components must not hold `&mut` host state, they
//! operate on the context through the effect closures the host runs.
//!
//! Minimal by design: no scope tree, no config reconciliation, no HMR. A flat
//! map of mounted components is enough until a unit needs a child unit.

use std::any::Any;
use std::collections::{HashMap, HashSet};

/// Undoes one committed effect. Owns whatever it needs to restore the prior
/// context state (e.g. the previous value).
pub type Inverse = Box<dyn FnOnce(&mut Context) + Send>;

/// A context mutation plus its inverse: `apply` runs at mount, returns the
/// inverse `unmount` replays.
pub type Effect = Box<dyn Fn(&mut Context) -> Inverse + Send>;

/// A declared reaction to a changed dependency.
pub type OnChange = Box<dyn FnMut(&mut Context, &'static str) + Send>;

/// A composition unit: what it reads (spatial), and what it commits when
/// mounted (temporal). Each effect's inverse is replayed on unmount.
pub struct Component {
    /// Context keys this component reads. Change one of these → this
    /// component's `on_change` fires (and only these keys trigger it).
    pub reads: Vec<&'static str>,
    /// Effects applied in order at mount; each returns its inverse.
    pub effects: Vec<Effect>,
    /// Runs when a declared read key changes. Called with the changed key.
    pub on_change: Option<OnChange>,
}

impl Component {
    pub fn new(reads: Vec<&'static str>) -> Self {
        Self {
            reads,
            effects: Vec::new(),
            on_change: None,
        }
    }

    /// Add an effect. `apply` receives the context and must return an
    /// [`Inverse`] closure that restores what it changed.
    pub fn effect(mut self, apply: impl Fn(&mut Context) -> Inverse + Send + 'static) -> Self {
        self.effects.push(Box::new(apply));
        self
    }

    /// Declare a reaction to a read-key change.
    pub fn on_change(
        mut self,
        cb: impl FnMut(&mut Context, &'static str) + Send + 'static,
    ) -> Self {
        self.on_change = Some(Box::new(cb));
        self
    }
}

struct ScopeInner {
    reads: Vec<&'static str>,
    inverses: Vec<Inverse>,
    on_change: Option<OnChange>,
}

/// Host-owned keyed state. The single write path is [`Context::set`]; reads
/// go through [`Context::get`]. Mount/unmount exercises the spatiotemporal
/// axes; a `set` on a declared key notifies exactly its readers.
#[derive(Default)]
pub struct Context {
    values: HashMap<&'static str, Box<dyn Any + Send>>,
    readers: HashMap<&'static str, HashSet<usize>>,
    scopes: HashMap<usize, ScopeInner>,
    next_scope: usize,
    // ponytail: reentrancy guard. A set inside a notification is dropped,
    // not queued. If nested notifications become correct-or-bust, queue +
    // flush.
    notifying: bool,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a value by key. `T` must match what the writer stored.
    pub fn get<T: Any + Send>(&self, key: &'static str) -> Option<&T> {
        self.values.get(key)?.downcast_ref::<T>()
    }

    /// True if a value exists for `key` (any type).
    pub fn has(&self, key: &'static str) -> bool {
        self.values.contains_key(key)
    }

    /// The single committed write path. Stores the value and notifies only
    /// the components that declared `key` in their `reads`.
    pub fn set<T: Any + Send>(&mut self, key: &'static str, value: T) {
        self.values.insert(key, Box::new(value));
        self.notify(key);
    }

    /// Remove a value. Also notifies readers.
    pub fn remove(&mut self, key: &'static str) {
        if self.values.remove(&key).is_some() {
            self.notify(key);
        }
    }

    /// Mount a component: apply its effects, record their inverses, register
    /// its read keys and change reaction. Returns a handle for `unmount`.
    pub fn mount(&mut self, component: Component) -> usize {
        let id = self.next_scope;
        self.next_scope += 1;

        for key in &component.reads {
            self.readers.entry(*key).or_default().insert(id);
        }

        let mut inverses = Vec::with_capacity(component.effects.len());
        for effect in component.effects {
            inverses.push(effect(self));
        }

        self.scopes.insert(
            id,
            ScopeInner {
                reads: component.reads,
                inverses,
                on_change: component.on_change,
            },
        );
        id
    }

    /// Unmount: replay every recorded inverse in reverse order, then
    /// unregister reads. Context returns to pre-mount state.
    pub fn unmount(&mut self, id: usize) {
        let Some(inner) = self.scopes.remove(&id) else {
            return;
        };
        for key in &inner.reads {
            if let Some(set) = self.readers.get_mut(key) {
                set.remove(&id);
            }
        }
        for inverse in inner.inverses.into_iter().rev() {
            inverse(self);
        }
    }

    fn notify(&mut self, key: &'static str) {
        if self.notifying {
            return;
        }
        self.notifying = true;
        let targets: Vec<usize> = self
            .readers
            .get(key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        for id in targets {
            // Move the callback out so it may mutate the context. If the
            // scope vanished between scheduling and delivery, skip it.
            let Some(cb) = self.scopes.get_mut(&id).and_then(|s| s.on_change.take()) else {
                continue;
            };
            let mut cb = cb;
            (cb)(self, key);
            if let Some(inner) = self.scopes.get_mut(&id) {
                inner.on_change = Some(cb);
            }
        }
        self.notifying = false;
    }
}

#[cfg(test)]
mod tests {
    // Tests exercise the deny-lints' forbidden macros (assert!/unwrap!) with
    // real panics, so this test module opts out of the crate-wide deny.
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const THEME: &str = "theme";
    const MODE: &str = "mode";
    const BACKGROUND: &str = "background";

    /// Canon temporal check: snapshot, mount, exercise every effect,
    /// unmount, diff — must be empty.
    #[test]
    fn unmount_reverts_every_effect() {
        let mut ctx = Context::new();
        ctx.set(THEME, "dark");

        let snapshot = ctx.values.len();

        let id = ctx.mount(
            Component::new(vec![])
                .effect(|c| {
                    let old = c.get::<&str>(MODE).copied();
                    c.set(MODE, "command");
                    Box::new(move |c| match old {
                        Some(prev) => c.set(MODE, prev),
                        None => c.remove(MODE),
                    })
                })
                .effect(|c| {
                    c.set(BACKGROUND, "#000");
                    Box::new(move |c| {
                        c.remove(BACKGROUND);
                    })
                }),
        );

        assert!(ctx.has(MODE));
        assert!(ctx.has(BACKGROUND));

        ctx.unmount(id);

        assert_eq!(ctx.values.len(), snapshot, "residue after unmount");
        assert!(!ctx.has(MODE));
        assert!(!ctx.has(BACKGROUND));
        assert_eq!(ctx.get::<&str>(THEME), Some(&"dark"));
    }

    /// Canon spatial check: change each declared key, confirm exactly its
    /// readers react; undeclared key changes must not.
    #[test]
    fn spatial_notifies_only_declared_readers() {
        let mut ctx = Context::new();

        let theme_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mode_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let t = theme_hits.clone();
        ctx.mount(
            Component::new(vec![THEME])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, THEME, "theme reader got non-theme key");
                    t.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }),
        );
        let m = mode_hits.clone();
        ctx.mount(
            Component::new(vec![MODE])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, MODE, "mode reader got non-mode key");
                    m.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }),
        );

        ctx.set(MODE, "command");
        assert_eq!(
            theme_hits.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "theme reader fired on mode change"
        );
        assert_eq!(mode_hits.load(std::sync::atomic::Ordering::Relaxed), 1);

        ctx.set(THEME, "light");
        assert_eq!(theme_hits.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            mode_hits.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "mode reader fired on theme change"
        );
    }

    /// Two mounts with the same resource must not collide: the second's
    /// inverse and the first's are independent.
    #[test]
    fn overlapping_effects_unmount_independently() {
        let mut ctx = Context::new();
        let a = ctx.mount(Component::new(vec![]).effect(|c| {
            let old = c.get::<&str>(MODE).copied();
            c.set(MODE, "a");
            Box::new(move |c| match old {
                Some(p) => c.set(MODE, p),
                None => c.remove(MODE),
            })
        }));
        let b = ctx.mount(Component::new(vec![]).effect(|c| {
            let old = c.get::<&str>(MODE).copied();
            c.set(MODE, "b");
            Box::new(move |c| match old {
                Some(p) => c.set(MODE, p),
                None => c.remove(MODE),
            })
        }));

        assert_eq!(ctx.get::<&str>(MODE), Some(&"b"));
        ctx.unmount(b);
        assert_eq!(ctx.get::<&str>(MODE), Some(&"a"));
        ctx.unmount(a);
        assert!(!ctx.has(MODE));
    }
}
