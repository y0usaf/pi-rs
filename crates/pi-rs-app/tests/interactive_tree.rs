#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 6.4 behavior pins for tree navigation: `/tree` with "Summarize"
//! sends the exact branch-summarization request (system prompt,
//! serialized conversation, maxTokens 2048) and appends the
//! branch_summary entry at the navigation target; escape aborts the
//! summarizer; `/fork` copies the active path into a new session file
//! (labels on the path recreated); `/clone` duplicates at the leaf. The
//! selector/tree frames themselves are pinned by
//! tests/ui-parity/tree-turn.json.
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

/// One scripted summary turn as an SSE body.
const SUMMARY_SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-parity-1\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"## Goal\\nShip it.\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":4}}\n\n",
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
                SUMMARY_SSE.len(),
                SUMMARY_SSE
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
        "baseUrl": base_url, "reasoning": false,
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 200000, "maxTokens": 1024
    })
}

/// A branched session: root chain e1..e4, branch A (e5 user, e6 assistant
/// with a read toolCall, e7 toolResult), branch B (e8 user, e9 assistant),
/// leaf label entry targeting e5.
fn write_branched_session(dir: &std::path::Path, cwd: &str) -> String {
    let entries = [
        serde_json::json!({"type": "session", "version": 3,
            "id": "0198a5c0-0000-7000-8000-000000000001",
            "timestamp": "2026-07-01T10:00:00.000Z", "cwd": cwd}),
        serde_json::json!({"type": "model_change", "id": "e1", "parentId": null,
            "timestamp": "2026-07-01T10:00:00.000Z",
            "provider": "anthropic", "modelId": "claude-parity-1"}),
        serde_json::json!({"type": "thinking_level_change", "id": "e2", "parentId": "e1",
            "timestamp": "2026-07-01T10:00:00.000Z", "thinkingLevel": "off"}),
        serde_json::json!({"type": "message", "id": "e3", "parentId": "e2",
            "timestamp": "2026-07-01T10:00:01.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "start"}],
                "timestamp": 1782813601000i64}}),
        serde_json::json!({"type": "message", "id": "e4", "parentId": "e3",
            "timestamp": "2026-07-01T10:00:02.000Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": "ok"}],
                "api": "anthropic-messages", "provider": "anthropic", "model": "claude-parity-1",
                "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": 1782813602000i64}}),
        serde_json::json!({"type": "message", "id": "e5", "parentId": "e4",
            "timestamp": "2026-07-01T10:01:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "try A"}],
                "timestamp": 1782813660000i64}}),
        serde_json::json!({"type": "message", "id": "e6", "parentId": "e5",
            "timestamp": "2026-07-01T10:01:01.000Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": "reading"},
                    {"type": "toolCall", "id": "tc1", "name": "read",
                     "arguments": {"path": "src/a.ts"}}],
                "api": "anthropic-messages", "provider": "anthropic", "model": "claude-parity-1",
                "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "toolUse", "timestamp": 1782813661000i64}}),
        serde_json::json!({"type": "message", "id": "e7", "parentId": "e6",
            "timestamp": "2026-07-01T10:01:02.000Z",
            "message": {"role": "toolResult", "toolCallId": "tc1", "toolName": "read",
                "content": [{"type": "text", "text": "const a = 1;"}], "isError": false,
                "timestamp": 1782813662000i64}}),
        serde_json::json!({"type": "message", "id": "e8", "parentId": "e4",
            "timestamp": "2026-07-01T10:02:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "try B"}],
                "timestamp": 1782813720000i64}}),
        serde_json::json!({"type": "message", "id": "e9", "parentId": "e8",
            "timestamp": "2026-07-01T10:02:01.000Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": "B done"}],
                "api": "anthropic-messages", "provider": "anthropic", "model": "claude-parity-1",
                "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": 1782813721000i64}}),
        serde_json::json!({"type": "label", "id": "e10", "parentId": "e9",
            "timestamp": "2026-07-01T10:03:00.000Z", "targetId": "e5", "label": "branch-a"}),
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

