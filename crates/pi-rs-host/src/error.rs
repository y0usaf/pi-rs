//! Typed errors for the host layer (code standard: `thiserror` per layer,
//! no `unwrap`/`expect` in library crates).

/// Marker embedded in watchdog-raised Lua errors so the host can
/// distinguish a budget kill from an ordinary handler error.
pub(crate) const WATCHDOG_MARKER: &str = "pi-rs-host watchdog:";
pub(crate) const CANCEL_MARKER: &str = "pi-rs-host cancelled";
pub(crate) const CONFLICT_MARKER: &str = "pi-rs-host conflict:";

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// A Lua-side failure (load error, handler error). mlua errors are
    /// flattened to strings at the thread boundary.
    #[error("lua: {0}")]
    Lua(String),

    /// The dispatch exceeded its watchdog budget of continuous Lua
    /// execution time (the window resets at every host await).
    #[error("handler timed out (watchdog, {0}ms of continuous Lua execution)")]
    Timeout(i64),

    /// The VM thread is gone or failed to start.
    #[error("lua vm thread unavailable")]
    VmUnavailable,

    /// The VM thread did not reply within the budget plus margin — a
    /// blocking C call the instruction hook cannot interrupt.
    #[error("lua vm thread unresponsive (blocking call beyond watchdog reach)")]
    VmUnresponsive,

    /// `call_tool` with a name no extension registered.
    #[error("no registered tool named '{0}'")]
    UnknownTool(String),

    /// `call_command` with an invocation name no extension resolves to.
    #[error("no registered command named '{0}'")]
    UnknownCommand(String),

    /// `call_role` with no active declaration for the requested legacy role.
    #[error("no active legacy declaration for role '{0}'")]
    UnknownRole(String),

    #[error("no active kernel root for '{0}'")]
    UnknownRoot(String),

    #[error("invalid kernel root kind '{0}'")]
    InvalidRootKind(String),

    #[error("invalid declaration kind '{0}'")]
    InvalidDeclarationKind(String),

    #[error("declaration conflict: {0}")]
    Conflict(String),

    #[error("dispatch cancelled")]
    Cancelled,

    #[error("stale read handle generation {handle}; current generation is {current}")]
    StaleHandle { handle: u64, current: u64 },

    #[error("unknown scope {0}")]
    UnknownScope(u64),

    #[error("scope {0} is disposed")]
    DisposedScope(u64),

    #[error("scope {0} is not owned by this package handle")]
    ScopeOwnership(u64),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Host-side I/O failure (e.g. reading an extension file).
    #[error("io: {0}")]
    Io(String),
}

impl HostError {
    /// Classify a Lua error string: watchdog kills become [`HostError::Timeout`].
    pub(crate) fn from_lua_message(msg: String, budget_ms: i64) -> Self {
        if msg.contains(WATCHDOG_MARKER) {
            HostError::Timeout(budget_ms)
        } else if msg.contains(CANCEL_MARKER) {
            HostError::Cancelled
        } else if let Some((_, conflict)) = msg.split_once(CONFLICT_MARKER) {
            HostError::Conflict(
                conflict
                    .lines()
                    .next()
                    .unwrap_or(conflict)
                    .trim()
                    .to_owned(),
            )
        } else {
            HostError::Lua(msg)
        }
    }
}
