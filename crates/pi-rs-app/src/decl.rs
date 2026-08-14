//! `decl` — the single public declaration mechanism for app runs (axiom 05),
//! backed by the pi-rs kernel.
//!
//! Applications/frontends, commands, tools, and resources all register through
//! one public path: [`Registry::declare`]. The [`Kind`] tag is metadata — it
//! describes *what* a unit is, never *how* it is launched: a unit is a kernel
//! [`Component`] with declared `reads` and per-effect inverses, exactly like
//! every other unit. There is no per-kind launcher branch.
//!
//! ## Generation / epoch lifecycle
//!
//! Each declared unit walks `Inactive → Preparing → Active → Draining →
//! Inactive`. A consumer's epoch is `[key, provider id, version, schema hash]`
//! ([`Epoch`]). When a declared read-key changes, the kernel spatial axis
//! notifies only that unit's readers; the registry then recomputes each
//! affected consumer's epoch and reloads it only when that epoch actually
//! changed. An undeclared key change fires nothing, and a change that keeps the
//! same epoch forces no reload.
//!
//! ## Atomic reload
//!
//! Rebuilding a unit runs `build → activate → publish → drain`. `build` is a
//! pure preflight that touches no live state (a failure leaves the published
//! generation untouched); `activate` mounts the new generation, `publish` is a
//! single swap of the unit's active scope, and `drain` finishes any in-flight
//! work ([`Lease`]) before the previous generation is unmounted.
//!
//! The kernel remains the single host-owned write path: the registry owns one
//! [`Context`] and every mounted unit is a kernel component whose effects write
//! that context and whose inverses restore it on unmount (no residue).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use pi_rs_kernel::{Component, Context, Inverse};

/// A `Send + Sync`-safe shared cell for values that must be visible from
/// inside kernel closures (the kernel requires `Effect`/`OnChange` to be
/// `Send`). Used for the per-unit provider identity (so rebuilds observe the
/// redeployed provider) and for the spatial-reaction dirty set.
type Shared<T> = Arc<Mutex<T>>;

/// Lock a [`Shared`], recovering from a poisoned mutex (a panicked lock holder)
/// rather than unwrapping. Single-threaded registry use never poisons in
/// practice; this shape exists to satisfy the crate-wide deny on `unwrap`.
fn lock<T>(shared: &Shared<T>) -> MutexGuard<'_, T> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A pure, re-runnable builder for a declared unit's kernel component.
type Builder = Box<dyn Fn() -> Result<Component, String> + Send + 'static>;

/// The kinds of units pi-rs ships. Every kind registers through
/// [`Registry::declare`]; `kind` is descriptive metadata, not a launcher branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    Command,
    Tool,
    Resource,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::App => "app",
            Kind::Command => "command",
            Kind::Tool => "tool",
            Kind::Resource => "resource",
        }
    }
}

/// Provider identity of a declared unit. A unit publishes under its `key`; its
/// consumers derive their epoch from this identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    pub key: &'static str,
    pub provider_id: &'static str,
    pub version: u64,
    pub schema_hash: &'static str,
}

/// A consumer's epoch: `[key, provider id, version, schema hash]`. Only a
/// change to this tuple forces a dependent reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Epoch {
    pub key: &'static str,
    pub provider_id: &'static str,
    pub version: u64,
    pub schema_hash: &'static str,
}

impl Epoch {
    pub fn from_provider(provider: &Provider) -> Self {
        Self {
            key: provider.key,
            provider_id: provider.provider_id,
            version: provider.version,
            schema_hash: provider.schema_hash,
        }
    }
}

