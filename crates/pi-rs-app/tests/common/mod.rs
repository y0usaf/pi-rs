//! Shared interactive product-test harness (PLAN A.2): the helpers that
//! are byte-identical across the `interactive_*.rs` tests and
//! `resume_replay.rs`. Mirrors the existing
//! `crates/pi-rs-session/tests/common/mod.rs` pattern.
//!
//! Each test file is its own binary and includes this via `mod common;`,
//! so the `ENV_LOCK` static is per-binary — exactly what the tests need
//! (they serialize the process-global `PI_CODING_AGENT_DIR`).
//!
//! Shared here: the scripted loopback HTTP/SSE stub server
//! (`spawn_stub`/`StubResponse`), the one-turn SSE body builder
//! `text_sse`, the Lua `{}`/`[]` encoding-artifact normalizer
//! `normalize_empty_object`, and the product-host/fixture helpers below.
//!
//! Per-test machinery — the `run_sequence` request envelopes (different
//! commands, `sessionFile` vs `sessionDir`, `nowMs`), scenario-specific
//! SSE bodies (`hang_sse`, `SUMMARY_SSE`, `DONE_SSE`, `success_sse`),
//! and the session writers — stays local to each test.
#![allow(dead_code, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use pi_rs_app::builtins::{CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

/// Process-global serializer for `PI_CODING_AGENT_DIR` (read at `Host::new`).
pub static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A host with the product packs embedded, rooted at `cwd`.
pub fn host(cwd: &str) -> Host {
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        TOOLS_PACK,
        CODING_AGENT_PACK,
        INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

/// Read one HTTP request body from the scripted stub's client stream.
pub fn read_request(stream: &mut TcpStream) -> serde_json::Value {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                let body = &bytes[end + 4..end + 4 + length];
                return serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
            }
        }
    }
    serde_json::Value::Null
}

/// One scripted HTTP response for `spawn_stub`.
pub enum StubResponse {
    /// 200 text/event-stream with Content-Length and Connection: close.
    Sse(String),
    /// 200 text/event-stream with no Content-Length: the body runs until
    /// close, which never comes — the client must abort.
    Hang(String),
    /// Non-stream response: status line plus JSON body.
    Json(u16, String),
}

/// A scripted loopback HTTP server. Each incoming connection is served
/// the next scripted response (falling back to the last, so a
/// single-response stub serves every connection), and every request body
/// is recorded. Returns the base URL and the recorded requests.
pub fn spawn_stub(responses: Vec<StubResponse>) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for (index, conn) in listener.incoming().enumerate() {
            let Ok(mut stream) = conn else { break };
            let request = read_request(&mut stream);
            seen.lock().unwrap().push(request);
            match responses.get(index).or_else(|| responses.last()) {
                Some(StubResponse::Sse(body)) => {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Some(StubResponse::Hang(body)) => {
                    let response =
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n{body}");
                    let _ = stream.write_all(response.as_bytes());
                    thread::spawn(move || {
                        let mut sink = [0u8; 64];
                        let _ = stream.read(&mut sink);
                    });
                }
                Some(StubResponse::Json(code, body)) => {
                    let response = format!(
                        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                None => break,
            }
        }
    });
    (format!("http://{address}"), requests)
}

/// One scripted assistant text turn as an SSE body (message id
/// `msg_01`, model `claude-parity-1`, scripted usage, stop reason
/// `end_turn`).
pub fn text_sse(text: &str, input_tokens: u64) -> String {
    format!(
        concat!(
            "event: message_start\n",
            "data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-parity-1\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":{input},\"output_tokens\":1}}}}}}\n\n",
            "event: content_block_start\n",
            "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
            "event: content_block_delta\n",
            "data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{text}}}}}\n\n",
            "event: content_block_stop\n",
            "data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
            "event: message_delta\n",
            "data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":4}}}}\n\n",
            "event: message_stop\n",
            "data: {{\"type\":\"message_stop\"}}\n\n"
        ),
        input = input_tokens,
        text = serde_json::Value::String(text.to_owned()),
    )
}

/// Bracketed-paste wrapper (`\x1b[200~…\x1b[201~`).
pub fn paste(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~")
}

/// The baseline scripted model shared by the parity sequences.
pub fn stub_model(base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "claude-parity-1", "name": "Claude Parity",
        "api": "anthropic-messages", "provider": "anthropic",
        "baseUrl": base_url, "reasoning": false,
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 200000, "maxTokens": 1024
    })
}

/// A tempdir fixture: cwd, `agent_dir`, `sessions` dir, with
/// `PI_CODING_AGENT_DIR` set (call under `ENV_LOCK`).
pub struct Fixture {
    _temp: tempfile::TempDir,
    pub cwd: String,
    pub agent_dir: std::path::PathBuf,
    pub sessions: std::path::PathBuf,
}

pub fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    Fixture {
        _temp: temp,
        cwd,
        agent_dir,
        sessions,
    }
}

/// Recursively fold Lua's `{}`/`[]` encoding artifact: any empty
/// object (a Lua table has one empty value for both encodings) compares
/// as an empty array.
pub fn normalize_empty_object(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) if map.is_empty() => {
            *value = serde_json::Value::Array(Vec::new());
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                normalize_empty_object(item);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_empty_object(item);
            }
        }
        _ => {}
    }
}

/// Read and parse the single session JSONL file in `fixture.sessions`.
pub fn session_entries(fixture: &Fixture) -> Vec<serde_json::Value> {
    let mut files: Vec<_> = std::fs::read_dir(&fixture.sessions)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 1, "expected one session file: {files:?}");
    std::fs::read_to_string(&files[0])
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Read and parse a JSONL file at `path`.
pub fn jsonl_entries(path: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Text of every message in a provider request (`content[0].text`).
pub fn message_texts(request: &serde_json::Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            message["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// Text of the user messages in a provider request.
pub fn user_texts(request: &serde_json::Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .map(|message| {
            message["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}
