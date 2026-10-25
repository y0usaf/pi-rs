//! Kimi For Coding's device-code login: the polled token, the stored credential and its refresh.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const ACCESS: &str = "kimi-access-1";
const REFRESH: &str = "kimi-refresh-1";
const ACCESS_2: &str = "kimi-access-2";

pub fn run(agent_dir: &str) {
    let token_calls = Arc::new(AtomicUsize::new(0));
    let counter = token_calls.clone();
    let base = fake_server(move |request, body| match request {
        "POST /api/oauth/device_authorization HTTP/1.1" => {
            let form = form(body);
            check(
                "device client id",
                form.get("client_id").map(String::as_str),
                Some("17e5f671-d194-4dfb-9706-5516cb48c098"),
            );
            (
                200,
                serde_json::json!({
                    "device_code": "kimi-device",
                    "user_code": "KIMI-1",
                    "verification_uri": "https://auth.kimi.com/device",
                    "verification_uri_complete": "https://auth.kimi.com/device?code=KIMI-1",
                    "interval": 1,
                    "expires_in": 600
                })
                .to_string(),
            )
        }
        "POST /api/oauth/token HTTP/1.1" => {
            let form = form(body);
            let call = counter.fetch_add(1, Ordering::SeqCst);
            match form.get("grant_type").map(String::as_str) {
                Some("urn:ietf:params:oauth:grant-type:device_code") => {
                    check(
                        "device code",
                        form.get("device_code").map(String::as_str),
                        Some("kimi-device"),
                    );
                    if call == 0 {
                        return (400, r#"{"error":"authorization_pending"}"#.to_owned());
                    }
                    (
                        200,
                        serde_json::json!({
                            "access_token": ACCESS,
                            "refresh_token": REFRESH,
                            "expires_in": 3600
                        })
                        .to_string(),
                    )
                }
                Some("refresh_token") => {
                    check(
                        "refresh token",
                        form.get("refresh_token").map(String::as_str),
                        Some(REFRESH),
                    );
                    (
                        200,
                        serde_json::json!({
                            "access_token": ACCESS_2,
                            "refresh_token": REFRESH,
                            "expires_in": 3600
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
        std::env::set_var("PI_OAUTH_KIMI_CODING_OAUTH_HOST", &base);
    }

    let login = Login::start(
        LoginRequest {
            provider: "kimi-coding".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| panic!("unexpected prompt {prompt:?}"),
        |event, _| {
            if let UiEvent::DeviceCode {
                user_code,
                verification_uri,
                ..
            } = event
            {
                check("user code", user_code.as_str(), "KIMI-1");
                check(
                    "verification uri",
                    verification_uri.as_str(),
                    "https://auth.kimi.com/device?code=KIMI-1",
                );
            }
        },
    );
    status.expect("Kimi device login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "key order",
        keys(&stored["kimi-coding"]),
        vec!["type", "access", "refresh", "expires"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check(
        "access",
        stored["kimi-coding"]["access"].as_str(),
        Some(ACCESS),
    );
    check(
        "refresh",
        stored["kimi-coding"]["refresh"].as_str(),
        Some(REFRESH),
    );
    assert!(
        stored["kimi-coding"]["expires"].as_f64().unwrap() > 0.0,
        "expires is set"
    );
    assert!(
        text.contains("\n  \"kimi-coding\""),
        "auth.json is two-space pretty"
    );

    check_request_token(agent_dir, ACCESS);
    expire(agent_dir);
    check_request_token(agent_dir, ACCESS_2);
    let after = auth_json(agent_dir);
    check(
        "refreshed access",
        after["kimi-coding"]["access"].as_str(),
        Some(ACCESS_2),
    );
    println!("  kimi ok");
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
    stored["kimi-coding"]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
}

fn check_request_token(agent_dir: &str, want: &str) {
    let models = serde_json::json!({
        "providers": {"kimi-coding": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "kimi-coding")
        .cloned()
        .expect("a catalog kimi-coding model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(want));
}
