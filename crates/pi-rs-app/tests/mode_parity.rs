//! PLAN 10 mode-parity pins: print (text/json) and RPC roles route
//! through the generic registered-role surface with Pi-identical
//! stdout/stderr framing, exit-status inputs, serialization, and
//! extension error delivery.
//!
//! The roles write their JSONL/event streams through pi.output; the
//! runtime mirrors every written line back in the role result
//! (writtenLines) so in-process tests can pin the byte framing without
//! spawning the binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use pi_rs_host::{Host, HostConfig};

fn host(cwd: &str) -> Host {
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_owned()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::CODING_AGENT_PACK,
        pi_rs_app::builtins::INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

fn request_with_model(
    fixture: &common::Fixture,
    model: serde_json::Value,
    prompt: &str,
    mode: &str,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "apiKey": "test-key",
        "runtimeApiKey": "test-key",
        "prompt": prompt,
        "moreMessages": [],
        "cwd": fixture.cwd,
        "agentDir": fixture.agent_dir.to_string_lossy(),
        "sessionDir": fixture.sessions.to_string_lossy(),
        "home": "/home/test",
        "appName": "pi",
        "version": "0.1.0",
        "thinkingLevel": "off",
        "modelFromCli": true,
        "thinkingFromCli": false,
        "projectTrusted": true,
        "mode": mode,
        "systemPrompt": serde_json::Value::Null,
        "appendSystemPrompt": [],
        "name": serde_json::Value::Null,
        "readmePath": "/pi-rs-pkg/README.md",
        "docsPath": "/pi-rs-pkg/docs",
        "examplesPath": "/pi-rs-pkg/examples",
    })
}

fn request(
    fixture: &common::Fixture,
    base_url: &str,
    prompt: &str,
    mode: &str,
) -> serde_json::Value {
    request_with_model(fixture, common::stub_model(base_url), prompt, mode)
}

/// The stub model with thinking support enabled (clamp/cycle paths).
fn reasoning_model(base_url: &str) -> serde_json::Value {
    let mut model = common::stub_model(base_url);
    model["reasoning"] = serde_json::json!(true);
    model["thinkingLevelMap"] = serde_json::json!({
        "minimal": "minimal", "low": "low", "medium": "medium", "high": "high"
    });
    model
}

/// Run one RPC command against a shared host; returns every JSON line
/// the role has written so far (accumulated across dispatches).
fn rpc(
    h: &Host,
    fixture: &common::Fixture,
    base_url: &str,
    command: serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut req = request(fixture, base_url, "", "rpc");
    req["rpcCommand"] = command;
    let mut result = h.call_role("rpc", &req.to_string()).unwrap().unwrap();
    common::normalize_empty_object(&mut result);
    result["writtenLines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| serde_json::from_str(line.as_str().unwrap()).unwrap())
        .collect()
}

/// The lines written by the most recent dispatch (after `previous` lines).
fn new_lines(all: &[serde_json::Value], previous: usize) -> &[serde_json::Value] {
    assert!(
        all.len() > previous,
        "no new lines: {all:?} after {previous}"
    );
    &all[previous..]
}

#[test]
fn print_text_mode_returns_final_text_parts_and_writes_nothing() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(
        common::text_sse("done", 10),
    )]);
    let req = request(&fixture, &base_url, "hello", "text");
    let mut result = host(&fixture.cwd)
        .call_role("print", &req.to_string())
        .unwrap()
        .unwrap();
    common::normalize_empty_object(&mut result);
    // Spec: text mode prints only the final assistant text parts (each +
    // "\n"); nothing streams and nothing was written by the role.
    assert_eq!(result["text"], "done");
    assert_eq!(result["textParts"], serde_json::json!(["done"]));
    assert_eq!(
        result["writtenLines"].as_array().unwrap().len(),
        0,
        "text mode must not stream deltas or header lines"
    );
    assert!(result["stopReason"].is_string());
    assert!(result["stopReason"] != "error");
    assert!(result["stopReason"] != "aborted");
    assert_eq!(common::user_texts(&requests.lock().unwrap()[0]), ["hello"]);
}

#[test]
fn print_text_mode_error_stop_reason_reports_error_message() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, _requests) = common::spawn_stub(vec![common::StubResponse::Json(
        500,
        r#"{"error":{"message":"provider boom"}}"#.to_owned(),
    )]);
    let req = request(&fixture, &base_url, "hello", "text");
    let mut result = host(&fixture.cwd)
        .call_role("print", &req.to_string())
        .unwrap()
        .unwrap();
    common::normalize_empty_object(&mut result);
    // Spec: a last assistant message with stopReason error/aborted prints
    // its error to stderr and exits 1 (main.rs maps the fields below).
    assert_eq!(result["stopReason"], "error");
    assert!(result["errorMessage"].is_string());
    assert!(
        result["errorMessage"]
            .as_str()
            .unwrap()
            .contains("provider boom")
    );
}

