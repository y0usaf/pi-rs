//! Differential Google Generative AI protocol replay (PLAN item 8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::protocols::google::{
    GoogleOptions, GoogleThinking, GoogleThinkingLevel, GoogleToolChoice, stream_google,
    stream_simple_google,
};
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai_types::{Context, Model, ThinkingBudgets, ThinkingLevel};
mod common;

use serde_json::{Value, json};


fn response(value: &Value) -> String {
    let (body, content_type) = if let Some(chunks) = value.get("chunks").and_then(Value::as_array) {
        (
            chunks
                .iter()
                .map(|chunk| format!("data: {}\n\n", serde_json::to_string(chunk).unwrap()))
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
const DROP: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "accept-encoding",
    "accept-language",
    "sec-fetch-mode",
    "user-agent",
];

fn thinking_level(value: &str) -> ThinkingLevel {
    match value {
        "minimal" => ThinkingLevel::Minimal,
        "low" => ThinkingLevel::Low,
        "medium" => ThinkingLevel::Medium,
        "high" => ThinkingLevel::High,
        "xhigh" => ThinkingLevel::XHigh,
        _ => ThinkingLevel::Max,
    }
}
fn base_options(value: &Value) -> StreamOptions {
    StreamOptions {
        api_key: value
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_string),
        max_tokens: value.get("maxTokens").and_then(Value::as_u64),
        temperature: value.get("temperature").and_then(Value::as_f64),
        headers: value.get("headers").and_then(Value::as_object).map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
                .collect()
        }),
        ..Default::default()
    }
}
fn options(value: &Value) -> GoogleOptions {
    let thinking = value.get("thinking").map(|thinking| GoogleThinking {
        enabled: thinking["enabled"].as_bool().unwrap_or(false),
        budget_tokens: thinking.get("budgetTokens").and_then(Value::as_i64),
        level: thinking
            .get("level")
            .and_then(Value::as_str)
            .map(|level| match level {
                "MINIMAL" => GoogleThinkingLevel::Minimal,
                "LOW" => GoogleThinkingLevel::Low,
                "MEDIUM" => GoogleThinkingLevel::Medium,
                "HIGH" => GoogleThinkingLevel::High,
                _ => GoogleThinkingLevel::Unspecified,
            }),
    });
    GoogleOptions {
        base: base_options(value),
        tool_choice: value
            .get("toolChoice")
            .and_then(Value::as_str)
            .map(|choice| match choice {
                "none" => GoogleToolChoice::None,
                "any" => GoogleToolChoice::Any,
                _ => GoogleToolChoice::Auto,
            }),
        thinking,
    }
}
fn budgets(value: &Value) -> Option<ThinkingBudgets> {
    value.get("thinkingBudgets").map(|value| ThinkingBudgets {
        minimal: value.get("minimal").and_then(Value::as_u64),
        low: value.get("low").and_then(Value::as_u64),
        medium: value.get("medium").and_then(Value::as_u64),
        high: value.get("high").and_then(Value::as_u64),
    })
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
    let (address, captured) = common::serve(responses);
    let mut model = models[case["model"].as_str().unwrap()].clone();
    if !case
        .get("noServerBase")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        model["baseUrl"] = json!(format!("http://{address}"));
    }
    let model: Model = serde_json::from_value(model).unwrap();
    let context: Context = serde_json::from_value(case["context"].clone()).unwrap();
    let simple = case.get("simple").and_then(Value::as_bool).unwrap_or(false);
    let stream = if simple {
        let values = &case["options"];
        match stream_simple_google(
            &model,
            &context,
            Some(SimpleStreamOptions {
                base: base_options(values),
                reasoning: values
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .map(thinking_level),
                thinking_budgets: budgets(values),
            }),
        ) {
            Ok(stream) => stream,
            Err(error) => {
                return json!({"name":case["name"],"requests":[],"syncError":error.to_string()});
            }
        }
    } else {
        stream_google(&model, &context, Some(options(&case["options"])))
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
        .map(|raw| common::normalize_drop(raw, DROP))
        .collect::<Vec<_>>();
    json!({"name":case["name"],"requests":requests,"events":events,"result":result})
}

#[tokio::test]
async fn pi_rs_matches_pi_google_generative_ai_oracle() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/google-generative-ai-parity");
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
