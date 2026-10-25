//! xAI's device-code login: the fake server's answers, the credential it stores, and the refresh.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const ACCESS: &str = "xai-access-1";
const REFRESH: &str = "xai-refresh-1";
const ACCESS_2: &str = "xai-access-2";

pub fn run(agent_dir: &str) {
    let token_calls = Arc::new(AtomicUsize::new(0));
    let counter = token_calls.clone();
    let base = fake_server(move |request, body| match request {
        "POST /oauth2/device/code HTTP/1.1" => {
            let form = form(body);
            check(
                "device client id",
                form.get("client_id").map(String::as_str),
                Some("b1a00492-073a-47ea-816f-4c329264a828"),
            );
            check(
                "device scope",
                form.get("scope").map(String::as_str),
                Some("openid profile email offline_access grok-cli:access api:access"),
            );
            check(
                "device referrer",
                form.get("referrer").map(String::as_str),
                Some("pi"),
            );
            (
                200,
                serde_json::json!({
                    "device_code": "xai-device",
                    "user_code": "XAI-1",
                    "verification_uri": "https://x.ai/device",
                    "interval": 1,
                    "expires_in": 600
                })
                .to_string(),
            )
        }
        "POST /oauth2/token HTTP/1.1" => {
            let form = form(body);
            let call = counter.fetch_add(1, Ordering::SeqCst);
            match form.get("grant_type").map(String::as_str) {
                Some("urn:ietf:params:oauth:grant-type:device_code") => {
                    check(
                        "token client id",
                        form.get("client_id").map(String::as_str),
                        Some("b1a00492-073a-47ea-816f-4c329264a828"),
                    );
                    check(
                        "token device code",
                        form.get("device_code").map(String::as_str),
                        Some("xai-device"),
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
                        "refresh client id",
                        form.get("client_id").map(String::as_str),
                        Some("b1a00492-073a-47ea-816f-4c329264a828"),
                    );
                    check(
                        "refresh token",
                        form.get("refresh_token").map(String::as_str),
                        Some(REFRESH),
                    );
                    (
                        200,
                        serde_json::json!({ "access_token": ACCESS_2, "expires_in": 3600 })
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
            "PI_OAUTH_XAI_DEVICE_CODE_URL",
            format!("{base}/oauth2/device/code"),
        );
        std::env::set_var("PI_OAUTH_XAI_TOKEN_URL", format!("{base}/oauth2/token"));
    }

    let login = Login::start(
        LoginRequest {
            provider: "xai".to_owned(),
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
                expires_in_seconds,
                ..
            } = event
            {
                check("user code", user_code.as_str(), "XAI-1");
                check(
                    "verification uri",
                    verification_uri.as_str(),
                    "https://x.ai/device",
                );
                check("expires in", *expires_in_seconds, 600);
            }
        },
    );
    status.expect("xAI device login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "key order",
        keys(&stored["xai"]),
        vec!["type", "access", "refresh", "expires"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check("type", stored["xai"]["type"].as_str(), Some("oauth"));
    check("access", stored["xai"]["access"].as_str(), Some(ACCESS));
    check("refresh", stored["xai"]["refresh"].as_str(), Some(REFRESH));
    assert!(
        stored["xai"]["expires"].as_f64().unwrap() > 0.0,
        "expires is set"
    );
    assert!(
        text.contains("\n  \"xai\""),
        "auth.json is two-space pretty"
    );

    check_request_token(agent_dir, ACCESS);
    expire(agent_dir);
    check_request_token(agent_dir, ACCESS_2);
    let after = auth_json(agent_dir);
    check(
        "refreshed access",
        after["xai"]["access"].as_str(),
        Some(ACCESS_2),
    );
    check(
        "kept refresh",
        after["xai"]["refresh"].as_str(),
        Some(REFRESH),
    );
    println!("  xai ok");
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
    stored["xai"]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
}

/// What the next request to xAI carries.
fn check_request_token(agent_dir: &str, want: &str) {
    let models = serde_json::json!({
        "providers": {"xai": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "xai")
        .cloned()
        .expect("a catalog xai model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(want));
}
