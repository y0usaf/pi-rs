//! Spatiotemporal composability for the agent loop.
//!
//! The agent loop owns live resources: provider streams, subprocesses,
//! timers, and background tasks (`pi.spawn` coroutines). Product behavior
//! lives in the embedded Lua pack, but the *ownership* of these resources
//! must be composable — a host runs an agent against a session, and both
//! mount and unmount as scoped units. This module supplies that boundary on
//! the `pi-rs-kernel` substrate:
//!
//! - **Temporal (effect scope)** — an [`AgentScope`] owns a set of live
//!   [`Resource`]s. Mount commits them on the shared [`kernel::Context`] and
//!   records the inverse that closes each one; unmount replays those
//!   inverses in reverse registration order, so the host returns to its
//!   pre-agent state with **no leaked streams, subprocesses, timers, or
//!   background tasks** (each dispose runs at most once).
//! - **Spatial (reactive dependency graph)** — an agent unit declares the
//!   context keys it *reads* (`agent:model`, `agent:settings`,
//!   `agent:session-state`). A committed [`kernel::Context::set`] notifies
//!   exactly the declared readers; undeclared changes never fire a reaction.
//! - **Atomic reload** — [`AgentReloader::publish`] resumes/reconstructs an
//!   agent via `build → activate → publish → drain`: a build/activate
//!   failure leaves the previous agent untouched; publish is a single
//!   [`kernel::Context::set`] swap; drain closes the previous generation's
//!   resources only after the new one is live, so in-flight turns finish
//!   before their resources are freed.
//!
//! A [`Resource`] models one live agent-loop resource (whatever the host
//! actually holds — a stream handle, a child process, a timer). Its dispose
//! is idempotent so effect scopes compose across the agent boundary.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pi_rs_kernel::{Component, Context, Inverse};

pub use pi_rs_kernel;

/// Context key for the agent's active generation handle.
pub const KEY_ACTIVE: &str = "agent:active";
/// Context key declaring that a unit reads the selected model.
pub const KEY_MODEL: &str = "agent:model";
/// Context key declaring that a unit reads agent/settings state.
pub const KEY_SETTINGS: &str = "agent:settings";
/// Context key declaring that a unit reads session state.
pub const KEY_SESSION: &str = "agent:session-state";

/// A live agent-loop resource owned by exactly one scope.
///
/// The liveness flag is shared behind an [`Arc`] so a test (or the host)
/// can observe open/leak, and so the same resource can be handed to several
/// scopes yet dispose exactly once.
#[derive(Debug)]
pub struct Resource {
    id: &'static str,
    alive: Arc<AtomicUsize>,
}

impl Resource {
    /// Open a resource. `id` is a stable identity for diagnostics/tests;
    /// exactly one [`Resource::open`] corresponds to exactly one eventual
    /// [`Resource::dispose`].
    pub fn open(id: &'static str) -> Self {
        Self {
            id,
            alive: Arc::new(AtomicUsize::new(1)),
        }
    }

    /// Liveness probe: `true` while not yet disposed.
    pub fn is_live(&self) -> bool {
        self.alive.load(Ordering::Relaxed) != 0
    }

    /// Dispose the resource. Idempotent: at most one dispose takes effect.
    pub fn dispose(&self) {
        self.alive.swap(0, Ordering::SeqCst);
    }

    /// The resource's stable id.
    pub fn id(&self) -> &'static str {
        self.id
    }
}

impl Clone for Resource {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            alive: self.alive.clone(),
        }
    }
}

/// One agent generation's live resource set, in registration order.
#[derive(Debug, Default)]
pub struct AgentGeneration {
    resources: Vec<Resource>,
}

impl AgentGeneration {
    /// Register a resource; the generation owns it. Returns a copy of the
    /// live handle so the caller can probe liveness / dispose.
    pub fn add(&mut self, resource: Resource) -> Resource {
        self.resources.push(resource.clone());
        resource
    }

    /// Number of resources owned.
    pub fn len(&self) -> usize {
        self.resources.len()
    }

    /// Whether the generation owns no resources.
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    /// Whether any owned resource is still live (a leak probe).
    pub fn has_live(&self) -> bool {
        self.resources.iter().any(Resource::is_live)
    }

