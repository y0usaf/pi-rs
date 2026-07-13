//! Port of `providers/openai-codex-responses.ts`.
//! Responses message/event conversion is shared with `openai_responses`;
//! HTTP/SSE and WebSocket transports retain Pi's fallback and continuation semantics.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use pi_rs_ai_types::{
    AssistantMessage, AssistantMessageDiagnostic, AssistantMessageEvent, AssistantRole, Context,
    DiagnosticCode, DiagnosticErrorInfo, Model, ModelThinkingLevel, ProviderResponse, StopReason,
    Tool, Transport, Usage, clamp_thinking_level, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Message as WebSocketMessage, http};

use super::openai_prompt_cache::clamp_openai_prompt_cache_key;
use super::openai_responses::{
    ResponsesEventSource, ResponsesFlavor, convert_responses_messages, process_responses_events,
    process_responses_stream,
};
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::{ProtocolError, merge_header, merge_header_map};
use crate::transport::{
    AssistantMessageEventStream, RetryOptions, RetryPolicy, TransportError,
    create_assistant_message_event_stream, post_with_retry,
};
use crate::util::headers_to_record;

const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_SSE_HEADER_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS: u64 = 15_000;
const SESSION_WEBSOCKET_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexReasoningSummary {
    Auto,
    Concise,
    Detailed,
    Off,
    On,
}

impl CodexReasoningSummary {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Concise => "concise",
            Self::Detailed => "detailed",
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

#[derive(Clone, Default)]
pub struct OpenAICodexResponsesOptions {
    pub base: StreamOptions,
    pub reasoning_effort: Option<ModelThinkingLevel>,
    pub reasoning_summary: Option<CodexReasoningSummary>,
    pub service_tier: Option<String>,
    pub text_verbosity: Option<String>,
}

fn thinking_level_name(level: ModelThinkingLevel) -> &'static str {
    match level {
        ModelThinkingLevel::Off => "none",
        ModelThinkingLevel::Minimal => "minimal",
        ModelThinkingLevel::Low => "low",
        ModelThinkingLevel::Medium => "medium",
        ModelThinkingLevel::High => "high",
        ModelThinkingLevel::XHigh => "xhigh",
        ModelThinkingLevel::Max => "max",
    }
}

fn convert_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": null
            })
        })
        .collect()
}

fn mapped_effort(model: &Model, effort: ModelThinkingLevel) -> Option<String> {
    if let Some(map) = &model.thinking_level_map
        && let Some(mapped) = map.get(&effort)
    {
        return mapped.clone();
    }
    Some(thinking_level_name(effort).to_string())
}

fn build_request_body(
    model: &Model,
    context: &Context,
    options: &OpenAICodexResponsesOptions,
) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(model.id));
    body.insert("store".to_string(), json!(false));
    body.insert("stream".to_string(), json!(true));
    body.insert(
        "instructions".to_string(),
        json!(
            context
                .system_prompt
                .as_deref()
                .filter(|prompt| !prompt.is_empty())
                .unwrap_or("You are a helpful assistant.")
        ),
    );
    body.insert(
        "input".to_string(),
        json!(convert_responses_messages(model, context, false)),
    );
    body.insert(
        "text".to_string(),
        json!({ "verbosity": options.text_verbosity.as_deref().unwrap_or("low") }),
    );
    body.insert(
        "include".to_string(),
        json!(["reasoning.encrypted_content"]),
    );
    if let Some(key) = clamp_openai_prompt_cache_key(options.base.session_id.as_deref()) {
        body.insert("prompt_cache_key".to_string(), json!(key));
    }
    body.insert("tool_choice".to_string(), json!("auto"));
    body.insert("parallel_tool_calls".to_string(), json!(true));
    if let Some(temperature) = options.base.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(service_tier) = &options.service_tier {
        body.insert("service_tier".to_string(), json!(service_tier));
    }
    if let Some(tools) = &context.tools
        && !tools.is_empty()
    {
        body.insert("tools".to_string(), Value::Array(convert_tools(tools)));
    }
    if let Some(effort) = options.reasoning_effort
        && let Some(effort) = mapped_effort(model, effort)
    {
        body.insert(
            "reasoning".to_string(),
            json!({
                "effort": effort,
                "summary": options.reasoning_summary.unwrap_or(CodexReasoningSummary::Auto).as_str()
            }),
        );
    }
    Value::Object(body)
}

