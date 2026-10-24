//! `client.ts`: the client's requests, its answers to the server's own requests, and closing.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::jsonrpc::{self, Id, Message};
use crate::protocol::{
    CallToolResult, InitializeResult, LATEST_PROTOCOL_VERSION, Root, SUPPORTED_PROTOCOL_VERSIONS,
    Tool, has_tools, parse_call_tool_result, parse_initialize, parse_tools_page,
};
use crate::transport::Transport;

const MAX_LIST_PAGES: usize = 1_000;

pub struct McpClient {
    transport: Box<dyn Transport>,
    name: String,
    version: String,
    next_id: u64,
    request_timeout: Duration,
    roots: Vec<Root>,
    started: bool,
    closed: bool,
    server_info: Option<crate::protocol::Implementation>,
    capabilities: Value,
    instructions: Option<String>,
    protocol_version: Option<String>,
}

impl McpClient {
    pub fn new(
        transport: Box<dyn Transport>,
        name: &str,
        version: &str,
        request_timeout: Duration,
        roots: Vec<Root>,
    ) -> Self {
        Self {
            transport,
            name: name.to_owned(),
            version: version.to_owned(),
            next_id: 1,
            request_timeout,
            roots,
            started: false,
            closed: false,
            server_info: None,
            capabilities: Value::Null,
            instructions: None,
            protocol_version: None,
        }
    }

    pub fn server_info(&self) -> Option<&crate::protocol::Implementation> {
        self.server_info.as_ref()
    }

    pub fn capabilities(&self) -> &Value {
        &self.capabilities
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }

    /// `initialize`, then `notifications/initialized`.
    pub fn connect(&mut self) -> Result<InitializeResult> {
        if self.started {
            return Err(Error::transport(format!(
                "Cannot connect MCP client in {} state",
                if self.closed { "closed" } else { "connecting" }
            )));
        }
        if let Err(error) = self.transport.start() {
            self.close();
            return Err(error);
        }
        self.started = true;
        let mut capabilities = serde_json::Map::new();
        if !self.roots.is_empty() {
            capabilities.insert("roots".into(), serde_json::json!({}));
        }
        let mut client_info = serde_json::Map::new();
        client_info.insert("name".into(), Value::from(self.name.clone()));
        client_info.insert("version".into(), Value::from(self.version.clone()));
        let params = serde_json::json!({
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "capabilities": Value::Object(capabilities),
            "clientInfo": Value::Object(client_info),
        });
        let result = match self.request("initialize", Some(params), self.request_timeout) {
            Ok(result) => parse_initialize(&result)?,
            Err(error) => {
                self.close();
                return Err(error);
            }
        };
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&result.protocol_version.as_str()) {
            self.close();
            return Err(Error::transport(format!(
                "MCP server selected unsupported protocol version {}",
                result.protocol_version
            )));
        }
        self.protocol_version = Some(result.protocol_version.clone());
        self.server_info = Some(result.server_info.clone());
        self.capabilities = result.capabilities.clone();
        self.instructions = result.instructions.clone();
        self.transport
            .set_protocol_version(&result.protocol_version);
        if let Err(error) = self.notify("notifications/initialized", None) {
            self.close();
            return Err(error);
        }
        Ok(result)
    }

    /// Whether the server initialized with the `tools` capability.
    pub fn has_tools(&self) -> bool {
        has_tools(&self.capabilities)
    }

    /// Every tool, following `nextCursor` through all pages.
    pub fn list_tools(&mut self) -> Result<Vec<Tool>> {
        let mut tools: Vec<Tool> = Vec::new();
        let mut cursors: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let params = cursor
                .as_ref()
                .map(|cursor| serde_json::json!({ "cursor": cursor }));
            let result = self.request("tools/list", params, self.request_timeout)?;
            let (page, next) = parse_tools_page(&result)?;
            tools.extend(page);
            let Some(next) = next else {
                return Ok(tools);
            };
            if cursors.contains(&next) {
                return Err(Error::transport(format!(
                    "MCP tools/list returned duplicate cursor: {next}"
                )));
            }
            cursors.push(next.clone());
            cursor = Some(next);
        }
        Err(Error::transport(format!(
            "MCP tools/list exceeded {MAX_LIST_PAGES} pages"
        )))
    }

    pub fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<CallToolResult> {
        let mut params = serde_json::Map::new();
        params.insert("name".into(), Value::from(name));
        if let Some(arguments) = arguments {
            params.insert("arguments".into(), arguments);
        }
        let result = self.request("tools/call", Some(Value::Object(params)), timeout)?;
        parse_call_tool_result(&result)
    }

    pub fn notify(&mut self, method: &str, params: Option<Value>) -> Result<()> {
        if self.closed || !self.started {
            return Err(Error::closed("MCP connection closed"));
        }
        self.transport.send(&jsonrpc::notification(method, params))
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.transport.close();
    }

    pub fn stderr_tail(&self) -> Option<String> {
        self.transport.stderr_tail()
    }

    /// Send a request and read messages until its answer arrives, answering the server's own
    /// requests on the way.
    fn request(&mut self, method: &str, params: Option<Value>, timeout: Duration) -> Result<Value> {
        if self.closed || !self.started {
            return Err(Error::closed("MCP connection closed"));
        }
        let id = Id::Number(self.next_id);
        self.next_id += 1;
        self.transport
            .send(&jsonrpc::request(&id, method, params))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout(timeout.as_millis() as u64));
            }
            let value = self.transport.next_message(remaining)?;
            match jsonrpc::parse(&value)? {
                Message::Response {
                    id: answered,
                    result,
                } if answered == id => return Ok(result),
                Message::ErrorResponse {
                    id: answered,
                    code,
                    message,
                    data,
                } if answered == id => {
                    return Err(Error::Rpc {
                        code,
                        message,
                        data,
                    });
                }
                Message::Response { .. } | Message::ErrorResponse { .. } => continue,
                Message::Request {
                    id: answered,
                    method,
                    params,
                } => self.answer(&answered, &method, params)?,
                Message::Notification { .. } => continue,
            }
        }
    }

    fn answer(&mut self, id: &Id, method: &str, _params: Option<Value>) -> Result<()> {
        match method {
            "ping" => self
                .transport
                .send(&jsonrpc::success(id, serde_json::json!({}))),
            "roots/list" => {
                let roots: Vec<Value> = self.roots.iter().map(Root::to_value).collect();
                self.transport
                    .send(&jsonrpc::success(id, serde_json::json!({ "roots": roots })))
            }
            _ => self.transport.send(&jsonrpc::error_response(
                id,
                jsonrpc::METHOD_NOT_FOUND,
                &format!("Method not found: {method}"),
            )),
        }
    }
}
