//! Shared scripted loopback HTTP/SSE server plus request normalization
//! used by the differential (parity) and transport tests in this crate.
//!
//! Each protocol's distinct contract — the shape of its request, event,
//! and final message vs its own `oracle.json` — lives in the calling test
//! and the oracle, NOT in the server. Only the sandbox-safe raw-TCP server
//! machinery and the common header-dropping helpers are shared here.
//!
//! Mirrors the `crates/pi-rs-session/tests/common/mod.rs` pattern.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Container for the raw request strings captured by the loopback server.
pub type Captured = Arc<Mutex<Vec<String>>>;

/// Read one HTTP request: headers plus a content-length body.
pub async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut all = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
        if let Some(pos) = all.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&all[..pos]).to_lowercase();
            let len = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if all.len() >= pos + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&all).into_owned()
}

/// Serve scripted bytes one connection per request (the last response is
/// repeated when a case sends more requests than responses), capturing raw
/// requests. Hanging responses park the connection so it stays open — the
/// client must abort.
fn serve_impl(responses: Vec<(Vec<u8>, bool)>) -> (std::net::SocketAddr, Captured) {
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
            let _ = sock.write_all(&response).await;
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

/// Serve canned textual responses (no hanging).
pub fn serve(responses: Vec<String>) -> (std::net::SocketAddr, Captured) {
    serve_impl(
        responses
            .into_iter()
            .map(|r| (r.into_bytes(), false))
            .collect(),
    )
}

/// Serve canned byte responses (no hanging) — used by the binary
/// eventstream (Bedrock) tests.
pub fn serve_bytes(responses: Vec<Vec<u8>>) -> (std::net::SocketAddr, Captured) {
    serve_impl(responses.into_iter().map(|r| (r, false)).collect())
}

/// Serve responses where some are marked to hang (parked open).
pub fn serve_hang(responses: Vec<(String, bool)>) -> (std::net::SocketAddr, Captured) {
    serve_impl(
        responses
            .into_iter()
            .map(|(r, h)| (r.into_bytes(), h))
            .collect(),
    )
}

// ---------------------------------------------------------------------
// Request normalization (mirrors the per-protocol gen-oracle.ts)
// ---------------------------------------------------------------------

/// Normalize a captured request, dropping the Claude-family headers, any
/// `x-stainless-*` header, and a `user-agent` that is not `claude-cli`.
/// Used by anthropic, azure-openai-responses, openai-completions and
/// openai-responses.
pub fn normalize_claude(raw: &str) -> Value {
    const DROP: &[&str] = &[
        "host",
        "content-length",
        "connection",
        "accept-encoding",
        "accept-language",
        "sec-fetch-mode",
    ];
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("");
    let path = first.next().unwrap_or("");
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let value = value.trim();
        if DROP.contains(&name.as_str())
            || name.starts_with("x-stainless-")
            || (name == "user-agent" && !value.starts_with("claude-cli/"))
        {
            continue;
        }
        headers.insert(name, value.to_string());
    }
    json!({
        "method": method,
        "path": path,
        "headers": headers,
        "body": if body.is_empty() { Value::Null } else { serde_json::from_str(body).unwrap() }
    })
}

/// Normalize a captured request by dropping exactly the named headers.
/// Each caller passes its own protocol-specific `drop` list (used by
/// bedrock-converse-stream, google-generative-ai, mistral,
/// openai-codex-responses, and google-vertex's header pass).
pub fn normalize_drop(raw: &str, drop: &[&str]) -> Value {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("");
    let path = first.next().unwrap_or("");
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        if !drop.contains(&name.as_str()) {
            headers.insert(name, value.trim().to_string());
        }
    }
    json!({
        "method": method,
        "path": path,
        "headers": headers,
        "body": if body.is_empty() { Value::Null } else { serde_json::from_str(body).unwrap() }
    })
}
