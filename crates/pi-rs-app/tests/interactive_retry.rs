#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 7.10 product-path pins for provider auto-retry: retryable provider
//! errors are removed from live context, retried with configured backoff, and
//! Escape cancels the pending backoff through the interactive handler.

mod common;

fn success_sse(text: &str) -> String {
    concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":TEXT}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    )
    .replace("TEXT", &serde_json::Value::String(text.to_owned()).to_string())
}

fn run(
    base_url: &str,
    cwd: &str,
    agent_dir: &str,
    session_dir: &str,
    steps: serde_json::Value,
) -> serde_json::Value {
    common::host(cwd)
        .call_command(
        "interactive-tree-parity-sequence",
        &serde_json::json!({
            "columns": 72, "rows": 24, "cwd": cwd, "agentDir": agent_dir,
            "sessionDir": session_dir, "apiKey": "test", "runtimeApiKey": "test",
            "modelFromCli": true, "thinkingFromCli": true, "thinkingLevel": "off",
            "model": {
                "id": "claude-retry", "name": "Claude Retry", "provider": "anthropic",
                "api": "anthropic-messages", "baseUrl": base_url, "reasoning": false,
                "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                "contextWindow": 200000, "maxTokens": 1024
            },
            "steps": steps
        }).to_string(),
    ).unwrap().unwrap()
}

#[test]
fn retryable_error_retries_then_recovers() {
    let _guard = common::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ retry = { baseDelayMs = 1, maxRetries = 2 } })\n",
    )
    .unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Json(400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"429 overloaded"}}"#.to_owned()), common::StubResponse::Sse(success_sse("recovered"))]);
    run(
        &base_url,
        temp.path().to_str().unwrap(),
        agent_dir.to_str().unwrap(),
        sessions.to_str().unwrap(),
        serde_json::json!([
            {"input": ["\u{001b}[200~go\u{001b}[201~", "\r"], "captures": [{"event": "auto_retry_start", "name": "retrying"}]},
            {"name": "done"}
        ]),
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let second = &requests[1];
    assert_eq!(
        second["messages"].as_array().unwrap().len(),
        1,
        "error removed from retry context: {second}"
    );
}

#[test]
fn escape_cancels_retry_backoff_without_another_request() {
    let _guard = common::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("config.lua"),
        "local pi = ...\npi.config.settings({ retry = { baseDelayMs = 2000, maxRetries = 2 } })\n",
    )
    .unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Json(400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"429 overloaded"}}"#.to_owned())]);
    let result = run(
        &base_url,
        temp.path().to_str().unwrap(),
        agent_dir.to_str().unwrap(),
        sessions.to_str().unwrap(),
        serde_json::json!([
            {"input": ["\u{001b}[200~go\u{001b}[201~", "\r"], "captures": [{"event": "auto_retry_start", "name": "retrying", "action": "escape"}]},
            {"name": "cancelled"}
        ]),
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(
        result["frames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|frame| frame["name"] == "retrying")
    );
}