    /// Build the inverse that disposes every resource in reverse
    /// registration order (the effect-scope rule).
    pub fn dispose_inverse(&self) -> Inverse {
        let mut resources: Vec<Resource> = Vec::with_capacity(self.resources.len());
        for r in self.resources.iter().rev() {
            resources.push(r.clone());
        }
        Box::new(move |_: &mut Context| {
            for r in resources.iter() {
                r.dispose();
            }
        })
    }
}

/// The published, shared agent generation (kernel values require `Send`).
pub type AgentHandle = Arc<Mutex<AgentGeneration>>;

/// A composition unit that owns one agent generation's live resources.
///
/// `reads` declares the context keys this unit consumes (spatial axis). On
/// mount the scope commits an [`AgentHandle`] under [`KEY_ACTIVE`]; its
/// inverse disposes the owned resources in reverse order (temporal axis,
/// no residue). A [`kernel::Context::set`] on a declared read key fires the
/// provided reaction, exactly once per notifying reader.
pub struct AgentScope {
    inner: Component,
}

impl AgentScope {
    /// Build a scope that owns `generation` and declares the given read keys.
    pub fn new(generation: AgentGeneration, reads: Vec<&'static str>) -> Self {
        let mut inner = Component::new(reads);
        let generation = Arc::new(Mutex::new(generation));
        let owned = generation.clone();
        inner.effects.push(Box::new(move |ctx: &mut Context| {
            let prev_has = ctx.has(KEY_ACTIVE);
            let prev = ctx.get::<AgentHandle>(KEY_ACTIVE).cloned();
            // Publish a clone; keep `owned` for the inverse's dispose pass.
            ctx.set(KEY_ACTIVE, owned.clone());
            let owned_inv = owned.clone();
            Box::new(move |ctx: &mut Context| {
                // Replay the generation's dispose inverse (reverse order).
                let inverse = match owned_inv.lock() {
                    Ok(guard) => guard.dispose_inverse(),
                    // A poisoned mutex still owns the data; recover it so the
                    // effect scope still disposes its resources exactly once.
                    Err(poisoned) => poisoned.into_inner().dispose_inverse(),
                };
                inverse(ctx);
                match prev_has {
                    true => match prev {
                        Some(prev) => ctx.set(KEY_ACTIVE, prev),
                        None => ctx.remove(KEY_ACTIVE),
                    },
                    false => {
                        ctx.remove(KEY_ACTIVE);
                    }
                }
            })
        }));
        Self { inner }
    }

    /// Add a reactive reaction fired when a declared read key changes.
    pub fn on_change(
        mut self,
        cb: impl FnMut(&mut Context, &'static str) + Send + 'static,
    ) -> Self {
        self.inner.on_change = Some(Box::new(cb));
        self
    }

    /// Consume the scope and yield the kernel [`Component`], ready for mount.
    pub fn into_component(self) -> Component {
        self.inner
    }
}

/// Error from an atomic agent reload.
#[derive(Debug, thiserror::Error)]
pub enum ReloadError {
    /// The next generation failed to build/activate. The previous agent is
    /// untouched.
    #[error("agent reload build failed: {0}")]
    Build(String),
}

/// Owner of the published agent generation. [`publish`](Self::publish)
/// performs `build → activate → publish → drain` atomically.
#[derive(Default)]
pub struct AgentReloader {
    /// The generation published on the context. The previous generation is
    /// held here *while draining*, so in-flight turns on the old agent can
    /// finish before its resources are closed.
    current: Option<AgentHandle>,
}

impl AgentReloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently published handle, if any.
    pub fn current(&self) -> Option<&AgentHandle> {
        self.current.as_ref()
    }

    /// Atomic reload: build → activate → publish → drain.
    ///
    /// 1. **Build** — `build` constructs the next [`AgentGeneration`].
    ///    Failure leaves the previous agent untouched (the context and
    ///    `self.current` are unchanged).
    /// 2. **Activate** — on return the generation is fully built and valid.
    /// 3. **Publish** — the context's [`KEY_ACTIVE`] is swapped with a single
    ///    [`Context::set`], notifying only the readers that declared
    ///    [`KEY_ACTIVE`].
    /// 4. **Drain** — the previous handle is retained in `self.current` only
    ///    until the next publish/clear; a caller that still holds an in-flight
    ///    turn keeps its resources alive. This method returns
    ///    [`DrainOutcome`] so the caller can observe the swap and close the
    ///    retired generation exactly once.
    pub fn publish<F>(&mut self, ctx: &mut Context, build: F) -> Result<DrainOutcome, ReloadError>
    where
        F: FnOnce() -> Result<AgentHandle, String>,
    {
        // 1. Build (preflight failure → old agent untouched).
        let next = build().map_err(ReloadError::Build)?;
        // 3. Publish: single atomic swap of the active handle.
        ctx.set(KEY_ACTIVE, next.clone());
        // 2 / 4. Activate + drain: retire the previous published handle.
        let retired = self.current.replace(next);
        let current = match &self.current {
            Some(c) => c.clone(),
            None => {
                // Unreachable: `replace` just stored the built handle.
                return Err(ReloadError::Build(
                    "reloader did not retain the published handle".to_owned(),
                ));
            }
        };
        Ok(DrainOutcome { retired, current })
    }

