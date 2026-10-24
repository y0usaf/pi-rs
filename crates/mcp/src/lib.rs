//! `packages/mcp`: a Model Context Protocol client, its JSON-RPC and protocol types, and the
//! stdio and streamable HTTP transports. Synchronous: one client owns one transport and answers
//! the server's own requests between its own.

pub mod client;
pub mod content;
pub mod error;
pub mod http;
pub mod jsonrpc;
pub mod protocol;
pub mod stdio;
pub mod transport;

pub use client::McpClient;
pub use content::LlmContent;
pub use error::{Error, Result};
pub use http::StreamableHttpTransport;
pub use protocol::{
    CallToolResult, ContentBlock, Implementation, InitializeResult, LATEST_PROTOCOL_VERSION, Root,
    SUPPORTED_PROTOCOL_VERSIONS, Tool,
};
pub use stdio::{StdioTransport, StdioTransportOptions};
pub use transport::Transport;