/// Lifecycle phase of a declared unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Inactive,
    Preparing,
    Active,
    Draining,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Inactive => "inactive",
            Phase::Preparing => "preparing",
            Phase::Active => "active",
            Phase::Draining => "draining",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DeclError {
    #[error("unknown unit '{0}'")]
    UnknownUnit(String),
    #[error("unit '{0}' is already registered")]
    DuplicateUnit(String),
    #[error("unit '{0}' build failed: {1}")]
    BuildFailed(String, String),
    #[error("unit '{0}' has in-flight work; drain deferred until it completes")]
    Busy(&'static str),
    #[error("unit '{0}' is {1}, not {2}")]
    WrongPhase(&'static str, &'static str, &'static str),
}

/// One declared unit, as authored before it is handed to the registry. All
/// kinds share this shape; there is no per-kind authoring branch.
pub struct Definition {
    pub kind: Kind,
    pub name: &'static str,
    pub provider: Provider,
    pub reads: Vec<&'static str>,
    effects: Vec<Effect>,
    /// Optional pure preflight run by `build` before any state is touched.
    preflight: Option<Box<dyn Fn() -> Result<(), String> + Send + 'static>>,
    /// Optional host-registry role a declared application runs as. Declared
    /// metadata; there is no role-launcher branch keyed off it.
    role: Option<&'static str>,
    /// For applications, whether this frontend is the terminal interactive one
    /// vs. the headless print one. Declared metadata used only to *select* a
    /// declared app; it never introduces a launcher branch distinct from the
    /// declaration.
    interactive: Option<bool>,
}

impl Definition {
    pub fn new(kind: Kind, name: &'static str, provider: Provider) -> Self {
        Self {
            kind,
            name,
            provider,
            reads: Vec::new(),
            effects: Vec::new(),
            preflight: None,
            role: None,
            interactive: None,
        }
    }

    pub fn reads(mut self, reads: Vec<&'static str>) -> Self {
        self.reads = reads;
        self
    }

    /// Attach the generic host-registry role a declared application runs as.
    /// This is declared metadata (what the unit is), not a launcher branch.
    pub fn role(mut self, role: &'static str) -> Self {
        self.role = Some(role);
        self
    }

    /// Mark a declared application as the terminal-interactive frontend (true)
    /// or the headless print frontend (false). Selection metadata only.
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = Some(interactive);
        self
    }

    pub fn effect(
        mut self,
        effect: impl Fn(&mut Context) -> Inverse + Send + Sync + 'static,
    ) -> Self {
        self.effects.push(Box::new(effect));
        self
    }

    /// Register a pure preflight; a failing preflight is a build failure that
    /// leaves the published generation untouched.
    pub fn preflight(mut self, f: impl Fn() -> Result<(), String> + Send + 'static) -> Self {
        self.preflight = Some(Box::new(f));
        self
    }
}

/// The authored effects, re-exported as the kernel effect type.
type Effect = Box<dyn Fn(&mut Context) -> Inverse + Send + Sync + 'static>;

struct UnitState {
    kind: Kind,
    provider: Shared<Provider>,
    reads: Vec<&'static str>,
    /// Declared host-registry role (metadata for applications); `None` for
    /// units that carry no role (commands/tools/resources).
    role: Option<&'static str>,
    /// Whether this declared unit is the interactive frontend (selection
    /// metadata; `None` for non-apps).
    interactive: Option<bool>,
    phase: Phase,
    scope: Option<usize>,
    epoch: Option<Epoch>,
    /// Bumped on every publish; observable react counter for tests.
    generation: u64,
}

/// A lease on one active unit. While held the unit is "in flight"; a drain
/// defers dropping the generation until all leases are released.
pub struct Lease {
    name: &'static str,
    in_flight: Shared<HashMap<&'static str, usize>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(count) = lock(&self.in_flight).get_mut(&self.name) {
            *count = count.saturating_sub(1);
        }
    }
}

/// The app-decl registry: one kernel-`Context`-backed store of declared units.
///
/// This is the *only* public registration path. Whatever a unit is (an
/// application, a command, a tool, a resource), it lands here through
/// [`Registry::declare`] as a kernel component.
pub struct Registry {
    cx: Context,
    units: HashMap<&'static str, UnitState>,
    /// Pure builders keyed by name so `build` can rerun (and fail) per reload
    /// without touching live state.
    builders: HashMap<&'static str, Builder>,
    dirty: Shared<HashSet<&'static str>>,
    /// Last epoch a given (reader, key) pair has been notified with.
    dep_epochs: HashMap<(&'static str, &'static str), Epoch>,
    in_flight: Shared<HashMap<&'static str, usize>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            cx: Context::new(),
            units: HashMap::new(),
            builders: HashMap::new(),
            dirty: Arc::new(Mutex::new(HashSet::new())),
            dep_epochs: HashMap::new(),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// THE one declaration mechanism (axiom 05). Applications, commands,
    /// tools, and resources all register here and only here; `kind` is the
    /// only thing that differs between them, and it is never branched on.
    pub fn declare(&mut self, definition: Definition) -> Result<(), DeclError> {
        self.declare_inner(definition)
    }