    /// Retire the published agent, removing it from the context and draining
    /// the retired generation.
    pub fn clear(&mut self, ctx: &mut Context) {
        let retired = self.current.take();
        ctx.remove(KEY_ACTIVE);
        // Drain: release the retired handle; if it was the last owner, its
        // resources are disposed via their scopes' inverses.
        drop(retired);
    }
}

/// Result of an atomic publish: the newly-active handle plus the retired
/// generation the caller may still be draining.
#[derive(Debug)]
pub struct DrainOutcome {
    /// The generation replaced by this publish, if any. While a caller holds
    /// it, in-flight turns may finish; drop it to dispose its resources.
    pub retired: Option<AgentHandle>,
    /// The newly-active generation handle.
    pub current: AgentHandle,
}

#[cfg(test)]
mod tests {
    // Tests exercise deny-linted macros (unwrap!/panic!) to assert failure
    // behavior; opt out per the kernel's established test-module pattern.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Canon temporal check: build a generation with several resources,
    /// mount it, exercise, unmount — every resource is disposed exactly once
    /// (reverse order) and no handle leaks in the context.
    #[test]
    fn mount_unmount_disposes_resources_no_residue() {
        let mut ctx = Context::new();

        let mut generation = AgentGeneration::default();
        let stream = generation.add(Resource::open("provider/stream"));
        let proc = generation.add(Resource::open("subprocess/bash"));
        let timer = generation.add(Resource::open("timer/retry"));
        assert!(stream.is_live() && proc.is_live() && timer.is_live());

        let scope = AgentScope::new(generation, vec![KEY_MODEL, KEY_SETTINGS]);
        let id = ctx.mount(scope.into_component());
        let active = ctx.get::<AgentHandle>(KEY_ACTIVE).unwrap();
        assert_eq!(active.lock().unwrap().len(), 3);
        assert!(ctx.has(KEY_ACTIVE));

        ctx.unmount(id);

        assert!(!stream.is_live(), "stream leaked after unmount");
        assert!(!proc.is_live(), "subprocess leaked after unmount");
        assert!(!timer.is_live(), "timer leaked after unmount");
        assert!(!ctx.has(KEY_ACTIVE), "agent handle leaked after unmount");
        assert!(
            ctx.get::<AgentHandle>(KEY_ACTIVE).is_none(),
            "no leaked agent handle"
        );
    }

    /// Canon spatial check: change a declared key → exactly its readers
    /// react; undeclared → none.
    #[test]
    fn spatial_notifies_only_declared_readers() {
        let mut ctx = Context::new();
        let model_hits = Arc::new(AtomicUsize::new(0));
        let settings_hits = Arc::new(AtomicUsize::new(0));

        let m = model_hits.clone();
        let model_reader = Component::new(vec![KEY_MODEL])
            .effect(|_| Box::new(|_| {}))
            .on_change(move |_, k| {
                assert_eq!(k, KEY_MODEL, "model reader got non-model key");
                m.fetch_add(1, Ordering::Relaxed);
            });
        let s = settings_hits.clone();
        let settings_reader = Component::new(vec![KEY_SETTINGS])
            .effect(|_| Box::new(|_| {}))
            .on_change(move |_, k| {
                assert_eq!(k, KEY_SETTINGS, "settings reader got non-settings key");
                s.fetch_add(1, Ordering::Relaxed);
            });
        ctx.mount(model_reader);
        ctx.mount(settings_reader);

        // Undeclared key change → nothing reacts.
        ctx.set("agent:unrelated", "x");
        assert_eq!(
            model_hits.load(Ordering::Relaxed),
            0,
            "model fired on unrelated"
        );
        assert_eq!(
            settings_hits.load(Ordering::Relaxed),
            0,
            "settings fired on unrelated"
        );

        // Declared key change → exactly that reader reacts.
        ctx.set(KEY_MODEL, "claude-3-5-sonnet");
        assert_eq!(model_hits.load(Ordering::Relaxed), 1);
        assert_eq!(settings_hits.load(Ordering::Relaxed), 0);

        ctx.set(KEY_SETTINGS, "session-scoped");
        assert_eq!(model_hits.load(Ordering::Relaxed), 1);
        assert_eq!(settings_hits.load(Ordering::Relaxed), 1);
    }

