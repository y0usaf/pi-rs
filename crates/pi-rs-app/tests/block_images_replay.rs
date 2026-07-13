#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 5.3: the sdk.ts `convertToLlmWithBlockImages` filter behind the
//! `pi.settings` seam. With `images.blockImages` set, a read-tool image
//! result reaches the session intact (defense-in-depth: the tool still
//! reads) but the next provider request replaces the image block with
//! the spec's "Image reading is disabled." placeholder.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use pi_rs_app::builtins::{CODING_AGENT_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

// 1x1 red PNG (within every resize limit: auto-resize passes the
// original bytes through).
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 218, 99, 252, 207, 192, 240, 31, 0,
    5, 5, 2, 0, 95, 200, 241, 210, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

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

const FIRST_TURN: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-4-7\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_img\",\"name\":\"read\",\"input\":{}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\": \\\"pic.png\\\"}\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":9}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n"
);

const SECOND_TURN: &str = concat!(
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

#[test]
fn block_images_filters_provider_context_but_not_the_session() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&requests);
    let server = thread::spawn(move || {
        for body in [FIRST_TURN, SECOND_TURN] {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            seen.lock().unwrap().push(request);
            stream.write_all(response(body).as_bytes()).unwrap();
        }
    });

    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let cwd = temp.path().to_string_lossy().into_owned();
    std::fs::write(temp.path().join("pic.png"), TINY_PNG).unwrap();
    // Canonical trusted project config switches the filter on.
    std::fs::create_dir_all(temp.path().join(".pi")).unwrap();
    std::fs::write(
        temp.path().join(".pi/config.lua"),
        "local pi = ...\npi.config.settings({ images = { blockImages = true } })\n",
    )
    .unwrap();

    let model = serde_json::json!({
        "id": "claude-opus-4-7", "name": "Claude Opus 4.7",
        "api": "anthropic-messages", "provider": "anthropic",
        "baseUrl": format!("http://{}", address), "reasoning": true,
        "input": ["text", "image"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 100000, "maxTokens": 1024
    });
    let host = Host::new(HostConfig {
        cwd: Some(cwd.clone()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let result = host
        .call_role(
            "print",
            &serde_json::json!({
                "model": model, "apiKey": "test-key", "prompt": "read the image",
                "cwd": cwd,
                "agentDir": agent_dir.to_string_lossy(),
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples",
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    server.join().unwrap();
    assert_eq!(result["text"], "done");

    // The second request carries the tool result with the image replaced
    // by the spec's placeholder — and no image block anywhere.
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    let body = &captured[1];
    let tool_result = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .find(|block| block["type"] == "tool_result")
        .expect("tool_result block");
    // With the image filtered to a text placeholder the anthropic
    // protocol collapses the all-text result into a joined string
    // (unfiltered, this request carries a content array with an image
    // block — the negative-control shape).
    assert_eq!(
        tool_result["content"],
        "Read image file [image/png]\nImage reading is disabled."
    );

    // Defense-in-depth: the session keeps the real image content.
    let session_text = std::fs::read_to_string(result["sessionPath"].as_str().unwrap()).unwrap();
    let has_image = session_text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|entry| entry.get("message").cloned())
        .filter(|message| message["role"] == "toolResult")
        .any(|message| {
            message["content"]
                .as_array()
                .is_some_and(|content| content.iter().any(|block| block["type"] == "image"))
        });
    assert!(has_image, "session toolResult should keep the image block");
}