#[test]
fn summarize_navigation_sends_the_branch_summary_request_and_appends_the_entry() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    let s1 = write_branched_session(&sessions, &cwd);

    let (base_url, requests) = spawn_stub();
    let result = host(&cwd)
        .call_command(
            "interactive-tree-parity-sequence",
            &serde_json::json!({
                "columns": 80, "rows": 30,
                "model": stub_model(&base_url), "apiKey": "test-key",
                "runtimeApiKey": "test-key",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": s1, "sessionDir": sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "nowMs": 1_782_900_000_000i64,
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
                "steps": [
                    // /tree → select e5 ("try A", the abandoned-branch
                    // user message) → "Summarize".
                    { "name": "open", "input": [paste("/tree"), "\r"] },
                    // Tree rows (default filter, active branch B first):
                    // e3, e4, e8, e9, e5, e6, e7 — selection starts on e9
                    // (nearest visible from the label-entry leaf).
                    { "name": "to-a", "input": ["\u{1b}[B"] },
                    { "name": "choose", "input": ["\r"] },
                    { "name": "summarize", "input": ["\u{1b}[B", "\r"] },
                ],
            })
            .to_string(),
        )
        .expect("command")
        .expect("result");

    // The summarization request: system prompt, serialized conversation,
    // and the default prompt with maxTokens 2048.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request["max_tokens"], 2048);
    assert_eq!(
        request["system"][0]["text"].as_str().unwrap(),
        "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary."
    );
    let prompt = request["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    // Old leaf (label entry) back to the common ancestor e4: e8, e9 in
    // chronological order, serialized.
    assert!(
        prompt.starts_with(
            "<conversation>\n[User]: try B\n\n[Assistant]: B done\n</conversation>\n\n"
        ),
        "{prompt}"
    );
    assert!(
        prompt.contains("Create a structured summary of this conversation branch"),
        "{prompt}"
    );
    assert!(prompt.ends_with("Preserve exact file paths, function names, and error messages."));

    // The branch_summary entry landed at the navigation target (e4, the
    // parent of the selected user message), with preamble + summary and
    // empty file lists (no read/write tool calls on the abandoned path).
    let entries = jsonl_entries(&s1);
    let summary = entries
        .iter()
        .find(|entry| entry["type"] == "branch_summary")
        .expect("branch_summary entry");
    assert_eq!(summary["parentId"], "e4");
    assert_eq!(summary["fromId"], "e4");
    assert_eq!(summary["fromHook"], false);
    assert_eq!(
        summary["summary"].as_str().unwrap(),
        "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n## Goal\nShip it."
    );
    // The editor was prefilled with the selected user message text.
    assert_eq!(result["editorText"].as_str().unwrap(), "try A");
    // The new leaf is the summary entry.
    assert_eq!(result["leafId"], summary["id"]);
}

#[test]
fn escape_aborts_summarization_and_navigation_is_cancelled() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    let s1 = write_branched_session(&sessions, &cwd);
    let before = std::fs::read_to_string(&s1).unwrap();

    // A hanging stub: never responds, so only an abort settles the task.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { break };
            // Hold the connection open; the client aborts.
            thread::spawn(move || {
                let mut stream = stream;
                let mut sink = [0u8; 64];
                while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
            });
        }
    });

    let result = host(&cwd)
        .call_command(
            "interactive-tree-parity-sequence",
            &serde_json::json!({
                "columns": 80, "rows": 30,
                "model": stub_model(&base_url), "apiKey": "test-key",
                "runtimeApiKey": "test-key",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": s1, "sessionDir": sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "nowMs": 1_782_900_000_000i64,
                "steps": [
                    { "name": "open", "input": [paste("/tree"), "\r"] },
                    { "name": "to-a", "input": ["\u{1b}[B"] },
                    { "name": "choose", "input": ["\r"] },
                    // Select "Summarize" but do not settle: the spawned
                    // task must observe the escape-abort.
                    { "name": "summarize", "input": ["\u{1b}[B", "\r"], "settle": false },
                    { "name": "abort", "input": ["\u{1b}"] },
                ],
            })
            .to_string(),
        )
        .expect("command")
        .expect("result");

    // Nothing was appended and the leaf did not move.
    assert_eq!(std::fs::read_to_string(&s1).unwrap(), before);
    assert_eq!(result["leafId"], "e10");
    // The tree selector re-opened on the previously selected entry: the
    // last frame is the abort step's re-shown tree.
    let frames = result["frames"].as_array().unwrap();
    let last = frames.last().unwrap();
    assert_eq!(last["name"], "abort");
    assert!(
        last["ansi"].as_str().unwrap().contains("Session Tree"),
        "tree selector should re-open after an aborted summarization"
    );
}

