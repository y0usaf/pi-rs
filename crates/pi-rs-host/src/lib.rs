//! Lua package host and generic mechanism kernel.
//!
//! The canonical boundary is one watchdog-bounded transaction: an immutable
//! event/context snapshot enters Lua and one validated action/effect batch is
//! published after a successful root dispatch. Package provenance affects only
//! byte loading and diagnostics; every source uses the same API and scope.
//!
//! Ownership is separated between public transaction types (`kernel`), Lua
//! transaction bindings (`kernel_api`), package resolution (`package`), VM
//! scheduling (`vm`), and narrow registry attribution (`runtime_registry`).
//! Compatibility bindings remain callable while downstream launchers migrate,
//! but they do not participate in or bypass the kernel publication path.

use std::sync::mpsc::{Sender, sync_channel};

mod ai;
mod api;
mod auth;
pub mod auth_storage;
mod clipboard;
pub mod config;
mod convert;
pub mod discover;
mod error;
mod exec;
pub mod hljs;
mod http;
pub mod image;
mod jsdiff;
pub mod kernel;
mod kernel_api;
pub mod model_registry;
mod os;
mod package;
mod paths;
pub mod resolve_config_value;
mod runtime_registry;
mod schema;
mod session;
mod settings;
pub mod settings_manager;
pub mod trust;
mod vm;

pub use error::HostError;
pub use package::PackageSource;

/// Default per-dispatch watchdog budget in milliseconds of *continuous
/// Lua execution* (time suspended awaiting host futures is free, and
/// every await resets the window).
pub const DEFAULT_DISPATCH_TIMEOUT_MS: i64 = 5000;

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Watchdog budget per host→Lua dispatch, in milliseconds of
    /// continuous Lua execution time between host awaits.
    pub dispatch_timeout_ms: i64,
    /// Working directory for the OS bindings — the `pi.exec` default cwd,
    /// `pi.cwd()`, and the base for `pi.path.resolve`/`relative` (spec:
    /// the loader's injected `cwd`). `None` = process cwd at startup.
    pub cwd: Option<String>,
    /// Whether project-local settings/resources are trusted for this VM.
    pub project_trusted: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            dispatch_timeout_ms: DEFAULT_DISPATCH_TIMEOUT_MS,
            cwd: None,
            project_trusted: true,
        }
    }
}

/// Outcome of one handler dispatch, attributed to the extension that
/// registered it. The payload is the handler's return value (`None` when
/// the handler returned nothing); errors are strings so one failing
/// handler never poisons the batch.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub source: String,
    pub result: Result<Option<serde_json::Value>, String>,
}

/// Host-side metadata mirror of one registered tool (spec:
/// `RegisteredTool`). `meta` is the JSON-able slice of the definition
/// (name, label, description, parameters, …) with function fields
/// stripped; the host hands it to callers uninterpreted.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    /// Source key of the registering extension.
    pub source: String,
    pub meta: serde_json::Value,
}
/// Thread-safe sink for partial tool results (`ToolDefinition.execute`'s
/// `onUpdate` parameter). Callbacks run synchronously on the Lua VM thread.
pub type ToolUpdateCallback = std::sync::Arc<dyn Fn(serde_json::Value) + Send + Sync>;

/// One extension path that failed to load (spec: the `errors` array of
/// `LoadExtensionsResult` — `{ path, error }`).
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: String,
    pub error: String,
}

/// Result of loading a batch of extension files (spec:
/// `LoadExtensionsResult`): loaded paths plus per-path errors — one
/// failing extension never aborts the batch.
#[derive(Debug, Clone)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    pub errors: Vec<LoadError>,
}

/// One embedded extension pack — a `.lua` chunk compiled into the binary
/// (`include_str!`) and loaded through the same public path as on-disk
/// extensions (locked decision: shipped defaults are embedded `.lua` via
/// the public API; divergence 2). Its source key is the synthetic path
/// `<name>`, the spec's convention for non-file extensions (loader.ts
/// `createExtension`: paths wrapped in `<...>`, e.g. `<inline>`).
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedPack {
    pub name: &'static str,
    pub source: &'static str,
}

impl EmbeddedPack {
    /// The synthetic source key this pack loads under: `<name>`.
    #[must_use]
    pub fn source_key(&self) -> String {
        format!("<{}>", self.name)
    }
}

/// Host-side mirror of one provider registration (spec: the queued
/// `registerProvider(name, config)` call — `ProviderConfig` in
/// `types.ts`). `config` is the JSON-able slice with function values
/// (`streamSimple`, `oauth` callbacks) stripped at any depth; the
/// functions stay Lua-side, invocable when their mechanisms land.
/// Application to a model registry (validation included, spec
/// `model-registry.ts`) is the embedder's.
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub name: String,
    /// Source key of the registering extension.
    pub source: String,
    pub config: serde_json::Value,
}

