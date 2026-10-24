//! `transports/transport.ts`: what a client needs of a transport.

use std::time::Duration;

use serde_json::Value;

use crate::error::Result;

pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub trait Transport: Send {
    /// Open the transport, before the client's `initialize`.
    fn start(&mut self) -> Result<()>;
    /// Send one message; a request's answer arrives through {@link next_message}.
    fn send(&mut self, message: &Value) -> Result<()>;
    /// The next message from the server, waiting at most `timeout`.
    fn next_message(&mut self, timeout: Duration) -> Result<Value>;
    /// The protocol version the server selected, which HTTP sends as a header on what follows.
    fn set_protocol_version(&mut self, _version: &str) {}
    fn close(&mut self);
    /// The end of a stdio server's stderr, for a failure report.
    fn stderr_tail(&self) -> Option<String> {
        None
    }
}
