//! Radius's gateway sign-in: the device flow, the browser flow through its loopback callback, and
//! the refresh the gateway answers without discovery.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::oauth::interaction::Prompt;
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const ACCESS: &str = "radius-access-1";
const REFRESH: &str = "radius-refresh-1";
const ACCESS_2: &str = "radius-access-2";
const ACCESS_3: &str = "radius-access-3";
const SCOPE: &str = "gateway offline_access";
const BROWSER_CODE: &str = "radius-browser-code";

pub fn run(agent_dir: &str) {
    let token_calls = Arc::new(AtomicUsize::new(0));
    let counter = token_calls.clone();
    let base = fake_server(move |request, body| {
        match request {
        "GET /v1/oauth HTTP/1.1" => (
            200,
            serde_json::json!({ "authorizationEndpoint": "https://radius.example/authorize?drop=me" })
                .to_string(),
        ),
        "POST /v1/oauth/device HTTP/1.1" => {
            let form = form(body);
            check(
                "device client id",
                form.get("client_id").map(String::as_str),
                Some("pi-gateway"),
            );
            check(
                "device scope",
                form.get("scope").map(String::as_str),
                Some(SCOPE),
            );
            (
                200,
                serde_json::json!({
                    "device_code": "radius-device",
                    "user_code": "RAD-1",
                    "verification_uri": "https://radius.example/device",
                    "interval": 1,
                    "expires_in": 600
                })
                .to_string(),
            )
        }
        "POST /v1/oauth/token HTTP/1.1" => {
            let form = form(body);
            let call = counter.fetch_add(1, Ordering::SeqCst);
            check(
                "token client id",
                form.get("client_id").map(String::as_str),
                Some("pi-gateway"),
            );
            match form.get("grant_type").map(String::as_str) {
                Some("urn:ietf:params:oauth:grant-type:device_code") => {
                    check(
                        "token device code",
                        form.get("device_code").map(String::as_str),
                        Some("radius-device"),
                    );
                    if call == 0 {
                        return (
                            400,
                            r#"{"error":"authorization_pending","error_description":"wait"}"#
                                .to_owned(),
                        );
                    }
                    (
                        200,
                        serde_json::json!({
                            "access_token": ACCESS,
                            "refresh_token": REFRESH,
                            "expires_in": 3600,
                            "scope": SCOPE
                        })
                        .to_string(),
                    )
                }
                Some("authorization_code") => {
                    check(
                        "browser code",
                        form.get("code").map(String::as_str),
                        Some(BROWSER_CODE),
                    );
                    check(
                        "browser redirect",
                        form.get("redirect_uri").map(String::as_str),
                        Some("http://127.0.0.1:1456/oauth/callback"),
                    );
                    assert!(
                        form.get("code_verifier")
                            .is_some_and(|verifier| !verifier.is_empty()),
                        "a code verifier is sent"
                    );
                    (
                        200,
                        serde_json::json!({
                            "access_token": ACCESS_3,
                            "refresh_token": "radius-refresh-3",
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
                        serde_json::json!({ "access_token": ACCESS_2, "refresh_token": REFRESH, "expires_in": 3600 })
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
    }
    });
    unsafe {
        std::env::set_var("PI_OAUTH_RADIUS_GATEWAY", base.as_str());
    }

    device(agent_dir);
    browser(agent_dir);
    println!("  radius ok");
}

fn device(agent_dir: &str) {
    let login = Login::start(
        LoginRequest {
            provider: "radius".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| match prompt {
            Prompt::Select { message, options } => {
                check("method message", message.as_str(), "Sign in to Radius:");
                check("method count", options.len(), 2);
                check("first method", options[0].0.as_str(), "browser");
                check(
                    "first method label",
                    options[0].1.as_str(),
                    "Sign in with browser (recommended)",
                );
                check(
                    "second method label",
                    options[1].1.as_str(),
                    "Sign in with device code (when signing in from another device)",
                );
                Some("device-code".to_owned())
            }
            other => panic!("unexpected prompt {other:?}"),
        },
        |event, _| {
            if let UiEvent::DeviceCode {
                user_code,
                verification_uri,
                expires_in_seconds,
                ..
            } = event
            {
                check("user code", user_code.as_str(), "RAD-1");
                check(
                    "verification uri",
                    verification_uri.as_str(),
                    "https://radius.example/device",
                );
                check("expires in", *expires_in_seconds, 600);
            }
        },
    );
    status.expect("Radius device login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "device key order",
        keys(&stored["radius"]),
        ["type", "access", "refresh", "expires", "scope"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check(
        "device type",
        stored["radius"]["type"].as_str(),
        Some("oauth"),
    );
    check(
        "device access",
        stored["radius"]["access"].as_str(),
        Some(ACCESS),
    );
    check(
        "device refresh",
        stored["radius"]["refresh"].as_str(),
        Some(REFRESH),
    );
    check(
        "device scope",
        stored["radius"]["scope"].as_str(),
        Some(SCOPE),
    );
    assert!(
        stored["radius"]["expires"].as_f64().unwrap() > 0.0,
        "expires is set"
    );
    assert!(
        text.contains("\n  \"radius\""),
        "auth.json is two-space pretty"
    );

    check_request_token(agent_dir, ACCESS);
    expire(agent_dir);
    check_request_token(agent_dir, ACCESS_2);
    let after = auth_json(agent_dir);
    check(
        "refreshed access",
        after["radius"]["access"].as_str(),
        Some(ACCESS_2),
    );
    check(
        "refreshed key order",
        keys(&after["radius"]),
        vec!["type", "access", "refresh", "expires"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
}

fn browser(agent_dir: &str) {
    let login = Login::start(
        LoginRequest {
            provider: "radius".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| match prompt {
            Prompt::Select { .. } => Some("browser".to_owned()),
            other => panic!("unexpected prompt {other:?}"),
        },
        |event, state| {
            if let UiEvent::Progress(progress) = event {
                check(
                    "callback notice",
                    progress.as_str(),
                    "Listening for OAuth callback on http://127.0.0.1:1456/oauth/callback",
                );
            }
            if let UiEvent::AuthUrl { url, instructions } = event {
                check(
                    "instructions",
                    instructions.as_deref(),
                    Some("Continue in your browser."),
                );
                check(
                    "authorize url",
                    url.split_once('?').map(|(head, _)| head),
                    Some("https://radius.example/authorize"),
                );
                let query = url
                    .split_once('?')
                    .map(|(_, query)| query)
                    .unwrap_or_default();
                check(
                    "authorize params",
                    query
                        .split('&')
                        .filter_map(|pair| pair.split_once('='))
                        .map(|(key, _)| key)
                        .collect::<Vec<_>>(),
                    vec![
                        "response_type",
                        "client_id",
                        "redirect_uri",
                        "scope",
                        "code_challenge",
                        "code_challenge_method",
                        "handoff",
                        "state",
                    ],
                );
                check(
                    "client id",
                    pi_durable::ai::oauth::query_param(query, "client_id").as_deref(),
                    Some("pi-gateway"),
                );
                check(
                    "redirect uri",
                    pi_durable::ai::oauth::query_param(query, "redirect_uri").as_deref(),
                    Some("http://127.0.0.1:1456/oauth/callback"),
                );
                check(
                    "challenge method",
                    pi_durable::ai::oauth::query_param(query, "code_challenge_method").as_deref(),
                    Some("S256"),
                );
                let state = state.to_owned();
                std::thread::spawn(move || {
                    callback(&format!(
                        "/oauth/callback?code={BROWSER_CODE}&state={state}"
                    ));
                });
            }
        },
    );
    status.expect("Radius browser login succeeds");
    let stored = auth_json(agent_dir);
    check(
        "browser key order",
        keys(&stored["radius"]),
        vec!["type", "access", "refresh", "expires"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check(
        "browser access",
        stored["radius"]["access"].as_str(),
        Some(ACCESS_3),
    );
}

/// The browser's redirect, on the port upstream fixes the callback to.
fn callback(path: &str) {
    let mut stream = TcpStream::connect("127.0.0.1:1456").expect("callback server listens");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nhost: 127.0.0.1:1456\r\nconnection: close\r\n\r\n"
    )
    .expect("write callback request");
    let mut reply = String::new();
    stream
        .read_to_string(&mut reply)
        .expect("read callback reply");
    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "callback page: {}",
        reply.lines().next().unwrap_or_default()
    );
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
    stored["radius"]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
}

/// What the next request to Radius carries.
fn check_request_token(agent_dir: &str, want: &str) {
    let models = serde_json::json!({
        "providers": {"radius": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "radius")
        .cloned()
        .expect("a catalog radius model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(want));
}
