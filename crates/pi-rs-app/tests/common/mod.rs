//! Shared interactive product-test harness (PLAN A.2): the helpers that
//! are byte-identical across the `interactive_*.rs` tests and
//! `resume_replay.rs`. Mirrors the existing
//! `crates/pi-rs-session/tests/common/mod.rs` pattern.
//!
//! Each test file is its own binary and includes this via `mod common;`,
//! so the `ENV_LOCK` static is per-binary — exactly what the tests need
//! (they serialize the process-global `PI_CODING_AGENT_DIR`).
//!
//! Per-test machinery — the `run_sequence` request envelopes (different
//! commands, `sessionFile` vs `sessionDir`, `nowMs`), the scripted SSE
//! stubs (`Scripted`/`Response` enums, `text_sse`/`success_sse`/`hang_sse`,
//! `spawn_stub`), and the session writers — stays local to each test.
#![allow(dead_code, clippy::unwrap_used)]

use std::io::Read;
use std::net::TcpStream;
use std::sync::Mutex;

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
/// `PI_CODING_AGENT_DIR` set (call under [`ENV_LOCK`]).
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
