//! Differential Google Vertex protocol replay (PLAN item 8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine;
use pi_rs_ai::protocols::google::{GoogleThinking, GoogleThinkingLevel, GoogleToolChoice};
use pi_rs_ai::protocols::google_vertex::{
    GoogleVertexOptions, stream_google_vertex, stream_simple_google_vertex,
};
use pi_rs_ai::protocols::options::{SimpleStreamOptions, StreamOptions};
use pi_rs_ai_types::{Context, Model, ThinkingBudgets, ThinkingLevel};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Captured = Arc<Mutex<Vec<String>>>;

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
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            copy.lock().unwrap().push(request);
            let value = match path.as_str() {
                "/token" | "/oauth-token" => "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 68\r\nconnection: close\r\n\r\n{\"access_token\":\"adc-token\",\"token_type\":\"Bearer\",\"expires_in\":3600}".to_string(),
                "/subject-text" => {
                    let body = "url-subject-token";
                    format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
                }
                "/subject-json" => {
                    let body = r#"{"token":"url-json-token"}"#;
                    format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
                }
                "/sts" => {
                    let body = r#"{"access_token":"sts-token","issued_token_type":"urn:ietf:params:oauth:token-type:access_token","token_type":"Bearer","expires_in":3600}"#;
                    format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
                }
                "/impersonate" => {
                    let body = r#"{"accessToken":"adc-token","expireTime":"2099-01-01T00:00:00Z"}"#;
                    format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
                }
                "/project/123" => {
                    let body = r#"{"projectId":"p"}"#;
                    format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
                }
                _ => {
                    let Some(value) = responses.get(index).or_else(|| responses.last()) else {
                        return;
                    };
                    index += 1;
                    value.clone()
                }
            };
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
    "user-agent",
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
    let content_type = headers
        .get("content-type")
        .map(String::as_str)
        .unwrap_or("");
    let body = if body.is_empty() {
        Value::Null
    } else if path == "/oauth-token" {
        let form = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        let pieces = form["assertion"].split('.').collect::<Vec<_>>();
        let header: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(pieces[0])
                .unwrap(),
        )
        .unwrap();
        let mut payload: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(pieces[1])
                .unwrap(),
        )
        .unwrap();
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(pieces[2])
            .unwrap();
        let private_key = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/google-vertex-parity/service-account-key.pem"),
        )
        .unwrap();
        let key_der = base64::engine::general_purpose::STANDARD
            .decode(
                private_key
                    .lines()
                    .filter(|line| !line.starts_with("-----"))
                    .collect::<String>(),
            )
            .unwrap();
        let key = ring::signature::RsaKeyPair::from_pkcs8(&key_der).unwrap();
        let signature_valid = ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            key.public().as_ref(),
        )
        .verify(
            format!("{}.{}", pieces[0], pieces[1]).as_bytes(),
            &signature,
        )
        .is_ok();
        let issued = payload["iat"].as_u64().unwrap();
        payload["exp"] = json!(payload["exp"].as_u64().unwrap() - issued);
        payload["iat"] = json!(0);
        json!({
            "grant_type": form["grant_type"],
            "assertion": {
                "header": header,
                "payload": payload,
                "signatureBytes": signature.len(),
                "signatureValid": signature_valid,
            }
        })
    } else if content_type.starts_with("application/json") {
        serde_json::from_str(body).unwrap()
    } else {
        json!(body)
    };
    json!({"method":method,"path":path,"headers":headers,"body":body})
}

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
fn options(value: &Value) -> GoogleVertexOptions {
    GoogleVertexOptions {
        base: base_options(value),
        tool_choice: value
            .get("toolChoice")
            .and_then(Value::as_str)
            .map(|choice| match choice {
                "none" => GoogleToolChoice::None,
                "any" => GoogleToolChoice::Any,
                _ => GoogleToolChoice::Auto,
            }),
        thinking: value.get("thinking").map(|thinking| GoogleThinking {
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
        }),
        project: value
            .get("project")
            .and_then(Value::as_str)
            .map(str::to_string),
        location: value
            .get("location")
            .and_then(Value::as_str)
            .map(str::to_string),
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
    let (address, captured) = serve(responses);
    let mut model = models[case["model"].as_str().unwrap()].clone();
    if !case
        .get("noServerBase")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        model["baseUrl"] = json!(format!(
            "http://{address}{}",
            case.get("baseSuffix").and_then(Value::as_str).unwrap_or("")
        ));
    }
    let credential_path =
        std::env::temp_dir().join(format!("pi-vertex-adc-{}.json", std::process::id()));
    let subject_path =
        std::env::temp_dir().join(format!("pi-vertex-subject-{}", std::process::id()));
    let old_credentials = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS");
    if let Some(kind) = case.get("adc").and_then(Value::as_str) {
        let credentials = match kind {
            "authorized-user" => {
                json!({"type":"external_account_authorized_user","client_id":"client","client_secret":"secret","refresh_token":"refresh","token_url":format!("http://{address}/token")})
            }
            "service-account" => {
                let key = std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tests/google-vertex-parity/service-account-key.pem"),
                )
                .unwrap();
                json!({"type":"service_account","project_id":"p","client_email":"test@p.iam.gserviceaccount.com","private_key":key,"token_uri":format!("http://{address}/oauth-token")})
            }
            _ => {
                let uses_json = matches!(kind, "workload-json-impersonated" | "workload-url-json");
                let uses_url = matches!(kind, "workload-url-text" | "workload-url-json");
                if !uses_url {
                    std::fs::write(
                        &subject_path,
                        if uses_json {
                            r#"{"token":"subject-token"}"#
                        } else {
                            "subject-token"
                        },
                    )
                    .unwrap();
                }
                let mut source = if uses_url {
                    json!({
                        "url":format!("http://{address}/subject-{}", if uses_json { "json" } else { "text" }),
                        "headers":{"x-subject-header":"present"},
                    })
                } else {
                    json!({"file":subject_path})
                };
                if uses_json {
                    source["format"] = json!({"type":"json","subject_token_field_name":"token"});
                }
                let mut credentials = json!({
                    "type":"external_account",
                    "audience":"//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
                    "subject_token_type":"urn:ietf:params:oauth:token-type:jwt",
                    "token_url":format!("http://{address}/sts"),
                    "cloud_resource_manager_url":format!("http://{address}/project/"),
                    "credential_source":source,
                });
                if kind == "workload-json-impersonated" {
                    credentials["service_account_impersonation_url"] =
                        json!(format!("http://{address}/impersonate"));
                    credentials["service_account_impersonation"] =
                        json!({"token_lifetime_seconds":1800});
                }
                credentials
            }
        };
        std::fs::write(&credential_path, credentials.to_string()).unwrap();
        unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &credential_path) };
    }
    let model: Model = serde_json::from_value(model).unwrap();
    let context: Context = serde_json::from_value(case["context"].clone()).unwrap();
    let values = &case["options"];
    let vertex = options(values);
    let stream = if case.get("simple").and_then(Value::as_bool).unwrap_or(false) {
        stream_simple_google_vertex(
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
        )
        .unwrap()
    } else {
        stream_google_vertex(&model, &context, Some(vertex.clone()))
    };
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(summarize(&event));
    }
    let mut result = serde_json::to_value(stream.result().await.unwrap()).unwrap();
    result["timestamp"] = json!(0);
    if case.get("adc").and_then(Value::as_str).is_some() {
        if let Some(value) = old_credentials {
            unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", value) }
        } else {
            unsafe { std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS") }
        }

        let _ = std::fs::remove_file(credential_path);
        let _ = std::fs::remove_file(subject_path);
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
async fn pi_rs_matches_pi_google_vertex_oracle() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/google-vertex-parity");
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
