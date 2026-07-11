#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 6.3 behavior pins for the session UI: `/resume` switches the
//! live runtime to the selected session (the next provider request
//! carries the new session's context, and the new turn persists into the
//! new file only), and `/new` replaces it with a fresh session. The
//! selector/dialog frames themselves are pinned by
//! tests/ui-parity/session-turn.json.
//!
//! This file is its own test binary: it owns the process-global
//! `PI_CODING_AGENT_DIR`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use pi_rs_app::builtins::{CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

fn read_request(stream: &mut TcpStream) -> serde_json::Value {
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

/// One scripted assistant text turn ("done") as an SSE body.
const DONE_SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-parity-1\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n"
);

fn spawn_stub() -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            let request = read_request(&mut stream);
            seen.lock().unwrap().push(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                DONE_SSE.len(),
                DONE_SSE
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}"), requests)
}

fn stub_model(base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "claude-parity-1", "name": "Claude Parity",
        "api": "anthropic-messages", "provider": "anthropic",
        "baseUrl": base_url, "reasoning": false,
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 200000, "maxTokens": 1024
    })
}

/// A single-turn session fixture (`user_text` → `assistant_text`).
fn write_session_fixture(
    dir: &std::path::Path,
    name: &str,
    id_suffix: &str,
    cwd: &str,
    modified_ms: i64,
    user_text: &str,
    assistant_text: &str,
) -> String {
    let entries = [
        serde_json::json!({"type": "session", "version": 3,
            "id": format!("0198a5c0-0000-7000-8000-0000000000{id_suffix}"),
            "timestamp": "2026-07-01T10:00:00.000Z", "cwd": cwd}),
        serde_json::json!({"type": "model_change", "id": "e1", "parentId": null,
            "timestamp": "2026-07-01T10:00:00.000Z",
            "provider": "anthropic", "modelId": "claude-parity-1"}),
        serde_json::json!({"type": "thinking_level_change", "id": "e2", "parentId": "e1",
            "timestamp": "2026-07-01T10:00:00.000Z", "thinkingLevel": "off"}),
        serde_json::json!({"type": "message", "id": "e3", "parentId": "e2",
            "timestamp": "2026-07-01T10:00:01.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": user_text}],
                "timestamp": modified_ms}}),
        serde_json::json!({"type": "message", "id": "e4", "parentId": "e3",
            "timestamp": "2026-07-01T10:00:02.000Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": assistant_text}],
                "api": "anthropic-messages", "provider": "anthropic", "model": "claude-parity-1",
                "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": modified_ms + 1000}}),
    ];
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    let content: String = entries.iter().map(|entry| format!("{entry}\n")).collect();
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

fn host(cwd: &str) -> Host {
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

/// `PI_CODING_AGENT_DIR` is process-global and read at `Host::new`;
/// each test sets its own agent dir, so they must not overlap.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn paste(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~")
}

fn user_texts(request: &serde_json::Value) -> Vec<String> {
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

#[test]
fn resume_switches_the_live_runtime_and_new_starts_fresh() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    // s2 is the most recently modified — the selector's first row.
    let s1 = write_session_fixture(
        &sessions,
        "s1.jsonl",
        "01",
        &cwd,
        1_751_360_000_000,
        "hi",
        "hello there",
    );
    let s2 = write_session_fixture(
        &sessions,
        "s2.jsonl",
        "02",
        &cwd,
        1_751_363_000_000,
        "alpha",
        "beta",
    );
    let s1_before = std::fs::read_to_string(&s1).unwrap();

    let (base_url, requests) = spawn_stub();
    let result = host(&cwd)
        .call_command(
            "interactive-session-parity-sequence",
            &serde_json::json!({
                "columns": 80, "rows": 30,
                "model": stub_model(&base_url), "apiKey": "test-key",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": s1, "sessionDir": sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "nowMs": 1_751_364_000_000i64,
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
                "steps": [
                    // /resume → selector (s2 is row 0) → Enter resumes s2.
                    { "name": "open", "input": [paste("/resume"), "\r"] },
                    { "name": "switched", "input": ["\r"] },
                    // The next prompt runs against the resumed session.
                    { "name": "turn", "input": [paste("again"), "\r"] },
                    // /new replaces the runtime with a fresh session…
                    { "name": "fresh", "input": [paste("/new"), "\r"] },
                    // …and its first prompt carries no prior context.
                    { "name": "fresh-turn", "input": [paste("start over"), "\r"] },
                ],
            })
            .to_string(),
        )
        .expect("command")
        .expect("result");

    // The runtime switched to s2 before the turn, then to a fresh file.
    let final_file = result["sessionFile"].as_str().unwrap().to_owned();
    assert_ne!(final_file, s1);
    assert_ne!(final_file, s2);

    // Request 1 (after /resume): s2's context exactly once plus the prompt.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(user_texts(&requests[0]), ["alpha", "again"]);
    // Request 2 (after /new): only the fresh prompt.
    assert_eq!(user_texts(&requests[1]), ["start over"]);

    // The resumed turn persisted into s2 only; s1 is untouched.
    assert_eq!(std::fs::read_to_string(&s1).unwrap(), s1_before);
    let s2_content = std::fs::read_to_string(&s2).unwrap();
    assert!(s2_content.contains("\"again\""), "{s2_content}");
    assert!(s2_content.contains("\"done\""), "{s2_content}");

    // The /new turn persisted into the fresh session file.
    let fresh_content = std::fs::read_to_string(&final_file).unwrap();
    assert!(fresh_content.contains("\"start over\""), "{fresh_content}");
    assert!(!fresh_content.contains("\"again\""), "{fresh_content}");
}

#[test]
fn jsonl_export_import_and_copy_run_through_product_commands() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    let s1 = write_session_fixture(
        &sessions,
        "s1.jsonl",
        "11",
        &cwd,
        1_751_360_000_000,
        "one",
        "first reply",
    );
    let s2 = write_session_fixture(
        &sessions,
        "s2.jsonl",
        "12",
        &cwd,
        1_751_363_000_000,
        "two",
        "second reply",
    );
    let output = temp.path().join("exports/branch.jsonl");

    let result = host(&cwd)
        .call_command(
            "interactive-session-parity-sequence",
            &serde_json::json!({
                "columns": 80, "rows": 30, "model": stub_model("http://127.0.0.1:1"),
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": s1, "sessionDir": sessions.to_string_lossy(),
                "modelFromCli": true, "nowMs": 1_752_237_296_789i64,
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
                "steps": [
                    { "name": "export", "input": [paste(&format!("/export \"{}\"", output.display())), "\r"] },
                    { "name": "import", "input": [paste(&format!("/import \"{s2}\"")), "\r"] },
                    { "name": "confirm", "input": ["\r"] },
                    { "name": "copy", "input": [paste("/copy"), "\r"] },
                ],
            })
            .to_string(),
        )
        .expect("command")
        .expect("result");

    assert_eq!(result["sessionFile"], s2);
    assert_eq!(result["copiedText"], "second reply");
    let rows: Vec<serde_json::Value> = std::fs::read_to_string(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["timestamp"], "2025-07-11T12:34:56.789Z");
    assert_eq!(rows[1]["parentId"], serde_json::Value::Null);
    for pair in rows[1..].windows(2) {
        assert_eq!(pair[1]["parentId"], pair[0]["id"]);
    }
}
