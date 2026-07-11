#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 6.2 acceptance: `--continue`/`--session` selections reopen a
//! session through the product packs — the next provider request carries
//! the rebuilt context exactly once, the JSONL gains only the new
//! entries, and the sdk.ts model/thinking restore precedence holds.
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
        "baseUrl": base_url, "reasoning": true,
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 200000, "maxTokens": 1024
    })
}

/// A prior single-turn session, optionally with a thinking entry and a
/// custom saved model reference.
fn write_session_fixture(
    dir: &std::path::Path,
    cwd: &str,
    saved_model: (&str, &str),
    thinking: Option<&str>,
) -> String {
    let mut entries = vec![
        serde_json::json!({"type": "session", "version": 3,
            "id": "0198a5c0-0000-7000-8000-000000000001",
            "timestamp": "2026-07-01T10:00:00.000Z", "cwd": cwd}),
        serde_json::json!({"type": "model_change", "id": "e1", "parentId": null,
            "timestamp": "2026-07-01T10:00:00.000Z",
            "provider": saved_model.0, "modelId": saved_model.1}),
    ];
    let mut parent = "e1";
    if let Some(level) = thinking {
        entries.push(
            serde_json::json!({"type": "thinking_level_change", "id": "e2",
            "parentId": parent, "timestamp": "2026-07-01T10:00:00.000Z",
            "thinkingLevel": level}),
        );
        parent = "e2";
    }
    entries.push(
        serde_json::json!({"type": "message", "id": "e3", "parentId": parent,
        "timestamp": "2026-07-01T10:00:01.000Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "hi"}], "timestamp": 0}}),
    );
    // The assistant message carries the saved model too — the branch's
    // last assistant message wins in buildSessionContext.
    entries.push(
        serde_json::json!({"type": "message", "id": "e4", "parentId": "e3",
        "timestamp": "2026-07-01T10:00:02.000Z",
        "message": {"role": "assistant", "content": [{"type": "text", "text": "hello there"}],
            "api": "anthropic-messages", "provider": saved_model.0, "model": saved_model.1,
            "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 0}}),
    );
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("2026-07-01T10-00-00-000Z_resume.jsonl");
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

fn entry_types(path: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn resume_replays_context_exactly_once_and_appends_only_new_entries() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let session_file = write_session_fixture(
        &temp.path().join("sessions"),
        &cwd,
        ("anthropic", "claude-parity-1"),
        Some("low"),
    );

    let (base_url, requests) = spawn_stub();
    let result = host(&cwd)
        .call_command(
            "pi-rs-run",
            &serde_json::json!({
                "model": stub_model(&base_url), "apiKey": "test-key", "prompt": "again",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": session_file,
                "modelFromCli": true, "thinkingFromCli": false,
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();

    assert_eq!(result["text"], "done");
    // sdk.ts thinking restore: the saved level applies when --thinking was
    // not given.
    assert_eq!(result["thinkingLevel"], "low");

    // The next provider request carries the rebuilt context exactly once:
    // restored user + assistant, then the new prompt.
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1, "{captured:?}");
    let messages = captured[0]["messages"].as_array().unwrap();
    let roles_and_text: Vec<(String, String)> = messages
        .iter()
        .map(|message| {
            let text = message["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            (message["role"].as_str().unwrap().to_owned(), text)
        })
        .collect();
    assert_eq!(
        roles_and_text,
        vec![
            ("user".to_owned(), "hi".to_owned()),
            ("assistant".to_owned(), "hello there".to_owned()),
            ("user".to_owned(), "again".to_owned()),
        ]
    );

    // The reopened session gains only the new turn — no duplicated
    // restore appends (the fixture already has a thinking entry).
    assert_eq!(
        entry_types(&session_file),
        vec![
            "session",
            "model_change",
            "thinking_level_change",
            "message",
            "message",
            "message", // new user
            "message", // new assistant
        ]
    );
    let last: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(&session_file)
            .unwrap()
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(last["message"]["role"], "assistant");
    assert_eq!(last["message"]["content"][0]["text"], "done");
}

#[test]
fn model_restore_prefers_saved_model_and_falls_back_with_warning() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("auth.json"),
        serde_json::json!({ "anthropic": { "type": "api_key", "key": "sk-a" } }).to_string(),
    )
    .unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();

    // Saved model is in the registry catalog with configured auth: the
    // session's model is restored over the CLI fallback (sdk.ts
    // "if (!model && hasExistingSession && existingSession.model)").
    let session_file = write_session_fixture(
        &temp.path().join("sessions-restore"),
        &cwd,
        ("anthropic", "claude-opus-4-8"),
        Some("off"),
    );
    let result = host(&cwd)
        .call_command(
            "interactive-provider-parity-sequence",
            &serde_json::json!({
                "columns": 72, "rows": 24, "appName": "pi", "version": "0.79.0",
                "branch": "main", "cwd": cwd, "home": "/home/user",
                "model": stub_model("http://127.0.0.1:9"),
                "sessionFile": session_file,
                "agentDir": agent_dir.to_string_lossy(),
                "steps": [],
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(result["model"]["provider"], "anthropic");
    assert_eq!(result["model"]["id"], "claude-opus-4-8");

    // Saved model unknown to the registry: fall back to the CLI-resolved
    // model with the spec's message.
    let session_file = write_session_fixture(
        &temp.path().join("sessions-fallback"),
        &cwd,
        ("ghost", "model-x"),
        None,
    );
    let (base_url, requests) = spawn_stub();
    let result = host(&cwd)
        .call_command(
            "pi-rs-run",
            &serde_json::json!({
                "model": stub_model(&base_url), "apiKey": "sk-a", "prompt": "again",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": session_file,
                "modelFromCli": false, "thinkingFromCli": false,
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        result["modelFallbackMessage"],
        "Could not restore model ghost/model-x. Using anthropic/claude-parity-1"
    );
    assert_eq!(result["model"]["id"], "claude-parity-1");
    assert_eq!(requests.lock().unwrap().len(), 1);
    // A restored session without a thinking entry backfills one (sdk.ts
    // "if (!hasThinkingEntry) sessionManager.appendThinkingLevelChange").
    let types = entry_types(&session_file);
    assert_eq!(types[4], "thinking_level_change");
}
