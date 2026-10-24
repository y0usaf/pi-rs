//! MCP servers: where `mcp.json` puts them (`core/mcp-servers.ts`, `extensions/mcp/config.ts`),
//! the connections started at startup (`extensions/mcp/runtime.ts`), and their tools as the agent's
//! own (`extensions/mcp/tools.ts`). Codemode, which shows these tools as nested ones, is not ported:
//! only servers whose tools are configured `direct` declare them to the model.

pub mod config;
pub mod runtime;
pub mod tool;

pub use runtime::Servers;
