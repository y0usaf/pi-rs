#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 6.5 behavior pins for compaction: `/compact` sends the exact
//! summarization request (system prompt, serialized conversation,
//! model-clamped maxTokens), appends the compaction entry, and the next
//! provider request carries the compacted context exactly once; escape
//! cancels without appending; messages submitted during compaction queue
//! and flush afterwards; a big-usage turn triggers threshold
//! auto-compaction; a context-overflow error compacts and auto-retries
//! once. Frames are pinned by tests/ui-parity/compaction-turn.json.
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

/// One scripted assistant text turn as an SSE body, with scripted usage.
fn text_sse(text: &str, input_tokens: u64) -> String {
    format!(
        concat!(
            "event: message_start\n",
            "data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-parity-1\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":{input},\"output_tokens\":1}}}}}}\n\n",
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
        input = input_tokens,
        text = serde_json::Value::String(text.to_owned()),
    )
}

enum Scripted {
    Sse(String),
    Status(u16, String),
}

fn spawn_stub(responses: Vec<Scripted>) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for (index, conn) in listener.incoming().enumerate() {
            let Ok(mut stream) = conn else { break };
            let request = read_request(&mut stream);
            seen.lock().unwrap().push(request);
            let scripted = responses.get(index).or_else(|| responses.last());
            let response = match scripted {
                Some(Scripted::Sse(body)) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                ),
                Some(Scripted::Status(code, body)) => format!(
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                ),
                None => break,
            };
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

/// A linear two-turn session sized so keepRecentTokens=15 cuts at e5:
/// the summarized history is [e3 user, e4 assistant].
fn write_session(dir: &std::path::Path, cwd: &str) -> String {
    let entries = [
        serde_json::json!({"type": "session", "version": 3,
            "id": "0198a5c0-0000-7000-8000-000000000002",
            "timestamp": "2026-07-01T10:00:00.000Z", "cwd": cwd}),
        serde_json::json!({"type": "model_change", "id": "e1", "parentId": null,
            "timestamp": "2026-07-01T10:00:00.000Z",
            "provider": "anthropic", "modelId": "claude-parity-1"}),
        serde_json::json!({"type": "thinking_level_change", "id": "e2", "parentId": "e1",
            "timestamp": "2026-07-01T10:00:00.000Z", "thinkingLevel": "off"}),
        serde_json::json!({"type": "message", "id": "e3", "parentId": "e2",
            "timestamp": "2026-07-01T10:00:01.000Z",
            "message": {"role": "user",
                "content": [{"type": "text", "text": "Please refactor the parser module for me now"}],
                "timestamp": 1782813601000i64}}),
        serde_json::json!({"type": "message", "id": "e4", "parentId": "e3",
            "timestamp": "2026-07-01T10:00:02.000Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": "Sure, I will start by reading the parser files."}],
                "api": "anthropic-messages", "provider": "anthropic", "model": "claude-parity-1",
                "usage": {"input": 100, "output": 12, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 112,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": 1782813602000i64}}),
        serde_json::json!({"type": "message", "id": "e5", "parentId": "e4",
            "timestamp": "2026-07-01T10:00:03.000Z",
            "message": {"role": "user",
                "content": [{"type": "text", "text": "Now fix the error handling in the loader module"}],
                "timestamp": 1782813603000i64}}),
        serde_json::json!({"type": "message", "id": "e6", "parentId": "e5",
            "timestamp": "2026-07-01T10:00:04.000Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": "Done. The loader now retries with backoff."}],
                "api": "anthropic-messages", "provider": "anthropic", "model": "claude-parity-1",
                "usage": {"input": 150, "output": 20, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 170,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": 1782813604000i64}}),
    ];
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("s1.jsonl");
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

struct Fixture {
    _temp: tempfile::TempDir,
    cwd: String,
    agent_dir: std::path::PathBuf,
    sessions: std::path::PathBuf,
    session_file: String,
}

fn fixture(keep_recent_tokens: Option<u64>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    if let Some(keep) = keep_recent_tokens {
        // Project settings: force the cut into the fixture's small turns.
        let pi_dir = temp.path().join(".pi");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::write(
            pi_dir.join("settings.json"),
            serde_json::json!({"compaction": {"keepRecentTokens": keep}}).to_string(),
        )
        .unwrap();
    }
    let sessions = temp.path().join("sessions");
    let session_file = write_session(&sessions, &cwd);
    Fixture {
        _temp: temp,
        cwd,
        agent_dir,
        sessions,
        session_file,
    }
}

fn run_sequence(fixture: &Fixture, base_url: &str, steps: serde_json::Value) -> serde_json::Value {
    host(&fixture.cwd)
        .call_command(
            "interactive-tree-parity-sequence",
            &serde_json::json!({
                "columns": 90, "rows": 30,
                "model": stub_model(base_url), "apiKey": "test-key",
                "runtimeApiKey": "test-key",
                "cwd": fixture.cwd, "agentDir": fixture.agent_dir.to_string_lossy(),
                "sessionFile": fixture.session_file,
                "sessionDir": fixture.sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "nowMs": 1_782_900_000_000i64,
                "steps": steps,
            })
            .to_string(),
        )
        .expect("command")
        .expect("result")
}

fn paste(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~")
}

fn jsonl_entries(path: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn message_texts(request: &serde_json::Value) -> Vec<String> {
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

const SUMMARIZATION_SYSTEM_PROMPT: &str = "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.";

const COMPACTED_CONTEXT_PREFIX: &str = "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";

#[test]
fn manual_compact_sends_the_request_appends_the_entry_and_cuts_context() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture(Some(15));
    let (base_url, requests) = spawn_stub(vec![
        Scripted::Sse(text_sse("## Goal\nShip it.", 1)),
        Scripted::Sse(text_sse("compacted reply", 1)),
    ]);

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "compact", "input": [paste("/compact"), "\r"] },
            { "name": "next", "input": [paste("next question"), "\r"] },
        ]),
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);

    // The summarization request: system prompt, serialized history
    // ([e3, e4] — the cut keeps from e5), model-clamped maxTokens
    // (min(floor(0.8 * 16384), 1024) = 1024).
    let summarization = &requests[0];
    assert_eq!(summarization["max_tokens"], 1024);
    assert_eq!(
        summarization["system"][0]["text"].as_str().unwrap(),
        SUMMARIZATION_SYSTEM_PROMPT
    );
    let prompt = summarization["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        prompt.starts_with(
            "<conversation>\n[User]: Please refactor the parser module for me now\n\n[Assistant]: Sure, I will start by reading the parser files.\n</conversation>\n\n"
        ),
        "{prompt}"
    );
    assert!(
        prompt.contains("The messages above are a conversation to summarize."),
        "{prompt}"
    );

    // The compaction entry: summary, cut point, tokensBefore from the
    // last assistant usage, explicit fromHook false, empty file lists.
    let entries = jsonl_entries(&fixture.session_file);
    let compaction = entries
        .iter()
        .find(|entry| entry["type"] == "compaction")
        .expect("compaction entry");
    assert_eq!(compaction["summary"], "## Goal\nShip it.");
    assert_eq!(compaction["firstKeptEntryId"], "e5");
    assert_eq!(compaction["tokensBefore"], 170);
    assert_eq!(compaction["fromHook"], false);
    assert_eq!(compaction["details"]["readFiles"], serde_json::json!([]));
    assert_eq!(
        compaction["details"]["modifiedFiles"],
        serde_json::json!([])
    );

    // The next provider request carries the compacted context exactly
    // once: summary message, kept turn, new user message — the
    // summarized history is gone.
    let texts = message_texts(&requests[1]);
    assert_eq!(texts.len(), 4, "{texts:?}");
    assert_eq!(
        texts[0],
        format!("{COMPACTED_CONTEXT_PREFIX}## Goal\nShip it.\n</summary>")
    );
    assert_eq!(texts[1], "Now fix the error handling in the loader module");
    assert_eq!(texts[2], "Done. The loader now retries with backoff.");
    assert_eq!(texts[3], "next question");
}

