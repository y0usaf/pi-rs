use std::fmt;

/// The failures a request can end in: the server's own error, a closed connection, a timeout, a
/// malformed answer, or an HTTP status.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Rpc {
        code: i64,
        message: String,
        data: Option<serde_json::Value>,
    },
    Invalid(String),
    Closed(String),
    Timeout(u64),
    Transport(String),
    Http {
        status: u16,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub fn closed(message: impl Into<String>) -> Self {
        Self::Closed(message.into())
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport(message.into())
    }

    /// The message alone, as upstream's `errorMessage()` reads an `Error`.
    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc { message, .. } => f.write_str(message),
            Self::Invalid(message) | Self::Closed(message) | Self::Transport(message) => {
                f.write_str(message)
            }
            Self::Timeout(ms) => write!(f, "MCP request timed out after {ms}ms"),
            Self::Http { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}