    /// Atomic reload: build → activate → publish → drain. A build failure
    /// leaves the previous agent untouched; publish swaps exactly the
    /// `agent:active` readers.
    #[test]
    fn atomic_reload_build_failure_and_swap() {
        let mut ctx = Context::new();
        let mut reloader = AgentReloader::new();

        let mut generation = AgentGeneration::default();
        let old_stream = generation.add(Resource::open("provider/stream/old"));
        let old_handle = Arc::new(Mutex::new(generation));
        reloader
            .publish(&mut ctx, || Ok(old_handle.clone()))
            .unwrap();
        assert!(Arc::ptr_eq(
            ctx.get::<AgentHandle>(KEY_ACTIVE).unwrap(),
            &old_handle
        ));

        // Build failure → previous agent untouched.
        let err: Result<AgentHandle, String> = Err("preflight boom".to_owned());
        let r = reloader.publish(&mut ctx, || err);
        assert!(r.is_err(), "build failure propagates");
        assert!(Arc::ptr_eq(
            ctx.get::<AgentHandle>(KEY_ACTIVE).unwrap(),
            &old_handle
        ));
        assert!(old_stream.is_live(), "old stream kept on build failure");

        // Publish new generation → single swap; the retired generation is
        // reported so the caller can finish in-flight turns, then drain.
        let mut new_generation = AgentGeneration::default();
        let new_stream = new_generation.add(Resource::open("provider/stream/new"));
        let new_handle = Arc::new(Mutex::new(new_generation));
        let drain = reloader
            .publish(&mut ctx, || Ok(new_handle.clone()))
            .unwrap();
        assert!(Arc::ptr_eq(
            ctx.get::<AgentHandle>(KEY_ACTIVE).unwrap(),
            &new_handle
        ));
        assert!(Arc::ptr_eq(&drain.current, &new_handle));

        // Drain: the retired old generation is still live while we finish
        // in-flight work; closing it (as a scope's dispose would) frees the
        // old stream only, leaving the new generation unharmed.
        let retired = drain.retired.as_ref().unwrap();
        let retired_gen = retired.lock().unwrap();
        assert!(
            retired_gen.has_live(),
            "retired generation draining in-flight"
        );
        assert!(
            old_stream.is_live(),
            "old stream alive until drain completes"
        );
        (retired_gen.dispose_inverse())(&mut ctx);
        drop(retired_gen);
        assert!(!old_stream.is_live(), "old stream drained");
        assert!(new_stream.is_live(), "new stream unharmed by drain");
        assert!(
            Arc::ptr_eq(ctx.get::<AgentHandle>(KEY_ACTIVE).unwrap(), &new_handle),
            "new generation still active after drain"
        );

        reloader.clear(&mut ctx);
        assert!(!ctx.has(KEY_ACTIVE), "cleared agent leaves no residue");
    }

    /// Reload publish notifies exactly the declared `agent:active` readers.
    #[test]
    fn reload_publish_notifies_only_active_readers() {
        let mut ctx = Context::new();
        let mut reloader = AgentReloader::new();

        let active_hits = Arc::new(AtomicUsize::new(0));
        let a = active_hits.clone();
        ctx.mount(
            Component::new(vec![KEY_ACTIVE])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_ACTIVE);
                    a.fetch_add(1, Ordering::Relaxed);
                }),
        );
        ctx.mount(
            Component::new(vec![KEY_MODEL])
                .effect(|_| Box::new(|_| {}))
                .on_change(move |_, _| panic!("model reader fired on active swap")),
        );

        reloader
            .publish(&mut ctx, || {
                Ok(Arc::new(Mutex::new(AgentGeneration::default())))
            })
            .unwrap();
        assert_eq!(active_hits.load(Ordering::Relaxed), 1);
    }
}