#[test]
fn escape_cancels_compaction_without_appending() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture(Some(15));
    let before = std::fs::read_to_string(&fixture.session_file).unwrap();
    let (base_url, requests) = spawn_stub(vec![Scripted::Sse(text_sse("unused", 1))]);

    let result = run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            // Arm the trigger, submit /compact, and settle in the next
            // step: the escape fires synchronously at compaction_start,
            // before the summarization request leaves.
            { "input": [paste("/compact"), "\r"], "settle": false,
              "captures": [{ "event": "compaction_start", "action": "escape", "name": "cancelling" }] },
            { "name": "cancelled" },
        ]),
    );

    // Nothing was appended, and no summarization request was sent (the
    // signal aborts before the stream opens).
    assert_eq!(
        std::fs::read_to_string(&fixture.session_file).unwrap(),
        before
    );
    assert_eq!(requests.lock().unwrap().len(), 0);
    let frames = result["frames"].as_array().unwrap();
    let last = frames.last().unwrap();
    assert_eq!(last["name"], "cancelled");
    assert!(
        last["ansi"]
            .as_str()
            .unwrap()
            .contains("Error: Compaction cancelled"),
        "cancelled frame should show the manual-compaction error row"
    );
}

#[test]
fn messages_queued_during_compaction_flush_afterwards() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture(Some(15));
    let (base_url, requests) = spawn_stub(vec![
        Scripted::Sse(text_sse("## Goal\nShip it.", 1)),
        Scripted::Sse(text_sse("flushed reply", 1)),
    ]);

    let result = run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            // The submit trigger runs at compaction_start: isCompacting
            // is true, so the text queues instead of prompting.
            { "input": [paste("/compact"), "\r"], "settle": false,
              "captures": [{ "event": "compaction_start", "action": "submit",
                             "text": "queued question", "name": "queued" }] },
            { "name": "done" },
        ]),
    );

    // Queued frame: the status row and the pending steering row.
    let frames = result["frames"].as_array().unwrap();
    let queued = frames
        .iter()
        .find(|frame| frame["name"] == "queued")
        .unwrap();
    let ansi = queued["ansi"].as_str().unwrap();
    assert!(
        ansi.contains("Queued message for after compaction"),
        "{ansi}"
    );
    assert!(ansi.contains("Steering: queued question"), "{ansi}");

    // After compaction_end the queue flushed into a prompt: request 2
    // carries the compacted context plus the queued question.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let texts = message_texts(&requests[1]);
    assert_eq!(texts.last().unwrap(), "queued question");
    assert!(texts[0].starts_with(COMPACTED_CONTEXT_PREFIX), "{texts:?}");
    // The flushed turn persisted after the compaction entry.
    let entries = jsonl_entries(&fixture.session_file);
    let compaction_index = entries
        .iter()
        .position(|entry| entry["type"] == "compaction")
        .unwrap();
    let flushed_user = entries
        .iter()
        .position(|entry| {
            entry["type"] == "message"
                && entry["message"]["content"][0]["text"] == "queued question"
        })
        .unwrap();
    assert!(flushed_user > compaction_index);
}