/// Host-side metadata mirror of one registered command (spec:
/// `ResolvedCommand`). When more than one extension registers the same
/// name, `invocation_name` disambiguates as `name:N` (spec:
/// `runner.resolveRegisteredCommands()`).
#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub invocation_name: String,
    /// Source key of the registering extension.
    pub source: String,
    pub description: Option<String>,
}

/// Host-side metadata mirror of one registered extension CLI flag (spec:
/// `ExtensionRunner.getFlags()`). The first registration per name wins.
#[derive(Debug, Clone)]
pub struct FlagInfo {
    pub name: String,
    pub source: String,
    pub description: Option<String>,
    pub flag_type: String,
    pub default: Option<serde_json::Value>,
}

/// Host-side metadata mirror of one public application/frontend role.
/// `role` is the generic launcher role selected by the embedder; `id`,
/// activation, and priority are declaration data owned by Lua packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleInfo {
    pub id: String,
    pub role: String,
    pub source: String,
    pub active: bool,
    pub priority: i64,
}

/// Handle to the Lua VM thread. Cheap to clone; all methods are
/// synchronous from the caller's side (dispatch runs on the VM thread).
#[derive(Clone)]
pub struct Host {
    tx: Sender<vm::Msg>,
    control: std::sync::Arc<kernel::Control>,
    owners: std::sync::Arc<()>,
}

impl Host {
    /// Start the VM thread and install the `pi` API.
    pub fn new(config: HostConfig) -> Result<Self, HostError> {
        let control = kernel::Control::new();
        let tx = vm::spawn(config, std::sync::Arc::clone(&control))?;
        Ok(Self {
            tx,
            control,
            owners: std::sync::Arc::new(()),
        })
    }

    /// Load any package provenance through the one source-neutral transaction.
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

    /// Load an extension chunk. `source_key` attributes registrations and
    /// error messages (a path for on-disk extensions, a symbolic key for
    /// embedded packs). The chunk runs on the coroutine path, so top-level
    /// awaits work.
    pub fn load(&self, source_key: &str, source: &str) -> Result<(), HostError> {
        self.load_package(PackageSource::Memory {
            key: source_key,
            source,
        })
        .map(|_| ())
    }

    /// Load one on-disk extension file through the canonical package loader.
    pub fn load_file(&self, path: &str) -> Result<(), HostError> {
        self.load_package(PackageSource::File {
            path: std::path::Path::new(path),
        })
        .map(|_| ())
    }

    /// Load embedded packs in order through the public load path,
    /// collecting per-pack errors in the same report shape as
    /// [`Host::load_extensions`]; the report's `path` is the synthetic
    /// source key `<name>` (spec: `loadExtensionFromFactory` runs inline
    /// factories through the same `createExtensionAPI` as file loads).
    pub fn load_embedded(&self, packs: &[EmbeddedPack]) -> LoadReport {
        let mut loaded = Vec::new();
        let mut errors = Vec::new();
        for pack in packs {
            let key = pack.source_key();
            match self.load_package(PackageSource::Embedded {
                name: pack.name,
                source: pack.source,
            }) {
                Ok(_) => loaded.push(key),
                Err(e) => errors.push(LoadError {
                    path: key,
                    error: format!("Failed to load extension: {e}"),
                }),
            }
        }
        LoadReport { loaded, errors }
    }

