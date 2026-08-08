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

mod common;

use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

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
            let request = common::read_request(&mut stream);
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
    let mut model = common::stub_model(base_url);
    model["reasoning"] = serde_json::json!(true);
    model["thinkingLevelMap"] = serde_json::json!({ "minimal": null, "xhigh": "max" });
    model["maxTokens"] = serde_json::json!(16384);
    model
}

fn run_sequence(
    fixture: &common::Fixture,
    base_url: &str,
    steps: serde_json::Value,
) -> serde_json::Value {
    common::host(&fixture.cwd)
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
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = spawn_stub();

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([{ "name": "turn", "input": ["hello", "\r"] }]),
    );

    let entries = common::session_entries(&fixture);
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
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
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
    let entries = common::session_entries(&fixture);
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
