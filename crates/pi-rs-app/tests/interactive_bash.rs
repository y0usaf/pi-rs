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

enum Scripted {
    Sse(String),
    Hang(String),
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
            match responses.get(index).or_else(|| responses.last()) {
                Some(Scripted::Sse(body)) => {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Some(Scripted::Hang(body)) => {
                    // No content-length: the body runs until close, which
                    // never comes — the client must abort.
                    let response =
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n{body}");
                    let _ = stream.write_all(response.as_bytes());
                    thread::spawn(move || {
                        let mut sink = [0u8; 64];
                        let _ = stream.read(&mut sink);
                    });
                }
                None => break,
            }
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

fn paste(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~")
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

#[test]
fn idle_bash_persists_and_reaches_the_next_request_as_text() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture();
    let (base_url, requests) = spawn_stub(vec![Scripted::Sse(text_sse("done"))]);

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "bash", "input": [paste("!printf hi"), "\r"], "waitBash": true },
            { "name": "next", "input": [paste("next question"), "\r"] },
        ]),
    );

    // The bashExecution entry persisted with the executor's result shape.
    let entries = session_entries(&fixture);
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
        message_texts(&requests[0]),
        vec![
            "Ran `printf hi`\n```\nhi\n```".to_owned(),
            "next question".to_owned(),
        ],
    );
}

#[test]
fn excluded_bash_persists_but_never_reaches_the_provider() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture();
    let (base_url, requests) = spawn_stub(vec![Scripted::Sse(text_sse("done"))]);

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            { "name": "bash", "input": [paste("!!printf hi"), "\r"], "waitBash": true },
            { "name": "next", "input": [paste("next question"), "\r"] },
        ]),
    );

    let entries = session_entries(&fixture);
    let bash = entries
        .iter()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "bashExecution")
        .expect("bashExecution entry");
    assert_eq!(bash["message"]["excludeFromContext"], true);

    // Excluded from context: only the user message reaches the provider.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        message_texts(&requests[0]),
        vec!["next question".to_owned()]
    );
}

#[test]
fn deferred_bash_flushes_after_the_turn_settles() {
    let _env = ENV_LOCK.lock().unwrap();
    let fixture = fixture();
    let (base_url, requests) = spawn_stub(vec![
        Scripted::Hang(hang_sse()),
        Scripted::Sse(text_sse("you're welcome")),
    ]);

    run_sequence(
        &fixture,
        &base_url,
        serde_json::json!([
            {
                "input": [paste("Tell me a story"), "\r"],
                "waitIdle": false,
                "captures": [{ "name": "streaming", "event": "message_update", "count": 3 }],
            },
            { "name": "deferred", "input": [paste("!printf deferred"), "\r"],
              "waitBash": true, "waitIdle": false },
            { "name": "aborted", "input": ["\u{1b}"] },
            { "name": "thanks", "input": [paste("thanks"), "\r"] },
        ]),
    );

    // JSONL order: the aborted assistant settles before the deferred
    // bashExecution flushes (agent-session.ts _runAgentPrompt finally).
    let entries = session_entries(&fixture);
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
    let texts = message_texts(&requests[1]);
    assert_eq!(
        texts,
        vec![
            "Tell me a story".to_owned(),
            "Ran `printf deferred`\n```\ndeferred\n```".to_owned(),
            "thanks".to_owned(),
        ],
    );
}