    /// Load extension files in order, collecting per-path errors (spec:
    /// `loadExtensions` — `"Failed to load extension: ..."`); discovery
    /// is [`discover::discover_extension_paths`].
    pub fn load_extensions(&self, paths: &[String]) -> LoadReport {
        let mut loaded = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            match self.load_file(path) {
                Ok(()) => loaded.push(path.clone()),
                Err(e) => errors.push(LoadError {
                    path: path.clone(),
                    error: format!("Failed to load extension: {e}"),
                }),
            }
        }
        let (reply, rx) = sync_channel(1);
        let conflicts = self
            .tx
            .send(vm::Msg::ExtensionConflicts { reply })
            .map_err(|_| HostError::VmUnavailable)
            .and_then(|()| rx.recv().map_err(|_| HostError::VmUnavailable))
            .and_then(|result| result);
        match conflicts {
            Ok(conflicts) => errors.extend(
                conflicts
                    .into_iter()
                    .map(|(path, error)| LoadError { path, error }),
            ),
            Err(error) => errors.push(LoadError {
                path: "<host>".to_owned(),
                error: error.to_string(),
            }),
        }
        LoadReport { loaded, errors }
    }

    /// Dispatch one immutable snapshot to the selected generic root. Queued
    /// actions/effects become visible only in the returned successful batch.
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

    /// Create a generation-checked read handle over an immutable JSON value.
    #[must_use]
    pub fn read_handle(&self, value: serde_json::Value) -> kernel::ReadHandle {
        self.control.issue_handle(value)
    }

    /// Resolve a read handle only while its generation is current.
    pub fn read(&self, handle: &kernel::ReadHandle) -> Result<serde_json::Value, HostError> {
        self.control.read_handle(handle)
    }

    /// Cancel and dispose a package scope, run its bounded disposers, remove
    /// its declarations, and invalidate all prior generation handles.
    pub fn dispose_package(&self, package: &kernel::PackageHandle) -> Result<(), HostError> {
        if self.control.scope_source(package.scope)? != package.source {
            return Err(HostError::ScopeOwnership(package.scope.get()));
        }
        self.control.dispose(package.scope)?;
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::DisposePackage {
                source: package.source.clone(),
                scope: package.scope,
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Inspect scope state for lifecycle/resource-cleanup evidence.
    pub fn scope_stats(
        &self,
        package: &kernel::PackageHandle,
    ) -> Result<kernel::ScopeStats, HostError> {
        self.control.stats(package.scope)
    }

    /// Emit an event to every subscribed handler, sequentially in
    /// registration order (spec: `runner.ts` emit semantics). One failing
    /// or hung handler doesn't stop the rest; each outcome is attributed.
    pub fn emit(
        &self,
        event: &str,
        payload: &serde_json::Value,
    ) -> Result<Vec<Outcome>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Emit {
                event: event.to_owned(),
                payload: payload.clone(),
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)
    }

    /// Metadata mirror of all registered tools, in extension load order;
    /// first registration per name wins across extensions (spec:
    /// `runner.getAllRegisteredTools()`).
    pub fn tools(&self) -> Result<Vec<ToolInfo>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Tools { reply })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Metadata mirror of all registered commands with resolved invocation
    /// names (spec: `runner.getRegisteredCommands()`).
    pub fn commands(&self) -> Result<Vec<CommandInfo>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Commands { reply })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Metadata mirror of extension CLI flags in load order; first registration
    /// per name wins (spec: `ExtensionRunner.getFlags()`).
    pub fn flags(&self) -> Result<Vec<FlagInfo>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Flags { reply })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Metadata mirror of application/frontend role declarations. Unlike
    /// commands, roles resolve by explicit activation + priority data; source
    /// identity is provenance only.
    pub fn roles(&self) -> Result<Vec<RoleInfo>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Roles { reply })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Set one parsed extension flag value in the shared runtime. Individual
    /// extensions can read it only when they registered that name.
    pub fn set_flag_value(&self, name: &str, value: serde_json::Value) -> Result<(), HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::SetFlagValue {
                name: name.to_owned(),
                value,
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Mirror of provider registrations in extension load order, then
    /// per-extension registration order; re-registration of a name
    /// merges defined keys in place (spec `upsertRegisteredProvider`).
    pub fn providers(&self) -> Result<Vec<ProviderInfo>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Providers { reply })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Execute a registered tool without observing partial updates.
    pub fn call_tool(
        &self,
        name: &str,
        tool_call_id: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, HostError> {
        self.call_tool_with_updates(name, tool_call_id, params, None)
    }

    /// Execute a registered tool and synchronously forward each partial
    /// result emitted through the spec's `onUpdate` parameter.
    pub fn call_tool_with_updates(
        &self,
        name: &str,
        tool_call_id: &str,
        params: &serde_json::Value,
        on_update: Option<ToolUpdateCallback>,
    ) -> Result<serde_json::Value, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::CallTool {
                name: name.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
                params: params.clone(),
                on_update,
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Run a registered command handler by invocation name on the
    /// coroutine path (spec: `RegisteredCommand.handler(args, ctx)`).
    pub fn call_command(
        &self,
        invocation_name: &str,
        args: &str,
    ) -> Result<Option<serde_json::Value>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::CallCommand {
                invocation_name: invocation_name.to_owned(),
                args: args.to_owned(),
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Run the active declaration for a generic application/frontend role.
    /// The handler uses the same coroutine, source attribution, invocation
    /// context, and watchdog path as every ordinary extension callback.
    pub fn call_role(
        &self,
        role: &str,
        args: &str,
    ) -> Result<Option<serde_json::Value>, HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::CallRole {
                role: role.to_owned(),
                args: args.to_owned(),
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
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