#[test]
fn big_usage_turn_triggers_threshold_auto_compaction() {
    let _env = ENV_LOCK.lock().unwrap();
    // Default settings: reserve 16384 over a 200000 window — the scripted
    // usage (190000 input) crosses the threshold.
    let fixture = fixture(None);
    let (base_url, requests) = spawn_stub(vec![
        Scripted::Sse(text_sse("big reply", 190_000)),
        Scripted::Sse(text_sse("## Goal\nShip it.", 1)),
    ]);

    let result = run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "turn", "input": [paste("hello"), "\r"],
              "captures": [{ "event": "compaction_start", "name": "compacting" }] },
        ]),
    );

    // Two requests: the turn, then the summarization.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]["system"][0]["text"].as_str().unwrap(),
        SUMMARIZATION_SYSTEM_PROMPT
    );

    // The auto-compaction loader frame.
    let frames = result["frames"].as_array().unwrap();
    let compacting = frames
        .iter()
        .find(|frame| frame["name"] == "compacting")
        .unwrap();
    assert!(
        compacting["ansi"]
            .as_str()
            .unwrap()
            .contains("Auto-compacting... (escape to cancel)"),
        "auto-compaction loader label"
    );

    // The compaction entry landed.
    let entries = jsonl_entries(&fixture.session_file);
    assert!(entries.iter().any(|entry| entry["type"] == "compaction"));
}

#[test]
fn context_overflow_compacts_and_retries_once() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture(Some(15));
    let (base_url, requests) = spawn_stub(vec![
        Scripted::Status(
            400,
            serde_json::json!({"type": "error", "error": {"type": "invalid_request_error",
                "message": "prompt is too long: 213462 tokens > 200000 maximum"}})
            .to_string(),
        ),
        Scripted::Sse(text_sse("## Goal\nShip it.", 1)),
        Scripted::Sse(text_sse("retried reply", 1)),
    ]);

    let result = run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "turn", "input": [paste("hello"), "\r"],
              "captures": [{ "event": "compaction_start", "name": "overflow" }] },
        ]),
    );

    // Three requests: the failing turn, the summarization, the retry.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    // The retry carries the compacted context and not the error message.
    let texts = message_texts(&requests[2]);
    assert!(texts[0].starts_with(COMPACTED_CONTEXT_PREFIX), "{texts:?}");
    assert_eq!(texts.last().unwrap(), "hello");
    assert!(
        !requests[2].to_string().contains("prompt is too long"),
        "the overflow error must not reach the retry context"
    );

    // The overflow loader label.
    let frames = result["frames"].as_array().unwrap();
    let overflow = frames
        .iter()
        .find(|frame| frame["name"] == "overflow")
        .unwrap();
    assert!(
        overflow["ansi"]
            .as_str()
            .unwrap()
            .contains("Context overflow detected, Auto-compacting... (escape to cancel)"),
        "overflow loader label"
    );

    // Session history keeps the error turn, the compaction entry, and the
    // retried assistant reply, in order.
    let entries = jsonl_entries(&fixture.session_file);
    let error_index = entries
        .iter()
        .position(|entry| entry["message"]["stopReason"] == "error")
        .expect("error assistant persisted");
    let compaction_index = entries
        .iter()
        .position(|entry| entry["type"] == "compaction")
        .expect("compaction entry");
    let retry_index = entries
        .iter()
        .position(|entry| entry["message"]["content"][0]["text"] == "retried reply")
        .expect("retried assistant persisted");
    assert!(error_index < compaction_index && compaction_index < retry_index);
}
