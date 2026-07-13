//! Differential Amazon Bedrock Converse Stream replay (PLAN item 8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::protocols::bedrock::{
    BedrockOptions, BedrockThinkingDisplay, BedrockToolChoice, stream_bedrock,
    stream_simple_bedrock,
};
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai_types::{Context, Model, ThinkingLevel};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Captured = Arc<Mutex<Vec<String>>>;
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
fn header(name: &str, value: &str, output: &mut Vec<u8>) {
    output.push(name.len() as u8);
    output.extend(name.as_bytes());
    output.push(7);
    output.extend((value.len() as u16).to_be_bytes());
    output.extend(value.as_bytes());
}
fn frame(event: &Value) -> Vec<u8> {
    let mut headers = Vec::new();
    header(":event-type", event["type"].as_str().unwrap(), &mut headers);
    header(":message-type", "event", &mut headers);
    header(":content-type", "application/json", &mut headers);
    let body = event["value"].to_string();
    let total = 16 + headers.len() + body.len();
    let mut result = Vec::new();
    result.extend((total as u32).to_be_bytes());
    result.extend((headers.len() as u32).to_be_bytes());
    result.extend(crc32(&result).to_be_bytes());
    result.extend(headers);
    result.extend(body.as_bytes());
    result.extend(crc32(&result).to_be_bytes());
    result
}
fn response(value: &Value) -> Vec<u8> {
    let (body, content_type) = if let Some(events) = value.get("events").and_then(Value::as_array) {
        (
            events.iter().flat_map(frame).collect::<Vec<_>>(),
            "application/vnd.amazon.eventstream",
        )
    } else if let Some(body) = value.get("json") {
        (body.to_string().into_bytes(), "application/json")
    } else {
        (
            value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
            "text/plain",
        )
    };
    let head = format!(
        "HTTP/1.1 {} X\r\ncontent-type: {content_type}\r\nx-amzn-requestid: fixture-request\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        value["status"].as_u64().unwrap(),
        body.len()
    );
    [head.into_bytes(), body].concat()
}
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut all = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = socket.read(&mut buffer).await.unwrap_or(0);
        if count == 0 {
            break;
        }
        all.extend_from_slice(&buffer[..count]);
        if let Some(position) = all.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&all[..position]).to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if all.len() >= position + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&all).into_owned()
}
fn serve(responses: Vec<Vec<u8>>) -> (std::net::SocketAddr, Captured) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let copy = Arc::clone(&captured);
    tokio::spawn(async move {
        let mut index = 0;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut socket).await;
            copy.lock().unwrap().push(request);
            let Some(value) = responses.get(index).or_else(|| responses.last()) else {
                return;
            };
            index += 1;
            let _ = socket.write_all(value).await;
            let _ = socket.shutdown().await;
        }
    });
    (address, captured)
}
const DROP: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "accept",
    "accept-encoding",
    "user-agent",
    "amz-sdk-invocation-id",
    "amz-sdk-request",
    "x-amz-user-agent",
];
fn normalize_request(raw: &str) -> Value {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("");
    let path = first.next().unwrap_or("");
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        if !DROP.contains(&name.as_str()) {
            headers.insert(name, value.trim().to_string());
        }
    }
    json!({"method":method,"path":path,"headers":headers,"body":if body.is_empty(){Value::Null}else{serde_json::from_str(body).unwrap()}})
}
fn thinking(value: &Value) -> Option<ThinkingLevel> {
    value.as_str().map(|value| match value {
        "minimal" => ThinkingLevel::Minimal,
        "low" => ThinkingLevel::Low,
        "medium" => ThinkingLevel::Medium,
        "high" => ThinkingLevel::High,
        "xhigh" => ThinkingLevel::XHigh,
        _ => ThinkingLevel::Max,
    })
}
fn base_options(value: &Value) -> StreamOptions {
    StreamOptions {
        max_tokens: value.get("maxTokens").and_then(Value::as_u64),
        temperature: value.get("temperature").and_then(Value::as_f64),
        cache_retention: value
            .get("cacheRetention")
            .map(|value| serde_json::from_value(value.clone()).unwrap()),
        headers: value.get("headers").and_then(Value::as_object).map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
                .collect()
        }),
        ..Default::default()
    }
}
fn options(value: &Value) -> BedrockOptions {
    BedrockOptions {
        base: base_options(value),
        region: value
            .get("region")
            .and_then(Value::as_str)
            .map(str::to_string),
        profile: value
            .get("profile")
            .and_then(Value::as_str)
            .map(str::to_string),
        tool_choice: value
            .get("toolChoice")
            .and_then(Value::as_str)
            .map(|choice| match choice {
                "any" => BedrockToolChoice::Any,
                "none" => BedrockToolChoice::None,
                _ => BedrockToolChoice::Auto,
            }),
        reasoning: value.get("reasoning").and_then(thinking),
        thinking_budgets: None,
        interleaved_thinking: value.get("interleavedThinking").and_then(Value::as_bool),
        thinking_display: value
            .get("thinkingDisplay")
            .and_then(Value::as_str)
            .map(|value| {
                if value == "omitted" {
                    BedrockThinkingDisplay::Omitted
                } else {
                    BedrockThinkingDisplay::Summarized
                }
            }),
        request_metadata: value
            .get("requestMetadata")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
                    .collect()
            }),
        bearer_token: value
            .get("bearerToken")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}
