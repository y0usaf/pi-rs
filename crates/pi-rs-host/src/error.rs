//! Typed host errors.

pub(crate) const WATCHDOG_MARKER: &str = "pi-rs-host watchdog:";
pub(crate) const CANCEL_MARKER: &str = "pi-rs-host cancelled";
pub(crate) const CONFLICT_MARKER: &str = "pi-rs-host conflict:";

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("lua: {0}")]
    Lua(String),

    #[error("handler timed out (watchdog, {0}ms of continuous Lua execution)")]
    Timeout(i64),

    #[error("lua vm thread unavailable")]
    VmUnavailable,

    #[error("no active kernel root for '{0}'")]
    UnknownRoot(String),

    #[error("{kind} root '{id}' selected by {selected_by} is not registered and active")]
    UnknownSelectedRoot {
        kind: String,
        id: String,
        selected_by: String,
    },

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

    #[error("io: {0}")]
    Io(String),
}

impl HostError {
    pub(crate) fn from_lua_message(message: String, budget_ms: i64) -> Self {
        if message.contains(WATCHDOG_MARKER) {
            HostError::Timeout(budget_ms)
        } else if message.contains(CANCEL_MARKER) {
            HostError::Cancelled
        } else if let Some((_, conflict)) = message.split_once(CONFLICT_MARKER) {
            HostError::Conflict(
                conflict
                    .lines()
                    .next()
                    .unwrap_or(conflict)
                    .trim()
                    .to_owned(),
            )
        } else {
            HostError::Lua(message)
        }
    }
}
