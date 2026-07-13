#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 7.2 behavior pins for thinking levels: a new session defaults to
//! DEFAULT_THINKING_LEVEL ("medium") clamped to the model; shift+tab
//! (app.thinking.cycle) walks the model's supported levels — an explicit
//! `null` in `thinkingLevelMap` skips that level even after the model
//! table crosses the Lua boundary — persisting a `thinking_level_change`
//! JSONL entry and the settings `defaultThinkingLevel`; and the next
//! provider request carries the level as an anthropic thinking budget.
//! Frames are pinned by tests/ui-parity/thinking-turn.json.
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

/// One scripted assistant text turn as an SSE body.
fn text_sse(text: &str) -> String {
    format!(
        concat!(
            "event: message_start\n",
            "data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-parity-1\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":10,\"output_tokens\":1}}}}}}\n\n",
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
        text = serde_json::Value::String(text.to_owned()),
    )
}

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
            let body = text_sse("ok");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}"), requests)
}

/// A reasoning model whose `thinkingLevelMap` marks `minimal` unsupported
/// (the explicit-null semantics 120 catalog rows use) and maps `xhigh`.
fn stub_model(base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "claude-parity-1", "name": "Claude Parity",
        "api": "anthropic-messages", "provider": "anthropic",
        "baseUrl": base_url, "reasoning": true,
        "thinkingLevelMap": { "minimal": null, "xhigh": "max" },
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 200000, "maxTokens": 16384
    })
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

/// `PI_CODING_AGENT_DIR` is process-global and read at `Host::new`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Fixture {
    _temp: tempfile::TempDir,
    cwd: String,
    agent_dir: std::path::PathBuf,
    sessions: std::path::PathBuf,
}

fn fixture() -> Fixture {
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

fn run_sequence(fixture: &Fixture, base_url: &str, steps: serde_json::Value) -> serde_json::Value {
    host(&fixture.cwd)
        .call_command(
            "interactive-bash-parity-sequence",
            &serde_json::json!({
                "columns": 90, "rows": 30,
                "model": stub_model(base_url), "apiKey": "test-key",
                "runtimeApiKey": "test-key",
                "cwd": fixture.cwd, "agentDir": fixture.agent_dir.to_string_lossy(),
                "sessionDir": fixture.sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "steps": steps,
            })
            .to_string(),
        )
        .expect("command")
        .expect("result")
}

fn session_entries(fixture: &Fixture) -> Vec<serde_json::Value> {
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

fn thinking_entries(entries: &[serde_json::Value]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry["type"] == "thinking_level_change")
        .map(|entry| entry["thinkingLevel"].as_str().unwrap().to_owned())
        .collect()
}

const SHIFT_TAB: &str = "\u{1b}[Z";

/// sdk.ts: a new session without a settings default lands on
/// DEFAULT_THINKING_LEVEL ("medium") clamped to the model, and the first
/// provider request carries the anthropic thinking budget for it.
#[test]
fn new_sessions_default_to_medium_clamped_to_the_model() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture();
    let (base_url, requests) = spawn_stub();

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([{ "name": "turn", "input": ["hello", "\r"] }]),
    );

    let entries = session_entries(&fixture);
    assert_eq!(thinking_entries(&entries), vec!["medium"]);
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1, "{captured:?}");
    assert_eq!(
        captured[0]["thinking"],
        serde_json::json!({ "type": "enabled", "budget_tokens": 8192, "display": "summarized" })
    );
}

/// interactive-mode.ts cycleThinkingLevel via shift+tab: the walk skips
/// the map's explicit-null level (off -> low, not minimal), persists the
/// change to the session JSONL and the settings default, and the next
/// provider request streams with the new level's budget.
#[test]
fn cycling_skips_null_map_levels_and_persists_to_session_settings_and_requests() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture();
    // Config default "off": one cycle pins the null-map skip.
    std::fs::write(
        fixture.agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ defaultThinkingLevel = 'off' })\n",
    )
    .unwrap();
    let (base_url, requests) = spawn_stub();

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "off-turn", "input": ["hello", "\r"] },
            { "name": "cycle", "input": [SHIFT_TAB] },
            { "name": "low-turn", "input": ["again", "\r"] },
        ]),
    );

    // JSONL: the startup entry ("off" from settings), then the cycled
    // level — "low", skipping the null-marked "minimal".
    let entries = session_entries(&fixture);
    assert_eq!(thinking_entries(&entries), vec!["off", "low"]);

    // The interactive mutation persists back into the managed config.lua block.
    let source = std::fs::read_to_string(fixture.agent_dir.join("config.lua")).unwrap();
    let settings = pi_rs_host::config::evaluate(&source, "config.lua").unwrap();
    assert_eq!(settings.settings["defaultThinkingLevel"], "low");

    // Requests: thinking disabled at "off" (the spec's explicit
    // `{type: "disabled"}` for reasoning models); the low budget after.
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2, "{captured:?}");
    assert_eq!(
        captured[0]["thinking"],
        serde_json::json!({ "type": "disabled" })
    );
    assert_eq!(
        captured[1]["thinking"],
        serde_json::json!({ "type": "enabled", "budget_tokens": 2048, "display": "summarized" })
    );
}
