//! Replay + request-shaping parity tests for
//! `protocols::openai_completions` against the spec's
//! `providers/openai-completions.ts`, run over a loopback raw-TCP HTTP
//! server (sandbox-safe). Fixture provenance:
//! `tests/fixtures/openai-completions/README.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use pi_rs_ai::protocols::openai_completions::{
    OpenAICompletionsOptions, OpenAIToolChoice, stream_openai_completions,
    stream_simple_openai_completions,
};
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai_types::{
    AssistantMessage, AssistantMessageEvent, Context, Model, ThinkingLevel, Usage, calculate_cost,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------------
// Loopback SSE server with request capture
// ---------------------------------------------------------------------

type Captured = Arc<Mutex<Vec<String>>>;

fn sse_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Serve one canned response per connection, capturing raw requests.
fn serve(responses: Vec<String>) -> (SocketAddr, Captured) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let capture = Arc::clone(&captured);
    tokio::spawn(async move {
        let mut responses = responses.into_iter();
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => return,
            };
            let request = read_request(&mut sock).await;
            capture.lock().unwrap().push(request);
            match responses.next() {
                Some(response) => {
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.shutdown().await;
                }
                None => return,
            }
        }
    });
    (addr, captured)
}

/// Read one HTTP request: headers plus a content-length body.
async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let content_length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while buf.len() - (pos + 4) < content_length {
                let n = match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    let head = request.split("\r\n\r\n").next().unwrap_or("");
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.eq_ignore_ascii_case(name)).then_some(value.trim())
    })
}

