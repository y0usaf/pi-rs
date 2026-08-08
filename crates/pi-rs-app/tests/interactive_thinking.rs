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
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("ok", 10))]);
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
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("ok", 10))]);
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
