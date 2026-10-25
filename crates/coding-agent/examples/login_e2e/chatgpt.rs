//! OpenAI's ChatGPT subscription login: a fresh client id comes back through the callback, the
//! device id comes from settings, and the credential carries both the client id and the scopes.

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::oauth::interaction::Prompt;
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const CODE: &str = "openai-code";
const ISSUED: &str = "client-issued-1";
const ACCESS: &str = "openai-access-1";
const REFRESH: &str = "openai-refresh-1";
const ACCESS_2: &str = "openai-access-2";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

pub fn run(agent_dir: &str) {
    let base = fake_server(|request, body| match request {
        "POST /api/accounts/oauth/token HTTP/1.1" => {
            let form = form(body);
            match form.get("grant_type").map(String::as_str) {
                Some("authorization_code") => {
                    check("code", form.get("code").map(String::as_str), Some(CODE));
                    check(
                        "issued client id",
                        form.get("client_id").map(String::as_str),
                        Some(ISSUED),
                    );
                    (
                        200,
                        serde_json::json!({
                            "access_token": ACCESS,
                            "refresh_token": REFRESH,
                            "expires_in": 3600,
                            "id_token": "demo-id-token",
                            "scope": SCOPE
                        })
                        .to_string(),
                    )
                }
                Some("refresh_token") => {
                    check(
                        "refresh client id",
                        form.get("client_id").map(String::as_str),
                        Some(ISSUED),
                    );
                    (
                        200,
                        serde_json::json!({
                            "access_token": ACCESS_2,
                            "refresh_token": REFRESH,
                            "expires_in": 3600,
                            "scope": SCOPE
                        })
                        .to_string(),
                    )
                }
                other => (
                    400,
                    serde_json::json!({ "error": format!("unexpected grant {other:?}") })
                        .to_string(),
                ),
            }
        }
        _ => (404, "{}".to_owned()),
    });
    unsafe {
        std::env::set_var(
            "PI_OAUTH_OPENAI_AUTHORIZE_URL",
            format!("{base}/api/accounts/authorize"),
        );
        std::env::set_var(
            "PI_OAUTH_OPENAI_TOKEN_URL",
            format!("{base}/api/accounts/oauth/token"),
        );
    }

    let login = Login::start(
        LoginRequest {
            provider: "openai".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let arrival: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>> =
        Default::default();
    let arrival_taken = arrival.clone();
    let status = drive(
        &login,
        move |prompt, _| match prompt {
            Prompt::ManualCode { .. } => {
                if let Some(arrived) = arrival_taken.lock().unwrap().take() {
                    let _ = arrived.recv();
                }
                None
            }
            other => panic!("unexpected prompt {other:?}"),
        },
        |event, _| {
            if let UiEvent::AuthUrl { url, .. } = event {
                let query = url.split_once('?').map(|(_, q)| q).unwrap_or_default();
                let state = pi_durable::ai::oauth::query_param(query, "state")
                    .expect("the authorize URL carries a state");
                let host_id = pi_durable::ai::oauth::query_param(query, "ext_agent_host_id")
                    .expect("the authorize URL carries the agent host id");
                assert!(
                    host_id.starts_with("urn:uuid:"),
                    "the device id travels as a urn:uuid: {host_id}"
                );
                let (arrived, arrived_rx) = std::sync::mpsc::channel();
                *arrival.lock().unwrap() = Some(arrived_rx);
                std::thread::spawn(move || {
                    browser(&state);
                    let _ = arrived.send(());
                });
            }
        },
    );
    status.expect("ChatGPT login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "key order",
        keys(&stored["openai"]),
        vec!["type", "access", "refresh", "expires", "clientId", "scopes"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check("access", stored["openai"]["access"].as_str(), Some(ACCESS));
    check(
        "refresh",
        stored["openai"]["refresh"].as_str(),
        Some(REFRESH),
    );
    check(
        "client id",
        stored["openai"]["clientId"].as_str(),
        Some(ISSUED),
    );
    check(
        "scopes",
        stored["openai"]["scopes"]
            .as_array()
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
        SCOPE.to_owned(),
    );
    assert!(
        text.contains("\n  \"openai\""),
        "auth.json is two-space pretty"
    );

    let settings = std::fs::read_to_string(std::path::Path::new(agent_dir).join("settings.json"))
        .expect("the login writes settings.json");
    let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
    let device_id = settings["deviceId"].as_str().expect("a device id is kept");
    check("device id is a uuid", device_id.len(), 36);

    check_request_token(agent_dir, ACCESS);
    expire(agent_dir);
    check_request_token(agent_dir, ACCESS_2);
    let after = auth_json(agent_dir);
    check(
        "refreshed access",
        after["openai"]["access"].as_str(),
        Some(ACCESS_2),
    );
    println!("  openai ok");
}

fn form(body: &str) -> std::collections::BTreeMap<String, String> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (
                pi_durable::ai::oauth::form_decode(key),
                pi_durable::ai::oauth::form_decode(value),
            )
        })
        .collect()
}

fn expire(agent_dir: &str) {
    let path = login::auth_path(agent_dir);
    let mut stored = auth_json(agent_dir);
    stored["openai"]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
}

/// The browser's side: the redirect on the fixed loopback port, with the code and the issued client
/// id OpenAI adds.
fn browser(state: &str) {
    use std::io::{Read, Write};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match std::net::TcpStream::connect("127.0.0.1:1455") {
            Ok(mut stream) => {
                let path = format!("/auth/callback?code={CODE}&state={state}&client_id={ISSUED}");
                write!(
                    stream,
                    "GET {path} HTTP/1.1\r\nhost: 127.0.0.1:1455\r\nconnection: close\r\n\r\n"
                )
                .expect("the redirect is sent");
                let mut answer = String::new();
                stream
                    .read_to_string(&mut answer)
                    .expect("the page comes back");
                assert!(
                    answer.starts_with("HTTP/1.1 200") || answer.starts_with("HTTP/1.1 404"),
                    "callback page: {answer}"
                );
                return;
            }
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) => panic!("the callback server is not listening: {error}"),
        }
    }
}

fn check_request_token(agent_dir: &str, want: &str) {
    let models = serde_json::json!({
        "providers": {"openai": {"baseUrl": "http://127.0.0.1:1"}}
    });
    std::fs::write(
        std::path::Path::new(agent_dir).join("models.json"),
        models.to_string(),
    )
    .unwrap();
    let registry = pi_coding_agent::models::ModelRegistry::load(agent_dir);
    let model: Model = registry
        .models()
        .iter()
        .find(|model| model.provider == "openai")
        .cloned()
        .expect("a catalog openai model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(want));
}
