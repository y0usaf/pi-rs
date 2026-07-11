#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Headless WS4.6 acceptance: the real Anthropic protocol is replayed through
//! the public Lua agent, including a tool round trip and JSONL persistence.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use pi_rs_app::builtins::{CODING_AGENT_PACK, TOOLS_PACK};
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

fn response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
}

#[test]
fn anthropic_replay_runs_lua_agent_tool_round_trip_and_persists_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&requests);
    let first = include_str!("../../pi-rs-ai/tests/fixtures/anthropic/replay_basic.sse").to_owned();
    let second = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-4-7\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
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
    let server = thread::spawn(move || {
        for body in [&first, second] {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            seen.lock().unwrap().push(request);
            stream.write_all(response(body).as_bytes()).unwrap();
        }
    });

    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let agent_dir = temp.path().join("agent");
    // Deterministic auth: the VM's `pi.auth` storage (pi-rs-run's per-request
    // getApiKey seam) must not see ambient developer credentials.
    // SAFETY: single-threaded at this point; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let model = serde_json::json!({
        "id": "claude-opus-4-7", "name": "Claude Opus 4.7",
        "api": "anthropic-messages", "provider": "anthropic",
        "baseUrl": format!("http://{}", address), "reasoning": true,
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 100000, "maxTokens": 1024
    });
    let host = Host::new(HostConfig {
        cwd: Some(cwd.clone()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let agent_dir_string = agent_dir.to_string_lossy().into_owned();
    let result = host
        .call_command(
            "pi-rs-run",
            &serde_json::json!({
                "model": model, "apiKey": "test-key", "prompt": "read the file", "cwd": cwd,
                "agentDir": agent_dir_string,
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    server.join().unwrap();

    assert_eq!(result["text"], "done");
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);

    // PLAN 5.1: the wired system prompt and default active tool set reach
    // the provider request. The prompt content itself is pinned to Pi by
    // tests/system-prompt-parity; here the same Lua ports produce the
    // expected value for this cwd/agent dir.
    let expected = host
        .call_command(
            "system-prompt-parity",
            &serde_json::json!({
                "mode": "session",
                "cwd": cwd,
                "agentDir": agent_dir_string,
                "toolNames": ["read", "bash", "edit", "write"],
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    for request in captured.iter() {
        assert_eq!(request["system"][0]["text"], expected["prompt"]);
        let tool_names: Vec<&str> = request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(tool_names, ["read", "bash", "edit", "write"]);
    }
    let session_text = std::fs::read_to_string(result["sessionPath"].as_str().unwrap()).unwrap();
    let entries: Vec<serde_json::Value> = session_text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let messages: Vec<&serde_json::Value> = entries
        .iter()
        .filter_map(|entry| entry.get("message"))
        .collect();
    assert!(messages.iter().any(|message| message["role"] == "user"));
    assert!(
        messages
            .iter()
            .any(|message| message["role"] == "toolResult")
    );
    assert!(
        messages
            .iter()
            .any(|message| message["role"] == "assistant")
    );
}
