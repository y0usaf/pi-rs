#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 7.1 behavior pins for `!`/`!!` bash mode: an idle `!` command
//! persists a bashExecution entry and reaches the next provider request
//! as its `Ran \`…\`` text form; `!!` persists with excludeFromContext
//! and never reaches the provider; a command submitted mid-turn defers —
//! the message flushes into agent state and the session only after the
//! turn settles, preserving order. Frames are pinned by
//! tests/ui-parity/bash-turn.json.
//!
//! This file is its own test binary: it owns the process-global
//! `PI_CODING_AGENT_DIR`.

mod common;

/// The bash-turn fixture's hanging stream: partial text, then the socket
/// stays open until the client aborts.
fn hang_sse() -> String {
    concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-parity-1\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Once upon a \"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"time\"}}\n\n",
    )
    .to_owned()
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
                "model": common::stub_model(base_url), "apiKey": "test-key",
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

#[test]
fn idle_bash_persists_and_reaches_the_next_request_as_text() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("done", 10))]);
    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "bash", "input": [common::paste("!printf hi"), "\r"], "waitBash": true },
            { "name": "next", "input": [common::paste("next question"), "\r"] },
        ]),
    );

    // The bashExecution entry persisted with the executor's result shape.
    let entries = common::session_entries(&fixture);
    let bash = entries
        .iter()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "bashExecution")
        .expect("bashExecution entry");
    assert_eq!(bash["message"]["command"], "printf hi");
    assert_eq!(bash["message"]["output"], "hi");
    assert_eq!(bash["message"]["exitCode"], 0);
    assert_eq!(bash["message"]["cancelled"], false);
    assert_eq!(bash["message"]["truncated"], false);
    assert!(bash["message"].get("excludeFromContext").is_none());

    // The next provider request carries messages.ts bashExecutionToText
    // ahead of the new user message.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        common::message_texts(&requests[0]),
        vec![
            "Ran `printf hi`\n```\nhi\n```".to_owned(),
            "next question".to_owned(),
        ],
    );
}

#[test]
fn excluded_bash_persists_but_never_reaches_the_provider() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("done", 10))]);
    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "bash", "input": [common::paste("!!printf hi"), "\r"], "waitBash": true },
            { "name": "next", "input": [common::paste("next question"), "\r"] },
        ]),
    );

    let entries = common::session_entries(&fixture);
    let bash = entries
        .iter()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "bashExecution")
        .expect("bashExecution entry");
    assert_eq!(bash["message"]["excludeFromContext"], true);

    // Excluded from context: only the user message reaches the provider.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        common::message_texts(&requests[0]),
        vec!["next question".to_owned()]
    );
}

#[test]
fn deferred_bash_flushes_after_the_turn_settles() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = common::spawn_stub(vec![
        common::StubResponse::Hang(hang_sse()),
        common::StubResponse::Sse(common::text_sse("you're welcome", 10)),
    ]);

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            {
                "input": [common::paste("Tell me a story"), "\r"],
                "waitIdle": false,
                "captures": [{ "name": "streaming", "event": "message_update", "count": 3 }],
            },
            { "name": "deferred", "input": [common::paste("!printf deferred"), "\r"],
              "waitBash": true, "waitIdle": false },
            { "name": "aborted", "input": ["\u{1b}"] },
            { "name": "thanks", "input": [common::paste("thanks"), "\r"] },
        ]),
    );

    // JSONL order: the aborted assistant settles before the deferred
    // bashExecution flushes (agent-session.ts _runAgentPrompt finally).
    let entries = common::session_entries(&fixture);
    let roles: Vec<String> = entries
        .iter()
        .filter(|entry| entry["type"] == "message")
        .map(|entry| entry["message"]["role"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "bashExecution", "user", "assistant"],
        "aborted turn settles, then the deferred bash flushes"
    );
    let aborted = entries
        .iter()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "assistant")
        .unwrap();
    assert_eq!(aborted["message"]["stopReason"], "aborted");

    // The next request carries the flushed bash text exactly once,
    // after the aborted turn and before the new prompt. The aborted
    // assistant itself is skipped by the provider conversion (pi's
    // transformMessages drops errored/aborted assistant messages).
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let texts = common::message_texts(&requests[1]);
    assert_eq!(
        texts,
        vec![
            "Tell me a story".to_owned(),
            "Ran `printf deferred`\n```\ndeferred\n```".to_owned(),
            "thanks".to_owned(),
        ],
    );
}