#[test]
fn print_json_mode_writes_header_then_event_jsonl() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, _requests) = common::spawn_stub(vec![common::StubResponse::Sse(
        common::text_sse("json answer", 10),
    )]);
    let req = request(&fixture, &base_url, "hello", "json");
    let mut result = host(&fixture.cwd)
        .call_role("print", &req.to_string())
        .unwrap()
        .unwrap();
    common::normalize_empty_object(&mut result);
    let lines = result["writtenLines"].as_array().unwrap();
    assert!(lines.len() >= 2, "header + events: {lines:?}");
    // First record is the session header (print-mode.ts getHeader).
    let header: serde_json::Value = serde_json::from_str(lines[0].as_str().unwrap()).unwrap();
    assert_eq!(header["type"], "session");
    assert!(header["id"].is_string());
    assert_eq!(header["cwd"], fixture.cwd);
    // Every later record is a strict JSONL session event.
    let events: Vec<serde_json::Value> = lines[1..]
        .iter()
        .map(|line| serde_json::from_str(line.as_str().unwrap()).unwrap())
        .collect();
    for event in &events {
        assert!(event["type"].is_string(), "{event}");
    }
    assert!(
        events.iter().any(|event| event["type"] == "message_end"),
        "message_end event expected: {events:?}"
    );
}

#[test]
fn rpc_get_state_shapes_the_pinned_session_state() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, _requests) =
        common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("x", 10))]);
    let h = host(&fixture.cwd);
    let lines = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "1"}),
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    let response = &lines[0];
    assert_eq!(response["id"], "1");
    assert_eq!(response["type"], "response");
    assert_eq!(response["command"], "get_state");
    assert_eq!(response["success"], true);
    let state = &response["data"];
    assert_eq!(state["model"]["id"], "claude-parity-1");
    assert_eq!(state["thinkingLevel"], "off");
    assert_eq!(state["isStreaming"], false);
    assert_eq!(state["isCompacting"], false);
    assert_eq!(state["steeringMode"], "one-at-a-time");
    assert_eq!(state["followUpMode"], "one-at-a-time");
    assert_eq!(state["autoCompactionEnabled"], true);
    assert_eq!(state["messageCount"], 0);
    assert_eq!(state["pendingMessageCount"], 0);
    assert!(state["sessionFile"].is_string());
    assert!(state["sessionId"].is_string());
}

#[test]
fn rpc_prompt_emits_success_then_streams_events() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(
        common::text_sse("rpc answer", 10),
    )]);
    let h = host(&fixture.cwd);
    // Prime the runtime with get_state so the prompt dispatch output
    // starts cleanly after it.
    let first = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "1"}),
    );
    assert_eq!(first.len(), 1);
    let all = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "prompt", "id": "2", "message": "hello rpc"}),
    );
    let lines = new_lines(&all, first.len());
    // First line of this dispatch: the authoritative success response
    // (preflight passed), then the streamed session events.
    let success = &lines[0];
    assert_eq!(success["id"], "2");
    assert_eq!(success["type"], "response");
    assert_eq!(success["command"], "prompt");
    assert_eq!(success["success"], true);
    assert!(success.get("data").is_none());
    assert!(
        lines.iter().any(|line| line["type"] == "message_end"),
        "streamed events expected: {lines:?}"
    );
    assert_eq!(
        common::user_texts(&requests.lock().unwrap()[0]),
        ["hello rpc"]
    );
    // get_state after the turn sees the persisted message.
    let after = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "3"}),
    );
    let state = new_lines(&after, all.len()).last().unwrap()["data"].clone();
    assert_eq!(state["messageCount"], 2); // user + assistant
    assert_eq!(state["pendingMessageCount"], 0);
}

#[test]
fn rpc_steer_and_follow_up_queue_then_prompt_drains() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, requests) = common::spawn_stub(vec![common::StubResponse::Sse(
        common::text_sse("done", 10),
    )]);
    let h = host(&fixture.cwd);
    let all = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "steer", "id": "1", "message": "steer me"}),
    );
    let steer = new_lines(&all, 0);
    assert_eq!(steer[0]["command"], "steer");
    assert_eq!(steer[0]["success"], true);
    let all2 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "follow_up", "id": "2", "message": "follow me"}),
    );
    let follow = new_lines(&all2, all.len());
    assert_eq!(follow[0]["command"], "follow_up");
    assert_eq!(follow[0]["success"], true);
    let all3 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "3"}),
    );
    let state = new_lines(&all3, all2.len());
    assert_eq!(state[0]["data"]["pendingMessageCount"], 2);
    let all4 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "prompt", "id": "4", "message": "go"}),
    );
    let lines = new_lines(&all4, all3.len());
    assert_eq!(lines[0]["command"], "prompt");
    assert!(lines.iter().any(|line| line["type"] == "message_end"));
    // The prompt runs first, then steering is drained into the same
    // request (agent-loop.ts ordering); the follow_up is drained after
    // the turn settles into a second provider request.
    let guard = requests.lock().unwrap();
    assert!(guard.len() >= 2, "steer + follow_up requests: {guard:?}");
    assert_eq!(common::user_texts(&guard[0]), ["go", "steer me"]);
    // The second request carries the full history plus the follow_up.
    drop(guard);
    let all5 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "5"}),
    );
    let state = new_lines(&all5, all4.len());
    assert_eq!(state[0]["data"]["pendingMessageCount"], 0);
}

