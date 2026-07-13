//! Pi-derived differential OpenAI Chat Completions replay (PLAN item 8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::protocols::openai_completions::{
    OpenAICompletionsOptions, OpenAIToolChoice, stream_openai_completions,
    stream_simple_openai_completions,
};
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai_types::{Context, Model, ThinkingLevel};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Captured = Arc<Mutex<Vec<String>>>;

fn response(value: &Value) -> String {
    let (body, content_type) = if let Some(events) = value.get("events").and_then(Value::as_array) {
        (
            events
                .iter()
                .map(|event| format!("data: {}\n\n", serde_json::to_string(event).unwrap()))
                .collect::<String>()
                + "data: [DONE]\n\n",
            "text/event-stream",
        )
    } else if let Some(body) = value.get("json") {
        (serde_json::to_string(body).unwrap(), "application/json")
    } else {
        (
            value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            "text/plain",
        )
    };
    format!(
        "HTTP/1.1 {} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        value["status"].as_u64().unwrap(),
        body.len()
    )
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

fn serve(responses: Vec<String>) -> (std::net::SocketAddr, Captured) {
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
            let _ = socket.write_all(value.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (address, captured)
}

const DROP: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "accept-encoding",
    "accept-language",
    "sec-fetch-mode",
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
        let value = value.trim();
        if !DROP.contains(&name.as_str())
            && !name.starts_with("x-stainless-")
            && !(name == "user-agent" && !value.starts_with("claude-cli/"))
        {
            headers.insert(name, value.to_string());
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
        api_key: value
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_string),
        max_tokens: value.get("maxTokens").and_then(Value::as_u64),
        temperature: value.get("temperature").and_then(Value::as_f64),
        session_id: value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
        headers: value.get("headers").and_then(Value::as_object).map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
                .collect()
        }),
        ..Default::default()
    }
}
fn options(value: &Value) -> OpenAICompletionsOptions {
    OpenAICompletionsOptions {
        base: base_options(value),
        reasoning_effort: value.get("reasoningEffort").and_then(thinking),
        tool_choice: value
            .get("toolChoice")
            .and_then(Value::as_str)
            .map(|choice| match choice {
                "none" => OpenAIToolChoice::None,
                "required" => OpenAIToolChoice::Required,
                _ => OpenAIToolChoice::Auto,
            }),
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
    let stream = if case.get("simple").and_then(Value::as_bool).unwrap_or(false) {
        match stream_simple_openai_completions(
            &model,
            &context,
            Some(SimpleStreamOptions {
                base: base_options(values),
                reasoning: values.get("reasoning").and_then(thinking),
                thinking_budgets: None,
            }),
        ) {
            Ok(stream) => stream,
            Err(error) => {
                return json!({"name":case["name"],"requests":[],"syncError":error.to_string()});
            }
        }
    } else {
        stream_openai_completions(&model, &context, Some(options(values)))
    };
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(summarize(&event));
    }
    let mut result = serde_json::to_value(stream.result().await.unwrap()).unwrap();
    result["timestamp"] = json!(0);
    let requests = captured
        .lock()
        .unwrap()
        .iter()
        .map(|raw| normalize_request(raw))
        .collect::<Vec<_>>();
    json!({"name":case["name"],"requests":requests,"events":events,"result":result})
}

#[tokio::test]
async fn pi_rs_matches_pi_openai_completions_oracle() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/openai-completions-parity");
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