    fn declare_inner(&mut self, definition: Definition) -> Result<(), DeclError> {
        let name = definition.name;
        if self.units.contains_key(name) {
            return Err(DeclError::DuplicateUnit(name.to_owned()));
        }
        let Definition {
            kind,
            name,
            provider,
            reads,
            effects,
            preflight,
            role,
            interactive,
        } = definition;

        // The provider identity is shared with every built (and rebuilt)
        // component so a redeploy's new provider is observed by fresh mounts.
        let provider_cell: Shared<Provider> = Arc::new(Mutex::new(provider));
        let effect_reads = reads.clone();
        let dirty_call: Shared<HashSet<&'static str>> = self.dirty.clone();
        // The builder is an `Fn` (re-runnable on every reload), so the
        // authored effects must be shareable: wrap each in an `Arc`.
        let effects: Vec<Arc<Effect>> = effects.into_iter().map(Arc::new).collect();
        let builder: Builder = Box::new(move || -> Result<Component, String> {
            if let Some(preflight) = &preflight {
                preflight()?;
            }
            let mut component = Component::new(effect_reads.clone());
            // Authored effects, each with its own inverse.
            for authored in effects.iter() {
                let authored = Arc::clone(authored);
                component = component.effect(move |cx| authored(cx));
            }
            // Spatial reaction: mark this unit dirty when a declared read
            // key changes. The registry later recomputes the epoch and
            // reloads only on an actual epoch change.
            let dirty = dirty_call.clone();
            component = component.on_change(move |_, _| {
                lock(&dirty).insert(name);
            });
            Ok(component)
        });

        self.builders.insert(name, builder);
        self.units.insert(
            name,
            UnitState {
                kind,
                provider: provider_cell,
                reads,
                role,
                interactive,
                phase: Phase::Inactive,
                scope: None,
                epoch: None,
                generation: 0,
            },
        );
        Ok(())
    }

    /// Build a fresh kernel component for a declared unit. This is the pure
    /// build step of the atomic reload: it touches no live state. A failure
    /// here (preflight) propagates as a build failure and leaves any published
    /// generation untouched.
    fn build(&self, name: &'static str) -> Result<Component, DeclError> {
        let builder = self
            .builders
            .get(&name)
            .ok_or(DeclError::UnknownUnit(name.to_owned()))?;
        builder().map_err(|message| DeclError::BuildFailed(name.to_owned(), message))
    }

    /// Activate a declared unit: `Inactive → Preparing → Active`.
    pub fn activate(&mut self, name: &'static str) -> Result<(), DeclError> {
        self.ensure_phase(name, Phase::Inactive)?;
        let unit = self
            .units
            .get_mut(&name)
            .ok_or(DeclError::UnknownUnit(name.to_owned()))?;
        unit.phase = Phase::Preparing;

        // build → activate.
        let component = self.build(name)?;
        let scope = self.cx.mount(component);

        let unit = self
            .units
            .get_mut(&name)
            .ok_or(DeclError::UnknownUnit(name.to_owned()))?;
        let provider = *lock(&unit.provider);
        unit.epoch = Some(Epoch::from_provider(&provider));
        unit.scope = Some(scope);
        unit.generation += 1;
        unit.phase = Phase::Active;
        // Publish this unit's provider epoch under its key. The registry owns
        // the published value (not an authored effect), so reloads and redeploys
        // can atomically swap generations without the old generation's inverses
        // clobbering the new value.
        self.publish(name, provider);
        // Record baseline dependency epochs so an identical later set is a
        // no-op (only an epoch change reloads).
        self.refresh_dep_epochs(name);
        Ok(())
    }

    /// Commit a provider-key change into the kernel context. Only declared
    /// readers are notified (kernel spatial axis); the registry then reloads
    /// exactly those whose epoch actually changed.
    pub fn set(&mut self, key: &'static str, epoch: Epoch) {
        self.cx.set(key, epoch);
        self.process_dirty_and_reload();
    }

    /// Atomically redeploy a unit with a new provider identity (e.g. a new
    /// version or schema hash): `build → activate → publish → drain`. A
    /// failing build leaves the published runtime untouched.
    pub fn redeploy(&mut self, name: &'static str, provider: Provider) -> Result<(), DeclError> {
        self.ensure_phase(name, Phase::Active)?;

        // build — pure preflight, touches nothing if it fails.
        let component = self.build(name)?;
        // activate the new generation.
        let new_scope = self.cx.mount(component);
        // publish: single swap of the active scope and the provider cell, in
        // one place.
        let unit = self
            .units
            .get_mut(&name)
            .ok_or(DeclError::UnknownUnit(name.to_owned()))?;
        let old_scope = unit.scope.take();
        unit.scope = Some(new_scope);
        *lock(&unit.provider) = provider;
        unit.epoch = Some(Epoch::from_provider(&provider));
        unit.generation += 1;
        // drain the old generation.
        if let Some(old_scope) = old_scope {
            self.cx.unmount(old_scope);
        }
        // Re-publish the new provider epoch; the old generation owned no
        // provider effect, so the published value reflects the new generation.
        self.publish(name, provider);
        self.refresh_dep_epochs(name);
        Ok(())
    }