fn request_body(request: &str) -> Value {
    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
    serde_json::from_str(body).unwrap()
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/openai-completions/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

fn openai_model(addr: SocketAddr) -> Model {
    serde_json::from_value(json!({
        "id": "gpt-5-mini",
        "name": "GPT-5 Mini",
        "api": "openai-completions",
        "provider": "openai",
        "baseUrl": format!("http://{addr}"),
        "reasoning": true,
        "input": ["text", "image"],
        "cost": { "input": 0.25, "output": 2, "cacheRead": 0.025, "cacheWrite": 0 },
        "contextWindow": 400000,
        "maxTokens": 128000
    }))
    .unwrap()
}

fn model_for(addr: SocketAddr, provider: &str, id: &str) -> Model {
    serde_json::from_value(json!({
        "id": id,
        "name": id,
        "api": "openai-completions",
        "provider": provider,
        "baseUrl": format!("http://{addr}"),
        "reasoning": true,
        "input": ["text"],
        "cost": { "input": 1, "output": 2, "cacheRead": 0.1, "cacheWrite": 0 },
        "contextWindow": 128000,
        "maxTokens": 8192
    }))
    .unwrap()
}

fn user_context(text: &str) -> Context {
    serde_json::from_value(json!({
        "messages": [{ "role": "user", "content": text, "timestamp": 1 }]
    }))
    .unwrap()
}

fn api_key_options(key: &str) -> OpenAICompletionsOptions {
    OpenAICompletionsOptions {
        base: StreamOptions {
            api_key: Some(key.to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Drain the stream: (events, final message from `result()`).
async fn collect(
    stream: &pi_rs_ai::transport::AssistantMessageEventStream,
) -> (Vec<AssistantMessageEvent>, Option<AssistantMessage>) {
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    let result = stream.result().await;
    (events, result)
}

/// Event JSON minus the `partial`/`message`/`error` snapshots.
fn event_summary(event: &AssistantMessageEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap();
    if let Some(map) = value.as_object_mut() {
        map.remove("partial");
        map.remove("message");
        map.remove("error");
    }
    value
}

/// A minimal complete transcript: one text delta, then a finish reason.
fn minimal_transcript(finish_reason: &str) -> String {
    format!(
        concat!(
            "data: {{\"id\":\"chatcmpl-9\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"ok\"}},\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"chatcmpl-9\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"{reason}\"}}]}}\n\n",
            "data: [DONE]\n\n",
        ),
        reason = finish_reason
    )
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn replays_reasoning_text_and_tool_calls() {
    let (addr, captured) = serve(vec![sse_response(&fixture("replay_basic.sse"))]);
    let model = openai_model(addr);

    let stream = stream_openai_completions(
        &model,
        &user_context("hi"),
        Some(api_key_options("sk-test")),
    );
    let (events, result) = collect(&stream).await;

    // Event sequence parity.
    let summaries: Vec<Value> = events.iter().map(event_summary).collect();
    let expected_events: Value =
        serde_json::from_str(&fixture("replay_basic.events.json")).unwrap();
    assert_eq!(Value::Array(summaries), expected_events);

    // Final message parity (timestamp normalized, cost derived).
    let mut message = result.unwrap();
    message.timestamp = 0;
    let mut expected: Value = serde_json::from_str(&fixture("replay_basic.message.json")).unwrap();
    let mut usage: Usage = serde_json::from_value(expected["usage"].clone()).unwrap();
    let cost = calculate_cost(&model, &mut usage);
    expected["usage"]["cost"] = serde_json::to_value(cost).unwrap();
    assert_eq!(serde_json::to_value(&message).unwrap(), expected);

    // SDK-reproduced request surface: URL path and Bearer auth.
    let request = captured.lock().unwrap().remove(0);
    assert!(request.starts_with("POST /chat/completions HTTP/1.1\r\n"));
    assert_eq!(
        request_header(&request, "authorization"),
        Some("Bearer sk-test")
    );
    let body = request_body(&request);
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["stream_options"], json!({ "include_usage": true }));
    assert_eq!(body["store"], json!(false));
}

#[tokio::test]
async fn builds_full_api_key_params() {
    let (addr, captured) = serve(vec![sse_response(&minimal_transcript("stop"))]);
    let model = openai_model(addr);

    let context: Context = serde_json::from_value(json!({
        "systemPrompt": "You are helpful.",
        "messages": [
            { "role": "user", "content": "Hello", "timestamp": 1 },
            {
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "Consider.", "thinkingSignature": "reasoning_content" },
                    { "type": "text", "text": "Reading." },
                    { "type": "toolCall", "id": "call_1", "name": "read", "arguments": { "path": "a.txt" } }
                ],
                "api": "openai-completions",
                "provider": "openai",
                "model": "gpt-5-mini",
                "usage": {
                    "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 }
                },
                "stopReason": "toolUse",
                "timestamp": 1
            },
            {
                "role": "toolResult",
                "toolCallId": "call_1",
                "toolName": "read",
                "content": [{ "type": "text", "text": "file contents" }],
                "isError": false,
                "timestamp": 1
            },
            { "role": "user", "content": "Continue", "timestamp": 1 }
        ],
        "tools": [{
            "name": "read",
            "description": "Read a file",
            "parameters": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        }]
    }))
    .unwrap();

    let mut options = api_key_options("sk-test");
    options.base.temperature = Some(0.5);
    options.base.max_tokens = Some(1000);
    options.base.session_id = Some("sess-1".to_string());
    options.tool_choice = Some(OpenAIToolChoice::Auto);
    options.reasoning_effort = Some(ThinkingLevel::High);

    let stream = stream_openai_completions(&model, &context, Some(options));
    let (_events, result) = collect(&stream).await;
    assert_eq!(result.unwrap().error_message, None);

    let request = captured.lock().unwrap().remove(0);
    let expected: Value = serde_json::from_str(&fixture("params_apikey.request.json")).unwrap();
    assert_eq!(request_body(&request), expected);
    // Loopback baseUrl is not api.openai.com and retention is short:
    // no prompt_cache_key, no session affinity headers.
    assert_eq!(request_header(&request, "session_id"), None);
    assert_eq!(request_header(&request, "x-session-affinity"), None);
}

#[tokio::test]
async fn deepseek_thinking_format_and_reasoning_content() {
    let (addr, captured) = serve(vec![sse_response(&minimal_transcript("stop"))]);
    let model = model_for(addr, "deepseek", "deepseek-reasoner");

    let context: Context = serde_json::from_value(json!({
        "messages": [
            { "role": "user", "content": "Hello", "timestamp": 1 },
            {
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hi." }],
                "api": "openai-completions",
                "provider": "deepseek",
                "model": "deepseek-reasoner",
                "usage": {
                    "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 }
                },
                "stopReason": "stop",
                "timestamp": 1
            },
            { "role": "user", "content": "More", "timestamp": 1 }
        ]
    }))
    .unwrap();

    let mut options = api_key_options("sk-test");
    options.reasoning_effort = Some(ThinkingLevel::Medium);
    let stream = stream_openai_completions(&model, &context, Some(options));
    let (_events, result) = collect(&stream).await;
    assert_eq!(result.unwrap().error_message, None);

    let body = request_body(&captured.lock().unwrap().remove(0));
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(body["reasoning_effort"], json!("medium"));
    // requiresReasoningContentOnAssistantMessages: deepseek assistant
    // messages carry an empty reasoning_content.
    assert_eq!(body["messages"][1]["reasoning_content"], json!(""));
    assert_eq!(body["messages"][1]["content"], json!("Hi."));
}

#[tokio::test]
async fn openrouter_anthropic_models_get_cache_control() {
    let (addr, captured) = serve(vec![sse_response(&minimal_transcript("stop"))]);
    let model = model_for(addr, "openrouter", "anthropic/claude-sonnet-4.5");

    let context: Context = serde_json::from_value(json!({
        "systemPrompt": "Sys",
        "messages": [{ "role": "user", "content": "hi", "timestamp": 1 }],
        "tools": [{
            "name": "read",
            "description": "Read a file",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }]
    }))
    .unwrap();

    let stream = stream_openai_completions(&model, &context, Some(api_key_options("sk-or")));
    let (_events, result) = collect(&stream).await;
    assert_eq!(result.unwrap().error_message, None);

    let body = request_body(&captured.lock().unwrap().remove(0));
    let cache_control = json!({ "type": "ephemeral" });
    // System prompt: developer role (openrouter anthropic/ models) and
    // string content converted to a cached text part.
    assert_eq!(
        body["messages"][0],
        json!({
            "role": "developer",
            "content": [{ "type": "text", "text": "Sys", "cache_control": cache_control }]
        })
    );
    // Last tool and last user message get cache_control.
    assert_eq!(body["tools"][0]["cache_control"], cache_control);
    assert_eq!(
        body["messages"][1]["content"],
        json!([{ "type": "text", "text": "hi", "cache_control": cache_control }])
    );
    // OpenRouter thinking format, no effort, no map: effort "none".
    assert_eq!(body["reasoning"], json!({ "effort": "none" }));
}

#[tokio::test]
async fn moonshot_compat_and_choice_usage_fallback() {
    let transcript = concat!(
        "data: {\"id\":\"chatcmpl-m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\",\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (addr, captured) = serve(vec![sse_response(transcript)]);
    let model = model_for(addr, "moonshotai", "kimi-k2");

    let context: Context = serde_json::from_value(json!({
        "messages": [{ "role": "user", "content": "hi", "timestamp": 1 }],
        "tools": [{
            "name": "read",
            "description": "Read a file",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }]
    }))
    .unwrap();

    let mut options = api_key_options("sk-m");
    options.base.max_tokens = Some(500);
    options.reasoning_effort = Some(ThinkingLevel::High);
    let stream = stream_openai_completions(&model, &context, Some(options));
    let (_events, result) = collect(&stream).await;
    let message = result.unwrap();
    assert_eq!(message.error_message, None);

    // Usage arrived in choice.usage (Moonshot fallback).
    assert_eq!(message.usage.input, 7);
    assert_eq!(message.usage.output, 3);

    let body = request_body(&captured.lock().unwrap().remove(0));
    // Moonshot: max_tokens (not max_completion_tokens), no store, no
    // strict, no reasoning_effort.
    assert_eq!(body["max_tokens"], json!(500));
    assert_eq!(body.get("max_completion_tokens"), None);
    assert_eq!(body.get("store"), None);
    assert_eq!(body["tools"][0]["function"].get("strict"), None);
    assert_eq!(body.get("reasoning_effort"), None);
}

#[tokio::test]
async fn tool_history_without_tools_sends_empty_tools() {
    let (addr, captured) = serve(vec![sse_response(&minimal_transcript("stop"))]);
    let model = openai_model(addr);

    let context: Context = serde_json::from_value(json!({
        "messages": [
            { "role": "user", "content": "run", "timestamp": 1 },
            {
                "role": "assistant",
                "content": [{ "type": "toolCall", "id": "call_2", "name": "bash", "arguments": {} }],
                "api": "openai-completions",
                "provider": "openai",
                "model": "gpt-5-mini",
                "usage": {
                    "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 }
                },
                "stopReason": "toolUse",
                "timestamp": 1
            },
            {
                "role": "toolResult",
                "toolCallId": "call_2",
                "toolName": "bash",
                "content": [{ "type": "text", "text": "done" }],
                "isError": false,
                "timestamp": 1
            }
        ]
    }))
    .unwrap();

    let stream = stream_openai_completions(&model, &context, Some(api_key_options("sk-test")));
    let (_events, result) = collect(&stream).await;
    assert_eq!(result.unwrap().error_message, None);

    let body = request_body(&captured.lock().unwrap().remove(0));
    assert_eq!(body["tools"], json!([]));
}

#[tokio::test]
async fn stream_without_finish_reason_errors() {
    let transcript = concat!(
        "data: {\"id\":\"chatcmpl-x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (addr, _captured) = serve(vec![sse_response(transcript)]);
    let model = openai_model(addr);

    let stream = stream_openai_completions(
        &model,
        &user_context("hi"),
        Some(api_key_options("sk-test")),
    );
    let (events, result) = collect(&stream).await;
    assert_eq!(
        result.unwrap().error_message.as_deref(),
        Some("Stream ended without finish_reason")
    );
    assert!(matches!(
        events.last(),
        Some(AssistantMessageEvent::Error { .. })
    ));
}

#[tokio::test]
async fn error_stop_reasons_fold_into_error_events() {
    let (addr, _captured) = serve(vec![sse_response(&minimal_transcript("content_filter"))]);
    let model = openai_model(addr);

    let stream = stream_openai_completions(
        &model,
        &user_context("hi"),
        Some(api_key_options("sk-test")),
    );
    let (_events, result) = collect(&stream).await;
    assert_eq!(
        result.unwrap().error_message.as_deref(),
        Some("Provider finish_reason: content_filter")
    );
}

#[tokio::test]
async fn simple_reasoning_maps_to_reasoning_effort() {
    let (addr, captured) = serve(vec![sse_response(&minimal_transcript("stop"))]);
    let model = openai_model(addr);

    let options = SimpleStreamOptions {
        base: StreamOptions {
            api_key: Some("sk-test".to_string()),
            ..Default::default()
        },
        reasoning: Some(ThinkingLevel::High),
        thinking_budgets: None,
    };
    let stream =
        stream_simple_openai_completions(&model, &user_context("hi"), Some(options)).unwrap();
    let (_events, result) = collect(&stream).await;
    assert_eq!(result.unwrap().error_message, None);

    let body = request_body(&captured.lock().unwrap().remove(0));
    assert_eq!(body["reasoning_effort"], json!("high"));
}

#[tokio::test]
async fn missing_api_key_is_an_error() {
    let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let model = openai_model(addr);

    // streamOpenAICompletions: folded into the error event.
    let stream = stream_openai_completions(&model, &user_context("hi"), None);
    let (_events, result) = collect(&stream).await;
    assert_eq!(
        result.unwrap().error_message.as_deref(),
        Some("No API key for provider: openai")
    );

    // streamSimpleOpenAICompletions: the spec throws synchronously.
    let error = stream_simple_openai_completions(&model, &user_context("hi"), None)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "No API key for provider: openai");
}