#[test]
fn fork_copies_the_path_before_the_selected_user_message_into_a_new_session() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    let s1 = write_branched_session(&sessions, &cwd);
    let s1_before = std::fs::read_to_string(&s1).unwrap();

    let (base_url, _requests) = spawn_stub();
    let result = host(&cwd)
        .call_command(
            "interactive-tree-parity-sequence",
            &serde_json::json!({
                "columns": 80, "rows": 30,
                "model": stub_model(&base_url), "apiKey": "test-key",
                "runtimeApiKey": "test-key",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": s1, "sessionDir": sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "nowMs": 1_782_900_000_000i64,
                "steps": [
                    // /fork → user messages [start, try A, try B]; the
                    // most recent (try B) starts selected; up once → try A.
                    { "name": "open", "input": [paste("/fork"), "\r"] },
                    { "name": "up", "input": ["\u{1b}[A"] },
                    { "name": "select", "input": ["\r"] },
                ],
            })
            .to_string(),
        )
        .expect("command")
        .expect("result");

    // A new session file holding only root→e4 (labels off-path dropped),
    // with the source as parentSession; the editor got the message text.
    let forked = result["sessionFile"].as_str().unwrap().to_owned();
    assert_ne!(forked, s1);
    assert_eq!(result["editorText"].as_str().unwrap(), "try A");
    let entries = jsonl_entries(&forked);
    assert_eq!(entries[0]["type"], "session");
    assert_eq!(entries[0]["parentSession"].as_str().unwrap(), s1);
    let ids: Vec<&str> = entries[1..]
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["e1", "e2", "e3", "e4"]);
    // The source is untouched.
    assert_eq!(std::fs::read_to_string(&s1).unwrap(), s1_before);
}

#[test]
fn clone_duplicates_the_session_at_the_leaf_with_path_labels_recreated() {
    let _env = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // SAFETY: serialized by ENV_LOCK; this binary owns the env.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions");
    let s1 = write_branched_session(&sessions, &cwd);

    let (base_url, _requests) = spawn_stub();
    let result = host(&cwd)
        .call_command(
            "interactive-tree-parity-sequence",
            &serde_json::json!({
                "columns": 80, "rows": 30,
                "model": stub_model(&base_url), "apiKey": "test-key",
                "runtimeApiKey": "test-key",
                "cwd": cwd, "agentDir": agent_dir.to_string_lossy(),
                "sessionFile": s1, "sessionDir": sessions.to_string_lossy(),
                "modelFromCli": true, "thinkingFromCli": false,
                "nowMs": 1_782_900_000_000i64,
                "steps": [
                    { "name": "clone", "input": [paste("/clone"), "\r"] },
                ],
            })
            .to_string(),
        )
        .expect("command")
        .expect("result");

    // Clone forks at the leaf (the label entry): the path from root to
    // the label entry, with the on-path label (targeting e5? no — e5 is
    // off-path; the label entry itself is filtered and only labels whose
    // target is on the path are recreated — e5 is not, so none are).
    let cloned = result["sessionFile"].as_str().unwrap().to_owned();
    assert_ne!(cloned, s1);
    let entries = jsonl_entries(&cloned);
    assert_eq!(entries[0]["parentSession"].as_str().unwrap(), s1);
    let ids: Vec<&str> = entries[1..]
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    // Path root→label-entry without label entries: e1, e2, e3, e4, e8, e9.
    assert_eq!(ids, ["e1", "e2", "e3", "e4", "e8", "e9"]);
}