fn summarize(event: &pi_rs_ai_types::AssistantMessageEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap();
    let map = value.as_object_mut().unwrap();
    map.remove("partial");
    map.remove("message");
    map.remove("error");
    value
}
fn canonicalize(value: &mut Value) {
    match value {
        Value::Number(number)
            if number.as_u64().is_none()
                && number.as_i64().is_none()
                && number.as_f64().is_some_and(|value| value.fract() == 0.0) =>
        {
            *value = json!(number.as_f64().unwrap() as i64)
        }
        Value::Array(items) => items.iter_mut().for_each(canonicalize),
        Value::Object(map) => map.values_mut().for_each(canonicalize),
        _ => {}
    }
}
async fn run(case: &Value, models: &Value) -> Value {
    let responses = case["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(response)
        .collect();
    let (address, captured) = serve(responses);
    let mut model = models[case["model"].as_str().unwrap()].clone();
    model["baseUrl"] = json!(format!("http://{address}"));
    let model: Model = serde_json::from_value(model).unwrap();
    let context: Context = serde_json::from_value(case["context"].clone()).unwrap();
    let values = &case["options"];
    let old_bearer = std::env::var_os("AWS_BEARER_TOKEN_BEDROCK");
    if case.get("simple").and_then(Value::as_bool).unwrap_or(false)
        && let Some(token) = values.get("bearerToken").and_then(Value::as_str)
    {
        unsafe { std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", token) }
    }
    let stream = if case.get("simple").and_then(Value::as_bool).unwrap_or(false) {
        stream_simple_bedrock(
            &model,
            &context,
            Some(SimpleStreamOptions {
                base: base_options(values),
                reasoning: values.get("reasoning").and_then(thinking),
                thinking_budgets: None,
            }),
        )
        .unwrap()
    } else {
        stream_bedrock(&model, &context, Some(options(values)))
    };
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(summarize(&event));
    }
    let mut result = serde_json::to_value(stream.result().await.unwrap()).unwrap();
    result["timestamp"] = json!(0);
    if let Some(value) = old_bearer {
        unsafe { std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", value) }
    } else {
        unsafe { std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK") }
    }
    let requests = captured
        .lock()
        .unwrap()
        .iter()
        .map(|raw| normalize_request(raw))
        .collect::<Vec<_>>();
    json!({"name":case["name"],"requests":requests,"events":events,"result":result})
}
#[tokio::test]
async fn pi_rs_matches_pi_bedrock_converse_stream_oracle() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/bedrock-converse-stream-parity");
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("cases.json")).unwrap())
            .unwrap();
    let oracle: Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("oracle.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    for (case, expected) in cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let mut actual = run(case, &cases["models"]).await;
        let mut expected = expected.clone();
        canonicalize(&mut actual);
        canonicalize(&mut expected);
        if actual != expected {
            failures.push(format!(
                "{}\nexpected={}\nactual={}",
                case["name"],
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual).unwrap()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
