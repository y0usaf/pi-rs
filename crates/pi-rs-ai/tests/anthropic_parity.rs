//! Differential anthropic-protocol harness (PLAN 4.1).
//!
//! `tests/anthropic-parity/oracle.json` is a reviewed capture from Pi's real
//! `streamAnthropic`/`streamSimpleAnthropic` (ref/pi @ c5582102, vendored
//! `@anthropic-ai/sdk` 0.91.1) against a scripted local HTTP stub. This test
//! replays `tests/anthropic-parity/cases.json` through pi-rs's public protocol
//! surface and compares, per case: every captured HTTP request (method,
//! path, meaningful headers, body), the emitted event sequence (without
//! partial/message snapshots), and the final message from `result()`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pi_rs_ai::protocols::anthropic::{
    AnthropicOptions, AnthropicThinkingDisplay, AnthropicToolChoice, stream_anthropic,
    stream_simple_anthropic,
};
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai::transport::AbortSignal;
use pi_rs_ai_types::{Context, Model};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------------
// Scripted loopback server (sandbox-safe raw TCP)
// ---------------------------------------------------------------------

type Captured = Arc<Mutex<Vec<String>>>;

/// Render one scripted response from cases.json to raw HTTP bytes plus
/// its hang flag.
fn scripted_response(response: &Value, shared_sse: &Value) -> (String, bool) {
    let status = response["status"].as_u64().unwrap();
    let hang = response["hang"].as_bool().unwrap_or(false);
    let events = response
        .get("sse")
        .and_then(Value::as_str)
        .map(|name| shared_sse[name].clone())
        .or_else(|| response.get("events").cloned());
    let (body, content_type) = if let Some(events) = events {
        let body: String = events
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {}\n\n",
                    event["event"].as_str().unwrap(),
                    serde_json::to_string(&event["data"]).unwrap()
                )
            })
            .collect();
        (body, "text/event-stream")
    } else if let Some(json_body) = response.get("json") {
        (
            serde_json::to_string(json_body).unwrap(),
            "application/json",
        )
    } else {
        (
            response
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            "text/plain",
        )
    };
    let mut head = format!("HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\n");
    if let Some(headers) = response.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            head.push_str(&format!("{name}: {}\r\n", value.as_str().unwrap()));
        }
    }
    if hang {
        // No content-length: the body runs until connection close,
        // which never comes — the client must abort.
        (format!("{head}\r\n{body}"), true)
    } else {
        (
            format!(
                "{head}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            ),
            false,
        )
    }
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

/// Serve the case's scripted responses (one connection per request,
/// last response repeated like the oracle driver), capturing raw
/// requests. Hanging sockets are parked so they stay open.
fn serve(responses: Vec<(String, bool)>) -> (std::net::SocketAddr, Captured) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let capture = Arc::clone(&captured);
    tokio::spawn(async move {
        let mut index = 0usize;
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => return,
            };
            let request = read_request(&mut sock).await;
            capture.lock().unwrap().push(request);
            let Some((response, hang)) = responses.get(index).or_else(|| responses.last()).cloned()
            else {
                return;
            };
            index += 1;
            let _ = sock.write_all(response.as_bytes()).await;
            if hang {
                // Park the connection; dropped when the runtime ends.
                tokio::spawn(async move {
                    let mut sink = [0u8; 64];
                    let _ = sock.read(&mut sink).await;
                    std::future::pending::<()>().await;
                });
            } else {
                let _ = sock.shutdown().await;
            }
        }
    });
    (addr, captured)
}

// ---------------------------------------------------------------------
// Request normalization used by the reviewed capture
// ---------------------------------------------------------------------

const DROPPED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "accept-encoding",
    "accept-language",
    "sec-fetch-mode",
];

fn normalize_request(raw: &str) -> Value {
    let mut parts = raw.split("\r\n\r\n");
    let head = parts.next().unwrap_or("");
    let body = parts.next().unwrap_or("");
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut request_parts = request_line.split(' ');
    let method = request_parts.next().unwrap_or("");
    let path = request_parts.next().unwrap_or("");
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let value = value.trim().to_string();
        if DROPPED_HEADERS.contains(&name.as_str()) || name.starts_with("x-stainless-") {
            continue;
        }
        if name == "user-agent" && !value.starts_with("claude-cli/") {
            continue;
        }
        headers.insert(name, value);
    }
    let body: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap()
    };
    json!({ "method": method, "path": path, "headers": headers, "body": body })
}

// ---------------------------------------------------------------------
// Option mapping (cases.json → typed options)
// ---------------------------------------------------------------------

fn stream_options(options: &Value, signal: AbortSignal) -> StreamOptions {
    StreamOptions {
        api_key: options
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_string),
        max_tokens: options.get("maxTokens").and_then(Value::as_u64),
        temperature: options.get("temperature").and_then(Value::as_f64),
        session_id: options
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
        cache_retention: options
            .get("cacheRetention")
            .map(|v| serde_json::from_value(v.clone()).unwrap()),
        headers: options.get("headers").and_then(Value::as_object).map(|h| {
            h.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect()
        }),
        metadata: options.get("metadata").and_then(Value::as_object).cloned(),
        max_retries: options
            .get("maxRetries")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        signal: Some(signal),
        ..Default::default()
    }
}