#[test]
fn rpc_unknown_command_and_model_errors_match_shapes() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, _requests) =
        common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("x", 10))]);
    let h = host(&fixture.cwd);
    let all = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "bogus", "id": "9"}),
    );
    let lines = new_lines(&all, 0);
    assert_eq!(lines[0]["id"], "9");
    assert_eq!(lines[0]["command"], "bogus");
    assert_eq!(lines[0]["success"], false);
    assert_eq!(lines[0]["error"], "Unknown command: bogus");
    let all2 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "set_model", "id": "10", "provider": "nope", "modelId": "x"}),
    );
    let lines = new_lines(&all2, all.len());
    assert_eq!(lines[0]["command"], "set_model");
    assert_eq!(lines[0]["success"], false);
    assert_eq!(lines[0]["error"], "Model not found: nope/x");
}

#[test]
fn rpc_extension_errors_emit_extension_error_records() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, _requests) =
        common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("x", 10))]);
    let h = host(&fixture.cwd);
    h.load(
        "examples/extensions/rpc-error-probe.lua",
        r#"
            local pi = ...
            pi.on("session_start", function(event, ctx)
                error("probe boom")
            end)
        "#,
    )
    .unwrap();
    let lines = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "1"}),
    );
    let error_line = lines
        .iter()
        .find(|line| line["type"] == "extension_error")
        .expect("extension_error record");
    assert!(error_line["error"].as_str().unwrap().contains("probe boom"));
    // The get_state response still follows (errors are isolated).
    let response = lines.last().unwrap();
    assert_eq!(response["command"], "get_state");
    assert_eq!(response["success"], true);
}

#[test]
fn rpc_set_model_and_thinking_level_persist() {
    let _env = common::ENV_LOCK.lock().unwrap();
    let fixture = common::fixture();
    let (base_url, _requests) =
        common::spawn_stub(vec![common::StubResponse::Sse(common::text_sse("x", 10))]);
    let h = host(&fixture.cwd);
    // The stub model does not support thinking: a non-clamping model
    // clamps "low" to "off" (spec setThinkingLevel), so use the
    // reasoning-capable variant for the persistence assertion.
    let mut base = request_with_model(&fixture, reasoning_model(&base_url), "", "rpc");
    let command = serde_json::json!({"type": "set_thinking_level", "id": "1", "level": "low"});
    base["rpcCommand"] = command.clone();
    let mut result = h.call_role("rpc", &base.to_string()).unwrap().unwrap();
    common::normalize_empty_object(&mut result);
    let lines: Vec<serde_json::Value> = result["writtenLines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| serde_json::from_str(line.as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(lines[0]["command"], "set_thinking_level");
    assert_eq!(lines[0]["success"], true);
    // The reasoning-capable model is now the runtime's model: reuse it.
    let all2 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_state", "id": "2"}),
    );
    let state = new_lines(&all2, lines.len());
    assert_eq!(state[0]["data"]["thinkingLevel"], "low");
    // set_model resolves against the model registry (spec: getAvailable);
    // the stub model is not registered, so it must fail with Pi's exact
    // message, and a registered catalog model must succeed.
    let all3 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "set_model", "id": "3", "provider": "anthropic", "modelId": "claude-parity-1"}),
    );
    let missing = new_lines(&all3, all2.len());
    assert_eq!(missing[0]["command"], "set_model");
    assert_eq!(missing[0]["success"], false);
    assert_eq!(
        missing[0]["error"],
        "Model not found: anthropic/claude-parity-1"
    );
    let all4 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "get_available_models", "id": "4"}),
    );
    let available = new_lines(&all4, all3.len());
    let model = available[0]["data"]["models"][0].clone();
    assert!(model["provider"].is_string());
    let all5 = rpc(
        &h,
        &fixture,
        &base_url,
        serde_json::json!({"type": "set_model", "id": "5", "provider": model["provider"], "modelId": model["id"]}),
    );
    let ok = new_lines(&all5, all4.len());
    assert_eq!(ok[0]["command"], "set_model");
    assert_eq!(ok[0]["success"], true);
    assert_eq!(ok[0]["data"]["id"], model["id"]);
}
