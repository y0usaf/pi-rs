//! Differential Azure OpenAI Responses protocol replay (PLAN item 8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::protocols::azure_openai_responses::{
    AzureOpenAIResponsesOptions, stream_azure_openai_responses,
    stream_simple_azure_openai_responses,
};
use pi_rs_ai::protocols::openai_responses::ReasoningSummary;
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai_types::{Context, Model, ThinkingLevel};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Captured = Arc<Mutex<Vec<String>>>;
fn response(value: &Value, shared: &Value) -> String {
    let events = value
        .get("sse")
        .and_then(Value::as_str)
        .map(|name| shared[name].clone())
        .or_else(|| value.get("events").cloned());
    let (body, content_type) = if let Some(events) = events {
        (
            events
                .as_array()
                .unwrap()
                .iter()
                .map(|event| format!("data: {}\n\n", serde_json::to_string(event).unwrap()))
                .collect(),
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
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if all.len() >= pos + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&all).into_owned()
}
fn serve(responses: Vec<String>) -> (std::net::SocketAddr, Captured) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
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
    (addr, captured)
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
    let first = lines.next().unwrap_or("");
    let mut request = first.split(' ');
    let method = request.next().unwrap_or("");
    let path = request.next().unwrap_or("");
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let value = value.trim();
        if DROP.contains(&name.as_str())
            || name.starts_with("x-stainless-")
            || (name == "user-agent" && !value.starts_with("claude-cli/"))
        {
            continue;
        }
        headers.insert(name, value.to_string());
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
        cache_retention: value
            .get("cacheRetention")
            .map(|v| serde_json::from_value(v.clone()).unwrap()),
        headers: value.get("headers").and_then(Value::as_object).map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect()
        }),
        ..Default::default()
    }
}
fn options(value: &Value) -> AzureOpenAIResponsesOptions {
    AzureOpenAIResponsesOptions {
        base: base_options(value),
        reasoning_effort: value.get("reasoningEffort").and_then(thinking),
        reasoning_summary: value
            .get("reasoningSummary")
            .and_then(Value::as_str)
            .map(|value| match value {
                "detailed" => ReasoningSummary::Detailed,
                "concise" => ReasoningSummary::Concise,
                _ => ReasoningSummary::Auto,
            }),
        azure_api_version: value
            .get("azureApiVersion")
            .and_then(Value::as_str)
            .map(str::to_string),
        azure_resource_name: value
            .get("azureResourceName")
            .and_then(Value::as_str)
            .map(str::to_string),
        azure_base_url: value
            .get("azureBaseUrl")
            .and_then(Value::as_str)
            .map(str::to_string),
        azure_deployment_name: value
            .get("azureDeploymentName")
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
        Value::Number(n)
            if n.as_u64().is_none()
                && n.as_i64().is_none()
                && n.as_f64().is_some_and(|f| f.fract() == 0.0) =>
        {
            *value = json!(n.as_f64().unwrap() as i64)
        }
        Value::Array(a) => a.iter_mut().for_each(canonicalize),
        Value::Object(o) => o.values_mut().for_each(canonicalize),
        _ => {}
    }
}
async fn run(case: &Value, models: &Value, shared: &Value) -> Value {
    const ENV_KEYS: &[&str] = &[
        "AZURE_OPENAI_API_VERSION",
        "AZURE_OPENAI_BASE_URL",
        "AZURE_OPENAI_RESOURCE_NAME",
        "AZURE_OPENAI_DEPLOYMENT_NAME_MAP",
    ];
    let old_env = ENV_KEYS
        .iter()
        .map(|key| (*key, std::env::var(key).ok()))
        .collect::<Vec<_>>();
    // This differential binary owns these provider variables for each sequential case.
    unsafe {
        for key in ENV_KEYS {
            std::env::remove_var(key);
        }
        if let Some(env) = case.get("env").and_then(Value::as_object) {
            for (key, value) in env {
                std::env::set_var(key, value.as_str().unwrap());
            }
        }
    }
    let responses = case["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| response(v, shared))
        .collect();
    let (addr, captured) = serve(responses);
    let mut model = models[case["model"].as_str().unwrap()].clone();
    let simple = case.get("simple").and_then(Value::as_bool).unwrap_or(false);
    let no_server_base = case
        .get("noServerBase")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut opts = case["options"].clone();
    if !no_server_base {
        if simple {
            model["baseUrl"] = json!(format!("http://{addr}"));
        } else {
            opts["azureBaseUrl"] = json!(format!("http://{addr}"));
        }
    }
    let model: Model = serde_json::from_value(model).unwrap();
    let context: Context = serde_json::from_value(case["context"].clone()).unwrap();
    let stream = if simple {
        match stream_simple_azure_openai_responses(
            &model,
            &context,
            Some(SimpleStreamOptions {
                base: base_options(&opts),
                reasoning: opts.get("reasoning").and_then(thinking),
                thinking_budgets: None,
            }),
        ) {
            Ok(v) => v,
            Err(e) => return json!({"name":case["name"],"requests":[],"syncError":e.to_string()}),
        }
    } else {
        stream_azure_openai_responses(&model, &context, Some(options(&opts)))
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
        .map(|v| normalize_request(v))
        .collect::<Vec<_>>();
    unsafe {
        for (key, value) in old_env {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
    json!({"name":case["name"],"requests":requests,"events":events,"result":result})
}
#[tokio::test]
async fn pi_rs_matches_pi_azure_openai_responses_oracle() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/azure-openai-responses-parity");
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let oracle: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("oracle.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for (case, expected) in cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let mut actual = run(case, &cases["models"], &cases["sse"]).await;
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