fn anthropic_options(options: &Value, signal: AbortSignal) -> AnthropicOptions {
    AnthropicOptions {
        base: stream_options(options, signal),
        thinking_enabled: options.get("thinkingEnabled").and_then(Value::as_bool),
        thinking_budget_tokens: options.get("thinkingBudgetTokens").and_then(Value::as_u64),
        effort: options
            .get("effort")
            .and_then(Value::as_str)
            .map(str::to_string),
        thinking_display: options
            .get("thinkingDisplay")
            .and_then(Value::as_str)
            .map(|v| match v {
                "omitted" => AnthropicThinkingDisplay::Omitted,
                _ => AnthropicThinkingDisplay::Summarized,
            }),
        interleaved_thinking: options.get("interleavedThinking").and_then(Value::as_bool),
        tool_choice: options.get("toolChoice").map(|choice| match choice {
            Value::String(name) => match name.as_str() {
                "any" => AnthropicToolChoice::Any,
                "none" => AnthropicToolChoice::None,
                _ => AnthropicToolChoice::Auto,
            },
            other => AnthropicToolChoice::Tool {
                name: other["name"].as_str().unwrap().to_string(),
            },
        }),
    }
}

fn simple_options(options: &Value, signal: AbortSignal) -> SimpleStreamOptions {
    SimpleStreamOptions {
        base: stream_options(options, signal),
        reasoning: options
            .get("reasoning")
            .map(|v| serde_json::from_value(v.clone()).unwrap()),
        thinking_budgets: options
            .get("thinkingBudgets")
            .map(|v| serde_json::from_value(v.clone()).unwrap()),
    }
}

// ---------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------

/// Event JSON minus the `partial`/`message`/`error` snapshots recorded by the
/// reviewed capture.
fn summarize(event: &pi_rs_ai_types::AssistantMessageEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap();
    if let Some(map) = value.as_object_mut() {
        map.remove("partial");
        map.remove("message");
        map.remove("error");
    }
    value
}

/// JS number semantics for comparison: `JSON.stringify` prints
/// integral doubles without a fraction, so a computed `0.0` on the pi-rs
/// side equals the oracle's `0`. Applied to both sides.
fn canonicalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if number.as_u64().is_none()
                && number.as_i64().is_none()
                && let Some(float) = number.as_f64()
                && float.is_finite()
                && float.fract() == 0.0
                && float.abs() < 9.007_199_254_740_992e15
            {
                *value = json!(float as i64);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(canonicalize_numbers),
        Value::Object(map) => map.values_mut().for_each(canonicalize_numbers),
        _ => {}
    }
}

fn parity_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/anthropic-parity")
}

async fn run_case(case: &Value, shared_sse: &Value, models: &Value) -> Value {
    let responses: Vec<(String, bool)> = case["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|response| scripted_response(response, shared_sse))
        .collect();
    let (addr, captured) = serve(responses);

    let mut model_json = models[case["model"].as_str().unwrap()].clone();
    model_json["baseUrl"] = Value::String(format!("http://{addr}"));
    let model: Model = serde_json::from_value(model_json).unwrap();
    let context: Context = serde_json::from_value(case["context"].clone()).unwrap();
    let options = &case["options"];
    let signal = AbortSignal::new();
    let abort_after = case.get("abortAfterEvents").and_then(Value::as_u64);

    let stream = if case.get("simple").and_then(Value::as_bool).unwrap_or(false) {
        match stream_simple_anthropic(
            &model,
            &context,
            Some(simple_options(options, signal.clone())),
        ) {
            Ok(stream) => stream,
            Err(error) => {
                return json!({
                    "name": case["name"],
                    "requests": [],
                    "syncError": error.to_string(),
                });
            }
        }
    } else {
        stream_anthropic(
            &model,
            &context,
            Some(anthropic_options(options, signal.clone())),
        )
    };

    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(summarize(&event));
        if abort_after == Some(events.len() as u64) {
            signal.abort();
        }
    }
    let mut result = serde_json::to_value(stream.result().await.unwrap()).unwrap();
    result["timestamp"] = json!(0);

    let requests: Vec<Value> = captured
        .lock()
        .unwrap()
        .iter()
        .map(|raw| normalize_request(raw))
        .collect();
    json!({
        "name": case["name"],
        "requests": requests,
        "events": events,
        "result": result,
    })
}

#[tokio::test]
async fn pi_rs_matches_the_pi_anthropic_oracle() {
    let dir = parity_dir();
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let oracle: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("oracle.json")).unwrap()).unwrap();
    let oracle_cases = oracle["cases"].as_array().unwrap();
    let case_list = cases["cases"].as_array().unwrap();
    assert_eq!(
        case_list.len(),
        oracle_cases.len(),
        "cases.json and its reviewed expected capture disagree"
    );

    let mut failures = Vec::new();
    for (case, expected) in case_list.iter().zip(oracle_cases) {
        let name = case["name"].as_str().unwrap();
        assert_eq!(expected["name"], case["name"], "oracle order mismatch");
        let mut expected = expected.clone();
        canonicalize_numbers(&mut expected);
        let mut actual = run_case(case, &cases["sse"], &cases["models"]).await;
        canonicalize_numbers(&mut actual);
        if actual != expected {
            failures.push(format!(
                "case {name}:\n  expected: {}\n  actual:   {}",
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual).unwrap()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} case(s) diverge from the pi oracle:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
