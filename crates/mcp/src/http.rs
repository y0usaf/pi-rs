//! `transports/streamable-http.ts`: a server reached over HTTP, answering a request with JSON or
//! with an SSE stream. The server-to-client GET stream is not opened, so the server's own
//! notifications (tools changing, log messages) do not arrive.

use std::collections::VecDeque;

use pi_ai::http::{self, Request};
use pi_ai::sse::{SseEvent, SseReader};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::transport::Transport;

const ERROR_MESSAGE_BODY_CHARS: usize = 500;

pub struct StreamableHttpTransport {
    url: String,
    headers: Vec<(String, String)>,
    session_id: Option<String>,
    protocol_version: Option<String>,
    queue: VecDeque<Value>,
    closed: bool,
    request_timeout: Option<std::time::Duration>,
}

impl StreamableHttpTransport {
    pub fn new(url: impl Into<String>, headers: Vec<(String, String)>) -> Self {
        Self {
            url: url.into(),
            headers,
            session_id: None,
            protocol_version: None,
            queue: VecDeque::new(),
            closed: false,
            request_timeout: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Option<std::time::Duration>) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn post(&mut self, message: &Value) -> Result<()> {
        if self.closed {
            return Err(Error::closed("MCP connection closed"));
        }
        let mut request = Request::new("POST", self.url.clone())
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json");
        for (name, value) in &self.headers {
            request = request.header(name, value.clone());
        }
        if let Some(session) = &self.session_id {
            request = request.header("mcp-session-id", session.clone());
        }
        if let Some(version) = &self.protocol_version {
            request = request.header("mcp-protocol-version", version.clone());
        }
        request.body = serde_json::to_vec(message).map_err(|e| Error::transport(e.to_string()))?;
        let response =
            http::send(&request, None).map_err(|error| Error::transport(error.to_string()))?;
        if let Some(session) = response.header("mcp-session-id") {
            self.session_id = Some(session.to_owned());
        }
        let status = response.status;
        let content_type = response.header("content-type").map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });
        if !(200..300).contains(&status) {
            let body = response.text().unwrap_or_default();
            return Err(http_error(status, &body, self.session_id.is_some()));
        }
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let Some(id) = id else {
            let _ = response.text();
            return Ok(());
        };
        if status == 202 || status == 204 {
            let _ = response.text();
            return Err(Error::Http {
                status,
                message: format!("MCP server accepted request {method} without a response"),
            });
        }
        match content_type.as_deref() {
            Some("application/json") => {
                let body = response
                    .text()
                    .map_err(|error| Error::transport(error.to_string()))?;
                self.push_json(&body)?;
                Ok(())
            }
            Some("text/event-stream") => {
                self.read_stream(response.into_reader(), &id);
                Ok(())
            }
            Some(other) => Err(Error::Http {
                status,
                message: format!("Unsupported MCP response content type: {other}"),
            }),
            None => Err(Error::Http {
                status,
                message: "Unsupported MCP response content type: missing".into(),
            }),
        }
    }

    fn push_json(&mut self, body: &str) -> Result<()> {
        let value: Value =
            serde_json::from_str(body).map_err(|error| Error::transport(error.to_string()))?;
        match value {
            Value::Array(messages) => self.queue.extend(messages),
            other => self.queue.push_back(other),
        }
        Ok(())
    }

    /// Read the SSE stream answering one request, up to the message carrying its id.
    fn read_stream(&mut self, reader: impl std::io::BufRead, id: &Value) {
        let mut answered = false;
        for event in SseReader::new(reader) {
            let Ok(SseEvent { event, data, .. }) = event else {
                return;
            };
            if data.trim().is_empty() || event.is_some_and(|name| name != "message") {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let is_answer = message
                .get("id")
                .is_some_and(|answered_id| answered_id == id);
            self.queue.push_back(message);
            if is_answer {
                answered = true;
                break;
            }
        }
        if !answered {
            self.queue.push_back(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id.clone(),
                "error": {
                    "code": crate::jsonrpc::INTERNAL_ERROR,
                    "message": "MCP response stream failed: stream ended without a response",
                },
            }));
        }
    }
}

fn http_error(status: u16, body: &str, has_session: bool) -> Error {
    if status == 401 {
        return Error::Http {
            status,
            message: "MCP server requires authentication".into(),
        };
    }
    if status == 404 && has_session {
        return Error::Http {
            status,
            message: "MCP session expired".into(),
        };
    }
    let text = body.trim();
    let snippet = if text.chars().count() > ERROR_MESSAGE_BODY_CHARS {
        let mut cut: String = text.chars().take(ERROR_MESSAGE_BODY_CHARS - 3).collect();
        cut.push_str("...");
        cut
    } else {
        text.to_owned()
    };
    Error::Http {
        status,
        message: if snippet.is_empty() {
            format!("MCP HTTP request failed with status {status}")
        } else {
            format!("MCP HTTP request failed with status {status}: {snippet}")
        },
    }
}

impl Transport for StreamableHttpTransport {
    fn start(&mut self) -> Result<()> {
        Ok(())
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        self.post(message)
    }

    fn next_message(&mut self, _timeout: std::time::Duration) -> Result<Value> {
        self.queue
            .pop_front()
            .ok_or_else(|| Error::transport("MCP response was not received"))
    }

    fn set_protocol_version(&mut self, version: &str) {
        self.protocol_version = Some(version.to_owned());
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let Some(session) = self.session_id.clone() else {
            return;
        };
        let mut request = Request::new("DELETE", self.url.clone())
            .timeout(Some(std::time::Duration::from_secs(1)));
        for (name, value) in &self.headers {
            request = request.header(name, value.clone());
        }
        request = request.header("mcp-session-id", session);
        if let Some(version) = &self.protocol_version {
            request = request.header("mcp-protocol-version", version.clone());
        }
        if let Ok(response) = http::send(&request, None) {
            let _ = response.text();
        }
    }
}