    /// Drain a unit: `Active → Draining → Inactive`. While in-flight work
    /// exists (a [`Lease`] is held) the generation is kept alive and drain is
    /// deferred (`DeclError::Busy`); once it completes the generation is
    /// unmounted residue-free.
    pub fn drain(&mut self, name: &'static str) -> Result<(), DeclError> {
        let in_flight = lock(&self.in_flight).get(&name).copied().unwrap_or(0);
        if in_flight > 0 {
            return Err(DeclError::Busy(name));
        }
        let phase = self.units.get(&name).map(|u| u.phase);
        let Some(phase) = phase else {
            return Err(DeclError::UnknownUnit(name.to_owned()));
        };
        if phase != Phase::Active && phase != Phase::Draining {
            return Err(DeclError::WrongPhase(
                name,
                phase.as_str(),
                Phase::Active.as_str(),
            ));
        }
        if let Some(unit) = self.units.get_mut(&name) {
            unit.phase = Phase::Draining;
        }
        if let Some(scope) = self.units.get_mut(&name).and_then(|u| u.scope.take()) {
            self.cx.unmount(scope);
        }
        if let Some(unit) = self.units.get_mut(&name) {
            let provider = *lock(&unit.provider);
            unit.phase = Phase::Inactive;
            unit.epoch = None;
            self.unpublish(name, &provider);
        }
        Ok(())
    }

    /// Guard that `name` is declared and currently in `expected`.
    fn ensure_phase(&self, name: &'static str, expected: Phase) -> Result<(), DeclError> {
        match self.units.get(&name).map(|u| u.phase) {
            None => Err(DeclError::UnknownUnit(name.to_owned())),
            Some(actual) if actual != expected => Err(DeclError::WrongPhase(
                name,
                actual.as_str(),
                expected.as_str(),
            )),
            _ => Ok(()),
        }
    }

    /// Immutable access to a unit's phase (for lifecycle tests).
    pub fn phase(&self, name: &'static str) -> Option<Phase> {
        self.units.get(&name).map(|u| u.phase)
    }

    /// The unit's current generation (publish count). Every reload bumps it.
    pub fn generation(&self, name: &'static str) -> u64 {
        self.units.get(&name).map(|u| u.generation).unwrap_or(0)
    }

    /// True if `key` currently has a value in the kernel context. Used to prove
    /// unmount leaves no residue.
    pub fn has(&self, key: &'static str) -> bool {
        self.cx.has(key)
    }