fn extract_account_id(token: &str) -> Result<String, ProtocolError> {
    let mut parts = token.split('.');
    let (_header, encoded, signature) = (parts.next(), parts.next(), parts.next());
    let encoded = match (encoded, signature, parts.next()) {
        (Some(encoded), Some(_), None) => encoded,
        _ => {
            return Err(ProtocolError(
                "Failed to extract accountId from token".to_string(),
            ));
        }
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(encoded))
        .map_err(|_| ProtocolError("Failed to extract accountId from token".to_string()))?;
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ProtocolError("Failed to extract accountId from token".to_string()))?;
    payload
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProtocolError("Failed to extract accountId from token".to_string()))
}

fn resolve_codex_url(base_url: &str) -> String {
    let raw = if base_url.trim().is_empty() {
        DEFAULT_CODEX_BASE_URL
    } else {
        base_url.trim()
    };
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

fn insert_headers(entries: Vec<(String, String)>) -> Result<HeaderMap, ProtocolError> {
    let mut headers = HeaderMap::new();
    for (key, value) in entries {
        headers.insert(
            HeaderName::from_bytes(key.as_bytes())
                .map_err(|error| ProtocolError(error.to_string()))?,
            HeaderValue::from_str(&value).map_err(|error| ProtocolError(error.to_string()))?,
        );
    }
    Ok(headers)
}

fn build_sse_headers(
    model: &Model,
    options: &OpenAICodexResponsesOptions,
    account_id: &str,
    token: &str,
) -> Result<HeaderMap, ProtocolError> {
    let mut headers = Vec::new();
    merge_header_map(&mut headers, model.headers.as_ref());
    merge_header_map(&mut headers, options.base.headers.as_ref());
    merge_header(&mut headers, "authorization", &format!("Bearer {token}"));
    merge_header(&mut headers, "chatgpt-account-id", account_id);
    merge_header(&mut headers, "originator", "pi");
    merge_header(&mut headers, "user-agent", "pi (browser)");
    merge_header(&mut headers, "openai-beta", "responses=experimental");
    merge_header(&mut headers, "accept", "text/event-stream");
    merge_header(&mut headers, "content-type", "application/json");
    if let Some(session_id) = &options.base.session_id {
        merge_header(&mut headers, "session-id", session_id);
        merge_header(&mut headers, "x-client-request-id", session_id);
    }
    insert_headers(headers)
}

fn format_http_error(error: &TransportError) -> String {
    let TransportError::Status {
        status,
        status_text,
        body,
    } = error
    else {
        return error.to_string();
    };
    let fallback = if body.is_empty() {
        if status_text.is_empty() {
            "Request failed"
        } else {
            status_text
        }
    } else {
        body
    };
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return fallback.to_string();
    };
    let Some(provider_error) = parsed.get("error") else {
        return fallback.to_string();
    };
    let code = provider_error
        .get("code")
        .or_else(|| provider_error.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if code.to_ascii_lowercase().contains("usage_limit_reached")
        || code.to_ascii_lowercase().contains("usage_not_included")
        || code.to_ascii_lowercase().contains("rate_limit_exceeded")
        || *status == 429
    {
        let plan = provider_error
            .get("plan_type")
            .and_then(Value::as_str)
            .map(|plan| format!(" ({} plan)", plan.to_ascii_lowercase()))
            .unwrap_or_default();
        return format!("You have hit your ChatGPT usage limit{plan}.");
    }
    provider_error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

type CodexWebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Continuation {
    last_request_body: Value,
    last_response_id: String,
    last_response_items: Vec<Value>,
}

struct CachedSocket {
    socket: CodexWebSocket,
    continuation: Option<Continuation>,
    last_used: Instant,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAICodexWebSocketDebugStats {
    pub requests: u64,
    pub connections_created: u64,
    pub connections_reused: u64,
    pub cached_context_requests: u64,
    pub store_true_requests: u64,
    pub full_context_requests: u64,
    pub delta_requests: u64,
    pub last_input_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_delta_input_items: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_previous_response_id: Option<String>,
    pub websocket_failures: u64,
    pub sse_fallbacks: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket_fallback_active: Option<bool>,
    #[serde(rename = "lastWebSocketError", skip_serializing_if = "Option::is_none")]
    pub last_websocket_error: Option<String>,
}

fn websocket_cache() -> &'static Mutex<HashMap<String, CachedSocket>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedSocket>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn websocket_stats() -> &'static Mutex<HashMap<String, OpenAICodexWebSocketDebugStats>> {
    static STATS: OnceLock<Mutex<HashMap<String, OpenAICodexWebSocketDebugStats>>> =
        OnceLock::new();
    STATS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn websocket_fallbacks() -> &'static Mutex<std::collections::HashSet<String>> {
    static FALLBACKS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    FALLBACKS.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

fn mutex_guard<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn get_openai_codex_websocket_debug_stats(
    session_id: &str,
) -> Option<OpenAICodexWebSocketDebugStats> {
    mutex_guard(websocket_stats()).get(session_id).cloned()
}

pub fn reset_openai_codex_websocket_debug_stats(session_id: Option<&str>) {
    match session_id {
        Some(session_id) => {
            mutex_guard(websocket_stats()).remove(session_id);
            mutex_guard(websocket_fallbacks()).remove(session_id);
        }
        None => {
            mutex_guard(websocket_stats()).clear();
            mutex_guard(websocket_fallbacks()).clear();
        }
    }
}

pub fn close_openai_codex_websocket_sessions(session_id: Option<&str>) {
    let mut cache = mutex_guard(websocket_cache());
    match session_id {
        Some(session_id) => {
            cache.remove(session_id);
        }
        None => cache.clear(),
    }
}

fn fallback_active(session_id: Option<&str>) -> bool {
    session_id.is_some_and(|id| mutex_guard(websocket_fallbacks()).contains(id))
}

fn record_fallback(session_id: Option<&str>) {
    let Some(id) = session_id else { return };
    let active = mutex_guard(websocket_fallbacks()).contains(id);
    let mut stats = mutex_guard(websocket_stats());
    let stats = stats.entry(id.to_string()).or_default();
    stats.sse_fallbacks += 1;
    stats.websocket_fallback_active = Some(active);
}

fn record_websocket_failure(session_id: Option<&str>, error: &str) {
    let Some(id) = session_id else { return };
    mutex_guard(websocket_fallbacks()).insert(id.to_string());
    let mut stats = mutex_guard(websocket_stats());
    let stats = stats.entry(id.to_string()).or_default();
    stats.websocket_failures += 1;
    stats.last_websocket_error = Some(error.to_string());
    stats.websocket_fallback_active = Some(true);
}

fn resolve_codex_websocket_url(base_url: &str) -> Result<String, ProtocolError> {
    let mut url = reqwest::Url::parse(&resolve_codex_url(base_url))
        .map_err(|error| ProtocolError(error.to_string()))?;
    let scheme = match url.scheme() {
        "https" => "wss".to_string(),
        "http" => "ws".to_string(),
        other => other.to_string(),
    };
    url.set_scheme(&scheme)
        .map_err(|_| ProtocolError("Invalid Codex WebSocket URL".to_string()))?;
    Ok(url.to_string())
}

fn create_codex_request_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let seed = now_ms().to_be_bytes();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = seed[index % seed.len()] ^ (index as u8).wrapping_mul(31);
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn build_websocket_headers(
    model: &Model,
    options: &OpenAICodexResponsesOptions,
    account_id: &str,
    token: &str,
    request_id: &str,
) -> Result<HeaderMap, ProtocolError> {
    let mut headers = Vec::new();
    merge_header_map(&mut headers, model.headers.as_ref());
    merge_header_map(&mut headers, options.base.headers.as_ref());
    headers.retain(|(key, _)| !matches!(key.as_str(), "accept" | "content-type" | "openai-beta"));
    merge_header(&mut headers, "authorization", &format!("Bearer {token}"));
    merge_header(&mut headers, "chatgpt-account-id", account_id);
    merge_header(&mut headers, "originator", "pi");
    merge_header(&mut headers, "user-agent", "pi (browser)");
    merge_header(
        &mut headers,
        "openai-beta",
        "responses_websockets=2026-02-06",
    );
    merge_header(&mut headers, "x-client-request-id", request_id);
    merge_header(&mut headers, "session-id", request_id);
    insert_headers(headers)
}

async fn connect_websocket(
    url: &str,
    headers: &HeaderMap,
    options: &OpenAICodexResponsesOptions,
) -> Result<CodexWebSocket, ProtocolError> {
    let mut request = url
        .into_client_request()
        .map_err(|error| ProtocolError(error.to_string()))?;
    for (name, value) in headers {
        let name = http::HeaderName::from_bytes(name.as_str().as_bytes())
            .map_err(|error| ProtocolError(error.to_string()))?;
        let value = http::HeaderValue::from_bytes(value.as_bytes())
            .map_err(|error| ProtocolError(error.to_string()))?;
        request.headers_mut().insert(name, value);
    }
    let connect = tokio_tungstenite::connect_async(request);
    let timeout_ms = options
        .base
        .websocket_connect_timeout_ms
        .unwrap_or(DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS);
    let result = match (&options.base.signal, timeout_ms) {
        (Some(signal), 0) => tokio::select! {
            _ = signal.aborted() => return Err(ProtocolError("Request was aborted".to_string())),
            value = connect => value,
        },
        (Some(signal), timeout) => tokio::select! {
            _ = signal.aborted() => return Err(ProtocolError("Request was aborted".to_string())),
            value = tokio::time::timeout(Duration::from_millis(timeout), connect) =>
                value.map_err(|_| ProtocolError(format!("WebSocket connect timeout after {timeout}ms")))?,
        },
        (None, 0) => connect.await,
        (None, timeout) => tokio::time::timeout(Duration::from_millis(timeout), connect)
            .await
            .map_err(|_| ProtocolError(format!("WebSocket connect timeout after {timeout}ms")))?,
    };
    result
        .map(|(socket, _)| socket)
        .map_err(|error| ProtocolError(error.to_string()))
}

fn request_without_input(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        object.shift_remove("input");
        object.shift_remove("previous_response_id");
    }
    body
}

fn cached_request_body(entry: &mut CachedSocket, body: &Value) -> Value {
    let Some(continuation) = &entry.continuation else {
        return body.clone();
    };
    if request_without_input(body) != request_without_input(&continuation.last_request_body) {
        entry.continuation = None;
        return body.clone();
    }
    let current = body
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut baseline = continuation
        .last_request_body
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    baseline.extend(continuation.last_response_items.clone());
    if current.len() < baseline.len() || current[..baseline.len()] != baseline {
        entry.continuation = None;
        return body.clone();
    }
    let mut next = body.clone();
    if let Some(object) = next.as_object_mut() {
        object.insert(
            "previous_response_id".to_string(),
            json!(continuation.last_response_id),
        );
        object.insert(
            "input".to_string(),
            Value::Array(current[baseline.len()..].to_vec()),
        );
    }
    next
}

fn websocket_protocol_error(value: &Value) -> Option<ProtocolError> {
    match value.get("type").and_then(Value::as_str) {
        Some("error") => {
            let code = value.get("code").and_then(Value::as_str).unwrap_or("");
            let message = value.get("message").and_then(Value::as_str).unwrap_or("");
            let detail = if !message.is_empty() {
                message
            } else if !code.is_empty() {
                code
            } else {
                return Some(ProtocolError(format!("Codex error: {value}")));
            };
            Some(ProtocolError(format!("Codex error: {detail}")))
        }
        Some("response.failed") => Some(ProtocolError(
            value
                .pointer("/response/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Codex response failed")
                .to_string(),
        )),
        _ => None,
    }
}

fn websocket_completion(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("response.completed" | "response.done" | "response.incomplete")
    )
}

async fn next_websocket_value(
    socket: &mut CodexWebSocket,
    options: &OpenAICodexResponsesOptions,
) -> Result<Value, ProtocolError> {
    loop {
        let next = socket.next();
        let item = match (&options.base.signal, options.base.timeout_ms) {
            (Some(signal), Some(timeout)) if timeout > 0 => tokio::select! {
                _ = signal.aborted() => return Err(ProtocolError("Request was aborted".to_string())),
                value = tokio::time::timeout(Duration::from_millis(timeout), next) =>
                    value.map_err(|_| ProtocolError(format!("WebSocket idle timeout after {timeout}ms")))?,
            },
            (Some(signal), _) => tokio::select! {
                _ = signal.aborted() => return Err(ProtocolError("Request was aborted".to_string())),
                value = next => value,
            },
            (None, Some(timeout)) if timeout > 0 => {
                tokio::time::timeout(Duration::from_millis(timeout), next)
                    .await
                    .map_err(|_| {
                        ProtocolError(format!("WebSocket idle timeout after {timeout}ms"))
                    })?
            }
            _ => next.await,
        };
        let Some(item) = item else {
            return Err(ProtocolError(
                "WebSocket stream closed before response.completed".to_string(),
            ));
        };
        let message = item.map_err(|error| {
            let text = error.to_string();
            if text.contains("Connection reset without closing handshake") {
                ProtocolError("WebSocket closed 1006".to_string())
            } else {
                ProtocolError(text)
            }
        })?;
        let text = match message {
            WebSocketMessage::Text(text) => text.to_string(),
            WebSocketMessage::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            WebSocketMessage::Close(frame) => {
                let detail = frame
                    .map(|frame| {
                        let code = u16::from(frame.code);
                        let reason = if frame.reason.is_empty() && code == 1009 {
                            "message too big"
                        } else {
                            frame.reason.as_ref()
                        };
                        if reason.is_empty() {
                            format!(" {code}")
                        } else {
                            format!(" {code} {reason}")
                        }
                    })
                    .unwrap_or_default();
                return Err(ProtocolError(format!("WebSocket closed{detail}")));
            }

            WebSocketMessage::Ping(_) | WebSocketMessage::Pong(_) | WebSocketMessage::Frame(_) => {
                continue;
            }
        };
        return serde_json::from_str(&text)
            .map_err(|error| ProtocolError(format!("Invalid Codex WebSocket JSON: {error}")));
    }
}

enum WebSocketFailure {
    Transport { error: ProtocolError, started: bool },
    Protocol(ProtocolError),
}

async fn run_websocket(
    model: &Model,
    options: &OpenAICodexResponsesOptions,
    headers: HeaderMap,
    full_body: &Value,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), WebSocketFailure> {
    let session_id = options.base.session_id.as_deref();
    let cached = session_id.and_then(|id| {
        let mut cache = mutex_guard(websocket_cache());
        cache
            .remove(id)
            .filter(|entry| entry.last_used.elapsed() < SESSION_WEBSOCKET_CACHE_TTL)
    });
    let reused = cached.is_some();
    let mut entry = match cached {
        Some(entry) => entry,
        None => CachedSocket {
            socket: connect_websocket(
                &resolve_codex_websocket_url(&model.base_url).map_err(|error| {
                    WebSocketFailure::Transport {
                        error,
                        started: false,
                    }
                })?,
                &headers,
                options,
            )
            .await
            .map_err(|error| WebSocketFailure::Transport {
                error,
                started: false,
            })?,
            continuation: None,
            last_used: Instant::now(),
        },
    };

    let use_cached = matches!(
        options.base.transport,
        None | Some(Transport::Auto | Transport::WebsocketCached)
    );
    let request_body = if use_cached {
        cached_request_body(&mut entry, full_body)
    } else {
        full_body.clone()
    };
    if let Some(id) = session_id {
        let mut all = mutex_guard(websocket_stats());
        let stats = all.entry(id.to_string()).or_default();
        stats.requests += 1;
        if reused {
            stats.connections_reused += 1
        } else {
            stats.connections_created += 1
        }
        if use_cached {
            stats.cached_context_requests += 1
        }
        if request_body.get("store") == Some(&Value::Bool(true)) {
            stats.store_true_requests += 1
        }
        stats.last_input_items = request_body
            .get("input")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if let Some(previous) = request_body
            .get("previous_response_id")
            .and_then(Value::as_str)
        {
            stats.delta_requests += 1;
            stats.last_delta_input_items = Some(stats.last_input_items);
            stats.last_previous_response_id = Some(previous.to_string());
        } else {
            stats.full_context_requests += 1;
            stats.last_delta_input_items = None;
            stats.last_previous_response_id = None;
        }
    }
    let mut envelope = request_body.clone();
    if let Some(object) = envelope.as_object_mut() {
        object.insert("type".to_string(), json!("response.create"));
        let type_value = object
            .shift_remove("type")
            .unwrap_or_else(|| json!("response.create"));
        let rest = std::mem::take(object);
        object.insert("type".to_string(), type_value);
        object.extend(rest);
    }
    entry
        .socket
        .send(WebSocketMessage::Text(envelope.to_string().into()))
        .await
        .map_err(|error| WebSocketFailure::Transport {
            error: ProtocolError(error.to_string()),
            started: false,
        })?;
    let first = next_websocket_value(&mut entry.socket, options)
        .await
        .map_err(|error| {
            if error.0.starts_with("Invalid Codex WebSocket JSON:") {
                WebSocketFailure::Protocol(error)
            } else {
                WebSocketFailure::Transport {
                    error,
                    started: false,
                }
            }
        })?;
    if let Some(error) = websocket_protocol_error(&first) {
        return Err(WebSocketFailure::Protocol(error));
    }

    let first_completes = websocket_completion(&first);
    stream.push(AssistantMessageEvent::Start {
        partial: output.clone(),
    });
    let (sender, receiver) = tokio::sync::mpsc::channel(32);
    sender
        .send(Ok(first))
        .await
        .map_err(|_| WebSocketFailure::Transport {
            error: ProtocolError("WebSocket stream closed".to_string()),
            started: true,
        })?;
    let mut socket = entry.socket;
    let task_options = options.clone();
    let reader = tokio::spawn(async move {
        if !first_completes {
            loop {
                let value = next_websocket_value(&mut socket, &task_options).await?;
                if let Some(error) = websocket_protocol_error(&value) {
                    let _ = sender.send(Err(error)).await;
                    return Err(ProtocolError("Codex WebSocket protocol error".to_string()));
                }
                let complete = websocket_completion(&value);
                if sender.send(Ok(value)).await.is_err() {
                    return Err(ProtocolError(
                        "WebSocket stream consumer closed".to_string(),
                    ));
                }
                if complete {
                    break;
                }
            }
        }
        Ok(socket)
    });
    let mut source = ResponsesEventSource::Values(receiver);
    let process = process_responses_events(
        &mut source,
        output,
        stream,
        model,
        options.service_tier.as_deref(),
        ResponsesFlavor::Codex,
    )
    .await;
    if let Err(error) = process {
        reader.abort();
        return Err(WebSocketFailure::Protocol(error));
    }
    let socket = reader
        .await
        .map_err(|error| WebSocketFailure::Transport {
            error: ProtocolError(error.to_string()),
            started: true,
        })?
        .map_err(|error| WebSocketFailure::Transport {
            error,
            started: true,
        })?;
    entry.socket = socket;
    entry.last_used = Instant::now();
    if use_cached && let Some(response_id) = output.response_id.clone() {
        let response_items = convert_responses_messages(
            model,
            &Context {
                system_prompt: None,
                messages: vec![pi_rs_ai_types::Message::Assistant(output.clone())],
                tools: None,
            },
            false,
        )
        .into_iter()
        .filter(|item| item.get("type").and_then(Value::as_str) != Some("function_call_output"))
        .collect();
        entry.continuation = Some(Continuation {
            last_request_body: full_body.clone(),
            last_response_id: response_id,
            last_response_items: response_items,
        });
    }
    if let Some(id) = session_id {
        let id = id.to_string();
        let last_used = entry.last_used;
        mutex_guard(websocket_cache()).insert(id.clone(), entry);
        tokio::spawn(async move {
            tokio::time::sleep(SESSION_WEBSOCKET_CACHE_TTL).await;
            let mut cache = mutex_guard(websocket_cache());
            if cache
                .get(&id)
                .is_some_and(|entry| entry.last_used == last_used)
            {
                cache.remove(&id);
            }
        });
    } else {
        let _ = entry.socket.close(None).await;
    }
    Ok(())
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &OpenAICodexResponsesOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let token = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let account_id = extract_account_id(token)?;
    let mut body = build_request_body(model, context, options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(body.clone(), model.clone()).await
    {
        body = next;
    }
    let transport = options.base.transport.unwrap_or(Transport::Auto);
    let session_id = options.base.session_id.as_deref();
    let disabled = transport != Transport::Sse && fallback_active(session_id);
    if disabled {
        record_fallback(session_id);
    }
    if transport != Transport::Sse && !disabled {
        let generated_request_id;
        let request_id = match session_id {
            Some(session_id) => session_id,
            None => {
                generated_request_id = create_codex_request_id();
                &generated_request_id
            }
        };
        let websocket_headers =
            build_websocket_headers(model, options, &account_id, token, request_id)?;
        match run_websocket(model, options, websocket_headers, &body, stream, output).await {
            Ok(()) => return Ok(()),
            Err(WebSocketFailure::Protocol(error)) => return Err(error),
            Err(WebSocketFailure::Transport { error, started }) => {
                if options
                    .base
                    .signal
                    .as_ref()
                    .is_some_and(crate::transport::AbortSignal::is_aborted)
                {
                    return Err(error);
                }
                let mut details = Map::new();
                details.insert(
                    "configuredTransport".to_string(),
                    json!(match transport {
                        Transport::Sse => "sse",
                        Transport::Websocket => "websocket",
                        Transport::WebsocketCached => "websocket-cached",
                        Transport::Auto => "auto",
                    }),
                );
                if !started {
                    details.insert("fallbackTransport".to_string(), json!("sse"));
                }
                details.insert("eventsEmitted".to_string(), json!(started));
                details.insert(
                    "phase".to_string(),
                    json!(if started {
                        "after_message_stream_start"
                    } else {
                        "before_message_stream_start"
                    }),
                );
                details.insert("requestBytes".to_string(), json!(body.to_string().len()));
                let error_message = error.to_string();
                let close_code = error_message
                    .strip_prefix("WebSocket closed ")
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|code| code.parse::<u64>().ok());
                output.append_diagnostic(AssistantMessageDiagnostic::new(
                    "provider_transport_failure",
                    DiagnosticErrorInfo {
                        name: Some(
                            if close_code.is_some() {
                                "WebSocketCloseError"
                            } else {
                                "Error"
                            }
                            .to_string(),
                        ),
                        message: error_message,
                        stack: None,
                        code: close_code
                            .map(|code| DiagnosticCode::Number(serde_json::Number::from(code))),
                    },
                    Some(details),
                ));

                record_websocket_failure(session_id, &error.to_string());
                record_fallback(session_id);
                if started {
                    return Err(error);
                }
            }
        }
    }
    let headers = build_sse_headers(model, options, &account_id, token)?;
    let response = post_with_retry(
        &reqwest::Client::new(),
        &resolve_codex_url(&model.base_url),
        &headers,
        &body.to_string(),
        &RetryOptions {
            max_retries: options.base.max_retries.unwrap_or(0),
            max_retry_delay_ms: options.base.max_retry_delay_ms,
            header_timeout_ms: DEFAULT_SSE_HEADER_TIMEOUT_MS,
            policy: RetryPolicy::Codex,
        },
        options.base.signal.as_ref(),
    )
    .await
    .map_err(|error| ProtocolError(format_http_error(&error)))?;
    if let Some(hook) = &options.base.on_response {
        hook(
            ProviderResponse {
                status: response.status().as_u16(),
                headers: headers_to_record(response.headers()),
            },
            model.clone(),
        )
        .await;
    }
    stream.push(AssistantMessageEvent::Start {
        partial: output.clone(),
    });
    process_responses_stream(
        response,
        options.base.signal.clone(),
        output,
        stream,
        model,
        options.service_tier.as_deref(),
        ResponsesFlavor::Codex,
    )
    .await?;
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(crate::transport::AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request was aborted".to_string()));
    }
    Ok(())
}

pub fn stream_openai_codex_responses(
    model: &Model,
    context: &Context,
    options: Option<OpenAICodexResponsesOptions>,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let task_stream = stream.clone();
    let model = model.clone();
    let context = context.clone();
    let options = options.unwrap_or_default();
    tokio::spawn(async move {
        let mut output = AssistantMessage {
            role: AssistantRole::Assistant,
            content: Vec::new(),
            api: model.api.clone(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            response_model: None,
            response_id: None,
            diagnostics: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: now_ms(),
        };
        match drive(&model, &context, &options, &task_stream, &mut output).await {
            Ok(()) => task_stream.push(AssistantMessageEvent::Done {
                reason: output.stop_reason,
                message: output,
            }),
            Err(error) => {
                output.stop_reason = if options
                    .base
                    .signal
                    .as_ref()
                    .is_some_and(crate::transport::AbortSignal::is_aborted)
                {
                    StopReason::Aborted
                } else {
                    StopReason::Error
                };
                output.error_message = Some(error.to_string());
                task_stream.push(AssistantMessageEvent::Error {
                    reason: output.stop_reason,
                    error: output,
                });
            }
        }
        task_stream.end();
    });
    stream
}

pub fn stream_simple_openai_codex_responses(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let api_key = options
        .as_ref()
        .and_then(|options| options.base.api_key.as_deref())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let reasoning_effort = options
        .as_ref()
        .and_then(|options| options.reasoning)
        .map(|level| clamp_thinking_level(model, ModelThinkingLevel::from(level)));
    Ok(stream_openai_codex_responses(
        model,
        context,
        Some(OpenAICodexResponsesOptions {
            base: build_base_options(model, options.as_ref(), Some(api_key)),
            reasoning_effort,
            ..Default::default()
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_url_shapes_match_spec() {
        assert_eq!(resolve_codex_url("https://x"), "https://x/codex/responses");
        assert_eq!(
            resolve_codex_url("https://x/codex"),
            "https://x/codex/responses"
        );
        assert_eq!(
            resolve_codex_url("https://x/codex/responses/"),
            "https://x/codex/responses"
        );
    }
}
