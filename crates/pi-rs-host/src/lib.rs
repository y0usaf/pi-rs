//! Lua package host and generic mechanism kernel.
//!
//! One watchdog-bounded transaction sends an immutable event/context snapshot
//! into Lua and publishes one validated action/effect batch after a successful
//! root dispatch. File, memory, and embedded package provenance changes only
//! byte loading and diagnostics.

use std::sync::mpsc::{Sender, sync_channel};

mod ai;
mod bindings;
mod convert;
pub mod effects;
mod error;
mod exec;
mod image;
pub mod kernel;
mod kernel_api;
mod middleware;
mod module_api;
mod os;
mod package;
mod runtime_registry;
mod tui_api;
mod vm;

pub use error::HostError;
pub use package::PackageSource;
pub(crate) use runtime_registry as api;

/// Default per-dispatch watchdog budget in milliseconds of continuous Lua
/// execution. Time suspended awaiting host futures is free.
pub const DEFAULT_DISPATCH_TIMEOUT_MS: i64 = 5000;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub dispatch_timeout_ms: i64,
    /// Default working directory for filesystem and process effects.
    /// `None` resolves the process working directory at startup.
    pub cwd: Option<String>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            dispatch_timeout_ms: DEFAULT_DISPATCH_TIMEOUT_MS,
            cwd: None,
        }
    }
}

/// Handle to the dedicated Lua VM thread.
#[derive(Clone)]
pub struct Host {
    tx: Sender<vm::Msg>,
    control: std::sync::Arc<kernel::Control>,
    effects: effects::EffectHub,
    owners: std::sync::Arc<()>,
}

impl Host {
    pub fn new(config: HostConfig) -> Result<Self, HostError> {
        let control = kernel::Control::new();
        let (effects, effect_runner) = effects::EffectHub::new(std::sync::Arc::clone(&control));
        let tx = vm::spawn(
            config,
            std::sync::Arc::clone(&control),
            effects.clone(),
            effect_runner,
        )?;
        Ok(Self {
            tx,
            control,
            effects,
            owners: std::sync::Arc::new(()),
        })
    }

    /// Load any package provenance through the canonical package transaction.
    pub fn load_package(
        &self,
        package: PackageSource<'_>,
    ) -> Result<kernel::PackageHandle, HostError> {
        let package = package.resolve()?;
        let (scope, _) = self.control.create_scope(package.source_key.clone())?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Load {
                source_key: package.source_key.clone(),
                source: package.source,
                scope,
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        match rx.recv().map_err(|_| HostError::VmUnavailable)? {
            Ok(()) => Ok(kernel::PackageHandle {
                source: package.source_key,
                scope,
                generation: self.control.generation(),
            }),
            Err(error) => {
                let _ = self.control.dispose(scope);
                Err(error)
            }
        }
    }

    pub fn load(&self, source_key: &str, source: &str) -> Result<(), HostError> {
        self.load_package(PackageSource::Memory {
            key: source_key,
            source,
        })
        .map(|_| ())
    }

    pub fn load_file(&self, path: &str) -> Result<(), HostError> {
        self.load_package(PackageSource::File {
            path: std::path::Path::new(path),
        })
        .map(|_| ())
    }

    /// Dispatch one immutable snapshot to a selected root.
    pub fn dispatch(
        &self,
        request: kernel::DispatchRequest,
    ) -> Result<kernel::DispatchBatch, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Dispatch { request, reply })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    #[must_use]
    pub fn read_handle(&self, value: serde_json::Value) -> kernel::ReadHandle {
        self.control.issue_handle(value)
    }

    pub fn read(&self, handle: &kernel::ReadHandle) -> Result<serde_json::Value, HostError> {
        self.control.read_handle(handle)
    }

    pub fn dispose_package(&self, package: &kernel::PackageHandle) -> Result<(), HostError> {
        if self.control.scope_source(package.scope)? != package.source {
            return Err(HostError::ScopeOwnership(package.scope.get()));
        }
        self.control.dispose(package.scope)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::DisposePackage {
                scope: package.scope,
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    pub fn scope_stats(
        &self,
        package: &kernel::PackageHandle,
    ) -> Result<kernel::ScopeStats, HostError> {
        self.control.stats(package.scope)
    }

    #[must_use]
    pub fn effect_stats(&self) -> effects::EffectStats {
        self.effects.stats()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if std::sync::Arc::strong_count(&self.owners) != 1 {
            return;
        }
        let scopes = self.control.active_scopes();
        for (scope, _) in &scopes {
            let _ = self.control.dispose(*scope);
        }
        let (reply, rx) = sync_channel(1);
        if self.tx.send(vm::Msg::Shutdown { scopes, reply }).is_ok() {
            let _ = rx.recv();
        }
    }
}