    /// The registered units, as `(name, kind)` pairs.
    pub fn units(&self) -> Vec<(&'static str, Kind)> {
        let mut pairs: Vec<(&'static str, Kind)> =
            self.units.iter().map(|(name, u)| (*name, u.kind)).collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        pairs
    }

    /// Acquire a lease on an active unit, marking it in-flight.
    pub fn acquire(&self, name: &'static str) -> Option<Lease> {
        if self.units.get(&name)?.phase != Phase::Active {
            return None;
        }
        lock(&self.in_flight)
            .entry(name)
            .and_modify(|c| *c += 1)
            .or_insert(1);
        Some(Lease {
            name,
            in_flight: self.in_flight.clone(),
        })
    }

    /// Recompute every dirty consumer's epoch; reload only those that changed.
    /// Loops until the notification fan-out settles (each reload updates the
    /// recorded epoch, so chains terminate).
    fn process_dirty_and_reload(&mut self) {
        for _ in 0..1_000 {
            let dirty: Vec<&'static str> = lock(&self.dirty).iter().copied().collect();
            if dirty.is_empty() {
                break;
            }
            lock(&self.dirty).clear();
            for name in dirty {
                if self.epoch_changed(name) {
                    let _ = self.reload_unit(name);
                }
            }
        }
    }

    /// True if any of `name`'s declared read keys now carries a different epoch
    /// than the last one it was notified with.
    fn epoch_changed(&self, name: &'static str) -> bool {
        let Some(reads) = self.units.get(&name).map(|u| u.reads.clone()) else {
            return false;
        };
        reads.into_iter().any(|key| {
            let now = self.cx.get::<Epoch>(key).copied();
            let before = self.dep_epochs.get(&(name, key)).copied();
            now != before
        })
    }

    /// Atomic reload of a consumer whose epoch changed:
    /// `build → activate → publish → drain`. A failing build leaves the
    /// published generation alive.
    fn reload_unit(&mut self, name: &'static str) -> Result<(), DeclError> {
        let phase = self.units.get(&name).map(|u| u.phase);
        let Some(phase) = phase else {
            return Err(DeclError::UnknownUnit(name.to_owned()));
        };
        if phase != Phase::Active {
            return Ok(());
        }
        // build — pure preflight, a failure leaves the published generation.
        let component = self.build(name)?;
        // activate the new generation.
        let new_scope = self.cx.mount(component);
        // publish: single swap of the active scope; the provider cell is
        // unchanged (a dependent reload reacts to a provider change, it does
        // not itself change the provider it observes).
        let unit = self
            .units
            .get_mut(&name)
            .ok_or(DeclError::UnknownUnit(name.to_owned()))?;
        let old_scope = unit.scope.take();
        unit.scope = Some(new_scope);
        let provider = *lock(&unit.provider);
        unit.epoch = Some(Epoch::from_provider(&provider));
        unit.generation += 1;
        // drain the old generation.
        if let Some(old_scope) = old_scope {
            self.cx.unmount(old_scope);
        }
        // Re-publish the current provider epoch: the old generation owned no
        // provider effect, so the published value is unchanged by the swap.
        self.publish(name, provider);
        // Record the now-current epoch for every read key so a repeated
        // identical notification is a no-op.
        self.refresh_dep_epochs(name);
        Ok(())
    }

    /// Publish a unit's provider epoch under its key (kernel single write
    /// path). Only declared readers are notified by the kernel. `name` is
    /// cleared from the dirty set because it reacted to its own republish, but
    /// as a publisher (not reader of that key) it need not reload.
    fn publish(&mut self, name: &'static str, provider: Provider) {
        let key = provider.key;
        self.cx.set(key, Epoch::from_provider(&provider));
        // This unit is not a reader of its own key unless it declared it; the
        // kernel fans out only to declared readers. Marking `name` is spurious
        // but harmless and keeps dirty-state consistent when a unit both
        // publishes and reads the same key.
        lock(&self.dirty).remove(name);
    }

    /// Remove a unit's published provider epoch from the kernel context.
    fn unpublish(&mut self, name: &'static str, provider: &Provider) {
        self.cx.remove(provider.key);
        lock(&self.dirty).remove(name);
    }

    fn refresh_dep_epochs(&mut self, name: &'static str) {
        let reads = self.units.get(&name).map(|u| u.reads.clone());
        let Some(reads) = reads else { return };
        for key in reads {
            if let Some(epoch) = self.cx.get::<Epoch>(key) {
                self.dep_epochs.insert((name, key), *epoch);
            }
        }
    }

    /// The shipped frontend applications, *declared* through the single
    /// [`Registry::declare`] mechanism (axiom 05): applications/frontends are
    /// units of a kind like any other, not a separate hardcoded launcher table.
    /// Returns the declared-unit names.
    pub fn declare_shipped(&mut self) -> Vec<&'static str> {
        let shipped = [
            (
                "print-application",
                Provider {
                    key: "app.print",
                    provider_id: "shipped-print",
                    version: 1,
                    schema_hash: "app-frontend-v1",
                },
                "print",
                false,
            ),
            (
                "interactive-frontend",
                Provider {
                    key: "app.interactive",
                    provider_id: "shipped-interactive",
                    version: 1,
                    schema_hash: "app-frontend-v1",
                },
                "interactive",
                true,
            ),
        ];
        shipped
            .into_iter()
            .filter_map(|(name, provider, role, interactive)| {
                let def = Definition::new(Kind::App, name, provider)
                    .role(role)
                    .interactive(interactive);
                self.declare(def).ok().map(|_| name)
            })
            .collect()
    }

    /// Select the declared application to launch by whether it is the
    /// terminal-interactive frontend or the headless print frontend. Returns
    /// the generic host-registry role the launcher invokes. This is the single
    /// place the launcher consults declared apps; it replaces the previous
    /// hardcoded inline role ternary. `shipped` default preserves observable
    /// Pi behavior when a binary hasn't declared apps (bare core still boots).
    pub fn select_frontend(&self, interactive: bool) -> &'static str {
        self.units
            .values()
            .find(|u| u.kind == Kind::App && u.interactive == Some(interactive))
            .and_then(|u| u.role)
            .unwrap_or("print")
    }
}

// ---------------------------------------------------------------------------
// Shipped app declarations (frontends). These are declared on the registry via
// [`Registry::declare_shipped`] — the one declaration mechanism (axiom 05);
// the previous hardcoded `interactive ? "interactive" : "print"` role ternary
// in the launcher is replaced by a declared-app lookup.
// ---------------------------------------------------------------------------
