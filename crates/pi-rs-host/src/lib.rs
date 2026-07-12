//! pi-rs-host — the Lua extension host (DESIGN.md WS1; spec:
//! `ref/pi/packages/coding-agent/src/core/extensions/`).
//!
//! The substrate half of the extension seam: an mlua VM on a dedicated
//! thread, an event bus with an open string vocabulary, the coroutine
//! async seam (handlers may await host futures — the locked decision), and
//! a per-dispatch watchdog that meters Lua execution slices only.
//!
//! The bridge is the canonical API (Lua-first): extension chunks are
//! called with the `pi` API table as their single argument — the Lua
//! mirror of the spec's `export default function (pi)`. Extensions receive
//! plain tables and return plain tables; results come back to the caller
//! as uninterpreted JSON values — the host never matches on result
//! variants. This crate is headless by design: no terminal, no rendering,
//! no prompting.

use std::sync::mpsc::{Sender, sync_channel};

mod ai;
mod api;
mod auth;
pub mod auth_storage;
mod clipboard;
mod convert;
pub mod discover;
mod error;
mod exec;
pub mod hljs;
mod http;
pub mod image;
mod jsdiff;
pub mod model_registry;
mod os;
mod paths;
pub mod resolve_config_value;
mod schema;
mod session;
mod settings;
pub mod settings_manager;
pub mod trust;
mod vm;

pub use error::HostError;

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

/// Handle to the Lua VM thread. Cheap to clone; all methods are
/// synchronous from the caller's side (dispatch runs on the VM thread).
#[derive(Clone)]
pub struct Host {
    tx: Sender<vm::Msg>,
}

impl Host {
    /// Start the VM thread and install the `pi` API.
    pub fn new(config: HostConfig) -> Result<Self, HostError> {
        let tx = vm::spawn(config)?;
        Ok(Self { tx })
    }

    /// Load an extension chunk. `source_key` attributes registrations and
    /// error messages (a path for on-disk extensions, a symbolic key for
    /// embedded packs). The chunk runs on the coroutine path, so top-level
    /// awaits work.
    pub fn load(&self, source_key: &str, source: &str) -> Result<(), HostError> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(vm::Msg::Load {
                source_key: source_key.to_owned(),
                source: source.to_owned(),
                reply,
            })
            .map_err(|_| HostError::VmUnavailable)?;
        rx.recv().map_err(|_| HostError::VmUnavailable)?
    }

    /// Load one on-disk extension file; the path is its source key.
    pub fn load_file(&self, path: &str) -> Result<(), HostError> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| HostError::Io(format!("read '{path}': {e}")))?;
        self.load(path, &source)
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
            match self.load(&key, pack.source) {
                Ok(()) => loaded.push(key),
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
}
