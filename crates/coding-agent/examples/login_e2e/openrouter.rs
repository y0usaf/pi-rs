//! OpenRouter's login: the browser's redirect reaches the flow's own loopback server, and the key it
//! hands out is permanent.

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::oauth::interaction::Prompt;
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const KEY: &str = "sk-or-v1-login-key";
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub fn run(agent_dir: &str) {
    let base = fake_server(|request, body| match request {
        "POST /api/v1/auth/keys HTTP/1.1" => {
            let sent: serde_json::Value =
                serde_json::from_str(body).expect("the key exchange posts JSON");
            check("code", sent["code"].as_str(), Some("or-code"));
            check(
                "challenge method",
                sent["code_challenge_method"].as_str(),
                Some("S256"),
            );
            assert!(
                sent["code_verifier"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty()),
                "a code verifier is sent"
            );
            (200, serde_json::json!({ "key": KEY }).to_string())
        }
        _ => (404, "{}".to_owned()),
    });
    unsafe {
        std::env::set_var("PI_OAUTH_OPENROUTER_AUTHORIZE_URL", format!("{base}/auth"));
        std::env::set_var(
            "PI_OAUTH_OPENROUTER_TOKEN_URL",
            format!("{base}/api/v1/auth/keys"),
        );
    }

    let login = Login::start(
        LoginRequest {
            provider: "openrouter".to_owned(),
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
                let callback = pi_durable::ai::oauth::query_param(query, "callback_url")
                    .expect("the authorize URL carries the loopback callback");
                assert!(
                    callback.starts_with("http://127.0.0.1:"),
                    "loopback: {callback}"
                );
                assert!(
                    pi_durable::ai::oauth::query_param(query, "code_challenge_method").as_deref()
                        == Some("S256"),
                    "PKCE is S256"
                );
                let (arrived, arrived_rx) = std::sync::mpsc::channel();
                *arrival.lock().unwrap() = Some(arrived_rx);
                std::thread::spawn(move || {
                    browser(&callback);
                    let _ = arrived.send(());
                });
            }
        },
    );
    status.expect("OpenRouter login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "key order",
        keys(&stored["openrouter"]),
        vec!["type", "access", "refresh", "expires"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check("access", stored["openrouter"]["access"].as_str(), Some(KEY));
    check(
        "refresh",
        stored["openrouter"]["refresh"].as_str(),
        Some(""),
    );
    check(
        "expires",
        stored["openrouter"]["expires"].as_i64(),
        Some(MAX_SAFE_INTEGER),
    );
    assert!(
        text.contains("\n  \"openrouter\""),
        "auth.json is two-space pretty"
    );

    let models = serde_json::json!({
        "providers": {"openrouter": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "openrouter")
        .cloned()
        .expect("a catalog openrouter model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(KEY));
    println!("  openrouter ok");
}

/// The browser's side: a GET on the loopback redirect with the authorization code.
fn browser(callback: &str) {
    use std::io::{Read, Write};
    let rest = callback.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').expect("a host and a path");
    let mut stream = std::net::TcpStream::connect(host).expect("loopback accepts");
    let path = format!("/{path}?code=or-code");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\n\r\n"
    )
    .expect("the redirect is sent");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .expect("the page comes back");
    assert!(
        answer.starts_with("HTTP/1.1 200"),
        "signed in page: {answer}"
    );
}
