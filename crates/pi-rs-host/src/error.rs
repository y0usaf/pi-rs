//! Typed errors for the host layer (code standard: `thiserror` per layer,
//! no `unwrap`/`expect` in library crates).

/// Marker embedded in watchdog-raised Lua errors so the host can
/// distinguish a budget kill from an ordinary handler error.
pub(crate) const WATCHDOG_MARKER: &str = "pi-rs-host watchdog:";

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
        } else {
            HostError::Lua(msg)
        }
    }
}
