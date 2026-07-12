//! Differential Codex Responses WebSocket/fallback replay (PLAN item 8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use futures_util::{SinkExt, StreamExt};
use pi_rs_ai::protocols::openai_codex_responses::{
    OpenAICodexResponsesOptions, close_openai_codex_websocket_sessions,
    get_openai_codex_websocket_debug_stats, reset_openai_codex_websocket_debug_stats,
    stream_openai_codex_responses,
};
use pi_rs_ai::protocols::options::StreamOptions;
use pi_rs_ai_types::{Context, Model, Transport};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const KEEP: &[&str] = &[
    "authorization",
    "chatgpt-account-id",
    "openai-beta",
    "originator",
    "session-id",
    "x-client-request-id",
];

fn selected<'a>(headers: impl Iterator<Item = (&'a str, &'a str)>) -> Value {
    let map: BTreeMap<_, _> = headers
        .filter(|(key, _)| KEEP.contains(key))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    json!(map)
}

async fn read_http(socket: &mut tokio::net::TcpStream) -> String {
    let mut all = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
        if let Some(pos) = all.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&all[..pos]).to_lowercase();
            let len = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if all.len() >= pos + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&all).into_owned()
}

fn normalize_http(raw: &str) -> Value {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("");
    let path = first.next().unwrap_or("");
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_lowercase(), value.trim().to_string()))
        .filter(|(key, _)| KEEP.contains(&key.as_str()))
        .collect::<BTreeMap<_, _>>();
    json!({"method":method,"path":path,"headers":headers,"body":serde_json::from_str::<Value>(body).unwrap_or(Value::Null)})
}

#[derive(Clone)]
struct Captured {
    ws: Arc<Mutex<Vec<Value>>>,
    http: Arc<Mutex<Vec<Value>>>,
}

fn serve(scenario: &Value) -> (std::net::SocketAddr, Captured) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let captured = Captured {
        ws: Arc::new(Mutex::new(Vec::new())),
        http: Arc::new(Mutex::new(Vec::new())),
    };
    let copy = captured.clone();
    let scenario = scenario.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut peek = [0; 2048];
            let n = socket.peek(&mut peek).await.unwrap_or(0);
            let is_websocket = String::from_utf8_lossy(&peek[..n])
                .to_ascii_lowercase()
                .contains("upgrade: websocket");
            let scenario = scenario.clone();
            let copy = copy.clone();
            tokio::spawn(async move {
                if is_websocket {
                    let handshake = Arc::new(Mutex::new(Value::Null));
                    let handshake_copy = Arc::clone(&handshake);
                    let Ok(mut websocket) = tokio_tungstenite::accept_hdr_async(
                        socket,
                        move |request: &Request, response: Response| {
                            let headers =
                                selected(request.headers().iter().filter_map(|(key, value)| {
                                    value.to_str().ok().map(|value| (key.as_str(), value))
                                }));
                            *handshake_copy.lock().unwrap() =
                                json!({"path":request.uri().path(),"headers":headers});
                            Ok(response)
                        },
                    )
                    .await
                    else {
                        return;
                    };
                    if scenario
                        .get("failBeforeStart")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        return;
                    }
                    let mut turn = 0usize;
                    while let Some(Ok(message)) = websocket.next().await {
                        let Message::Text(text) = message else {
                            continue;
                        };
                        let mut request = handshake.lock().unwrap().clone();
                        request["body"] = serde_json::from_str(&text).unwrap();
                        copy.ws.lock().unwrap().push(request);
                        if !scenario
                            .get("timeoutBeforeStart")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            for event in scenario["turns"][turn]["events"].as_array().unwrap() {
                                websocket
                                    .send(Message::Text(event.to_string().into()))
                                    .await
                                    .unwrap();
                            }
                        }
                        turn += 1;
                    }
                } else {
                    let raw = read_http(&mut socket).await;
                    copy.http.lock().unwrap().push(normalize_http(&raw));
                    let body = scenario["turns"][0]["events"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|event| format!("data: {}\n\n", event))
                        .collect::<String>();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            });
        }
    });
    (addr, captured)
}

fn transport(value: &str) -> Transport {
    match value {
        "sse" => Transport::Sse,
        "websocket" => Transport::Websocket,
        "websocket-cached" => Transport::WebsocketCached,
        _ => Transport::Auto,
    }
}

fn summarize(event: &pi_rs_ai_types::AssistantMessageEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("partial");
    object.remove("message");
    object.remove("error");
    value
}

fn scrub_result(mut value: Value) -> Value {
    value["timestamp"] = json!(0);
    if let Some(diagnostics) = value.get_mut("diagnostics").and_then(Value::as_array_mut) {
        for diagnostic in diagnostics {
            diagnostic["timestamp"] = json!(0);
            diagnostic
                .get_mut("error")
                .and_then(Value::as_object_mut)
                .map(|error| error.remove("stack"));
        }
    }
    value
}

fn canonicalize(value: &mut Value) {
    match value {
        Value::Number(number)
            if number.as_u64().is_none()
                && number.as_i64().is_none()
                && number.as_f64().is_some_and(|value| value.fract() == 0.0) =>
        {
            *value = json!(number.as_f64().unwrap() as i64);
        }
        Value::Array(values) => values.iter_mut().for_each(canonicalize),
        Value::Object(values) => values.values_mut().for_each(canonicalize),
        _ => {}
    }
}
#[tokio::test]
async fn pi_rs_matches_pi_codex_websocket_oracle() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/openai-codex-websocket-parity");
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let oracle: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("oracle.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for (scenario, expected) in cases["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["scenarios"].as_array().unwrap())
    {
        let session = scenario["sessionId"].as_str().unwrap();
        reset_openai_codex_websocket_debug_stats(Some(session));
        close_openai_codex_websocket_sessions(Some(session));
        let (addr, captured) = serve(scenario);
        let mut model = cases["model"].clone();
        model["baseUrl"] = json!(format!("http://{addr}"));
        let model: Model = serde_json::from_value(model).unwrap();
        let mut turns = Vec::new();
        for turn in scenario["turns"].as_array().unwrap() {
            let context: Context = serde_json::from_value(turn["context"].clone()).unwrap();
            let stream = stream_openai_codex_responses(
                &model,
                &context,
                Some(OpenAICodexResponsesOptions {
                    base: StreamOptions {
                        api_key: Some(cases["token"].as_str().unwrap().to_string()),
                        transport: Some(transport(scenario["transport"].as_str().unwrap())),
                        session_id: Some(session.to_string()),
                        timeout_ms: scenario.get("timeoutMs").and_then(Value::as_u64),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(summarize(&event));
            }
            turns.push(json!({"events":events,"result":scrub_result(serde_json::to_value(stream.result().await.unwrap()).unwrap())}));
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let mut actual = json!({"name":scenario["name"],"wsRequests":captured.ws.lock().unwrap().clone(),
            "httpRequests":captured.http.lock().unwrap().clone(),"turns":turns,
            "stats":get_openai_codex_websocket_debug_stats(session)});
        let mut expected = expected.clone();
        canonicalize(&mut actual);
        canonicalize(&mut expected);
        if actual != expected {
            failures.push(format!(
                "{}\nexpected={}\nactual={}",
                scenario["name"],
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual).unwrap()
            ));
        }
        close_openai_codex_websocket_sessions(Some(session));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
