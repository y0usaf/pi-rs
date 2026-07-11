//! Auth error type — typed errors per layer (locked code standard).
//!
//! The spec throws plain `Error`s with formatted messages; those arrive
//! here as [`AuthError::Message`] with the spec's exact strings so
//! user-visible output stays 1:1.

/// Errors from OAuth login, refresh, and credential resolution.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Spec: `throw new Error(message)` — the message matches the spec.
    #[error("{0}")]
    Message(String),
    /// A callback (prompt / manual input) was cancelled by the user.
    /// The spec models this as a rejected promise; Rust names it.
    #[error("login cancelled")]
    Cancelled,
}
