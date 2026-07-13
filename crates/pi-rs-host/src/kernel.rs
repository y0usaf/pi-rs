//! Generic snapshot/action kernel contracts.
//!
//! This module contains no product vocabulary. A dispatch reads one immutable
//! event/context snapshot and publishes one validated batch after Lua returns.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokio::sync::Notify;

use crate::HostError;

pub const KERNEL_API_VERSION: u32 = 1;
pub(crate) const MAX_BATCH_ITEMS: usize = 1_024;
pub(crate) const MAX_ITEM_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct Generation(u64);

impl Generation {
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct ScopeId(u64);

impl ScopeId {
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ResourceId(u64);

impl ResourceId {
    pub(crate) fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootKind {
    Application,
    Agent,
    Frontend,
    Session,
}

impl RootKind {
    pub(crate) fn parse(value: &str) -> Result<Self, HostError> {
        match value {
            "application" => Ok(Self::Application),
            "agent" => Ok(Self::Agent),
            "frontend" => Ok(Self::Frontend),
            "session" => Ok(Self::Session),
            _ => Err(HostError::InvalidRootKind(value.to_owned())),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Agent => "agent",
            Self::Frontend => "frontend",
            Self::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclarationKind {
    Tool,
    Command,
    Provider,
    Event,
    Renderer,
    UiSlot,
    Theme,
    Keymap,
}

impl DeclarationKind {
    pub(crate) fn parse(value: &str) -> Result<Self, HostError> {
        match value {
            "tool" => Ok(Self::Tool),
            "command" => Ok(Self::Command),
            "provider" => Ok(Self::Provider),
            "event" => Ok(Self::Event),
            "renderer" => Ok(Self::Renderer),
            "ui_slot" => Ok(Self::UiSlot),
            "theme" => Ok(Self::Theme),
            "keymap" => Ok(Self::Keymap),
            _ => Err(HostError::InvalidDeclarationKind(value.to_owned())),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Command => "command",
            Self::Provider => "provider",
            Self::Event => "event",
            Self::Renderer => "renderer",
            Self::UiSlot => "ui_slot",
            Self::Theme => "theme",
            Self::Keymap => "keymap",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CancellationToken(Arc<CancellationState>);

#[derive(Debug)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancellationToken {
    pub(crate) fn new() -> Self {
        Self(Arc::new(CancellationState {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }))
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            self.0.notify.notify_waiters();
        }
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.0.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[derive(Debug, Clone)]
pub struct ReadHandle {
    generation: Generation,
    value: Arc<Value>,
}

impl ReadHandle {
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }
}

#[derive(Debug, Clone)]
pub struct DispatchRequest {
    pub root: RootKind,
    pub event: Value,
    pub context: Value,
}

impl DispatchRequest {
    #[must_use]
    pub fn new(root: RootKind, event: Value, context: Value) -> Self {
        Self {
            root,
            event,
            context,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Action {
    pub sequence: u64,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Effect {
    pub sequence: u64,
    pub kind: String,
    pub payload: Value,
    pub scope: ScopeId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DispatchBatch {
    pub version: u32,
    pub generation: Generation,
    pub scope: ScopeId,
    pub source: String,
    pub actions: Vec<Action>,
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageHandle {
    pub(crate) source: String,
    pub(crate) scope: ScopeId,
    pub(crate) generation: Generation,
}

impl PackageHandle {
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn scope(&self) -> ScopeId {
        self.scope
    }

    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeStats {
    pub disposed: bool,
    pub cancelled: bool,
    pub resources: usize,
}

#[derive(Debug)]
struct ScopeState {
    source: String,
    token: CancellationToken,
    disposed: bool,
    resources: BTreeSet<ResourceId>,
}

#[derive(Debug)]
pub(crate) struct Control {
    generation: AtomicU64,
    next_scope: AtomicU64,
    next_resource: AtomicU64,
    scopes: Mutex<BTreeMap<ScopeId, ScopeState>>,
}

impl Control {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            generation: AtomicU64::new(1),
            next_scope: AtomicU64::new(1),
            next_resource: AtomicU64::new(1),
            scopes: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn generation(&self) -> Generation {
        Generation(self.generation.load(Ordering::Acquire))
    }

    pub(crate) fn create_scope(
        &self,
        source: String,
    ) -> Result<(ScopeId, CancellationToken), HostError> {
        let id = ScopeId(self.next_scope.fetch_add(1, Ordering::Relaxed));
        let token = CancellationToken::new();
        let mut scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        if scopes
            .values()
            .any(|state| !state.disposed && state.source == source)
        {
            return Err(HostError::Conflict(format!(
                "package source {source:?} is already loaded"
            )));
        }
        scopes.insert(
            id,
            ScopeState {
                source,
                token: token.clone(),
                disposed: false,
                resources: BTreeSet::new(),
            },
        );
        Ok((id, token))
    }

    pub(crate) fn token(&self, scope: ScopeId) -> Result<CancellationToken, HostError> {
        let scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        let state = scopes
            .get(&scope)
            .ok_or(HostError::UnknownScope(scope.get()))?;
        if state.disposed {
            return Err(HostError::DisposedScope(scope.get()));
        }
        Ok(state.token.clone())
    }

    pub(crate) fn register_resource(&self, scope: ScopeId) -> Result<ResourceId, HostError> {
        let mut scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        let state = scopes
            .get_mut(&scope)
            .ok_or(HostError::UnknownScope(scope.get()))?;
        if state.disposed {
            return Err(HostError::DisposedScope(scope.get()));
        }
        let id = ResourceId(self.next_resource.fetch_add(1, Ordering::Relaxed));
        state.resources.insert(id);
        Ok(id)
    }

    pub(crate) fn release_resource(&self, scope: ScopeId, resource: ResourceId) {
        let mut scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(state) = scopes.get_mut(&scope) {
            state.resources.remove(&resource);
        }
    }

    pub(crate) fn dispose(&self, scope: ScopeId) -> Result<(), HostError> {
        let mut scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        let state = scopes
            .get_mut(&scope)
            .ok_or(HostError::UnknownScope(scope.get()))?;
        if state.disposed {
            return Err(HostError::DisposedScope(scope.get()));
        }
        state.disposed = true;
        state.token.cancel();
        state.resources.clear();
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub(crate) fn active_scopes(&self) -> Vec<(ScopeId, String)> {
        let scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        scopes
            .iter()
            .filter(|(_, state)| !state.disposed)
            .map(|(scope, state)| (*scope, state.source.clone()))
            .collect()
    }

    pub(crate) fn scope_source(&self, scope: ScopeId) -> Result<String, HostError> {
        let scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        scopes
            .get(&scope)
            .map(|state| state.source.clone())
            .ok_or(HostError::UnknownScope(scope.get()))
    }

    pub(crate) fn stats(&self, scope: ScopeId) -> Result<ScopeStats, HostError> {
        let scopes = self.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        let state = scopes
            .get(&scope)
            .ok_or(HostError::UnknownScope(scope.get()))?;
        Ok(ScopeStats {
            disposed: state.disposed,
            cancelled: state.token.is_cancelled(),
            resources: state.resources.len(),
        })
    }

    pub(crate) fn issue_handle(&self, value: Value) -> ReadHandle {
        ReadHandle {
            generation: self.generation(),
            value: Arc::new(value),
        }
    }

    pub(crate) fn read_handle(&self, handle: &ReadHandle) -> Result<Value, HostError> {
        let current = self.generation();
        if handle.generation != current {
            return Err(HostError::StaleHandle {
                handle: handle.generation.get(),
                current: current.get(),
            });
        }
        Ok((*handle.value).clone())
    }
}
