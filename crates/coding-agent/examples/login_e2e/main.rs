//! `/login`, `/logout` and the auth.json they write, driven the way the screen drives them: the
//! same channel and answers, against a fake OAuth server on a real socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use pi_coding_agent::auth::Credential;
use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::oauth::interaction::Prompt;
use pi_durable::ai::types::Model;

mod chatgpt;
mod copilot;
mod kimi;
mod meta;
mod openrouter;
mod radius;
mod xai;

const ACCESS: &str = "sk-ant-oat01-login-access";
const REFRESH: &str = "sk-ant-ort01-login-refresh";
const CODE: &str = "login-code";
fn device_access_token() -> String {
    let payload = r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct1"}}"#;
    format!(
        "h.{}.s",
        pi_durable::ai::oauth::pkce::base64url(payload.as_bytes())
    )
}

pub fn check<T: PartialEq + std::fmt::Debug>(label: &str, got: T, want: T) {
    assert!(got == want, "{label}: got {got:?}, want {want:?}");
}

fn main() {
    let dir = std::env::temp_dir().join(format!("pi-login-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let agent_dir = dir.to_string_lossy().into_owned();

    let oauth = TcpListener::bind("127.0.0.1:0").expect("fake oauth binds");
    let oauth_port = oauth.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{oauth_port}");
    unsafe {
        std::env::set_var(
            "PI_OAUTH_ANTHROPIC_AUTHORIZE_URL",
            format!("{base}/authorize"),
        );
        std::env::set_var("PI_OAUTH_ANTHROPIC_TOKEN_URL", format!("{base}/token"));
        std::env::set_var("PI_OAUTH_OPENAI_CODEX_TOKEN_URL", format!("{base}/token"));
        std::env::set_var(
            "PI_OAUTH_OPENAI_CODEX_DEVICE_USER_CODE_URL",
            format!("{base}/device/usercode"),
        );
        std::env::set_var(
            "PI_OAUTH_OPENAI_CODEX_DEVICE_TOKEN_URL",
            format!("{base}/device/token"),
        );
    }
    thread::spawn(move || serve_oauth(oauth));

    anthropic_copy_code(&agent_dir);
    api_key(&agent_dir);
    logout(&agent_dir);
    codex_device(&agent_dir);
    cancel(&agent_dir);
    chatgpt::run(&agent_dir);
    copilot::run(&agent_dir);
    kimi::run(&agent_dir);
    meta::run(&agent_dir);
    openrouter::run(&agent_dir);
    radius::run(&agent_dir);
    xai::run(&agent_dir);
    catalog_api_key(&agent_dir);

    let _ = std::fs::remove_dir_all(&dir);
    println!(
        "login ok: anthropic copy code, api key, logout, codex device code, cancel, chatgpt, copilot, kimi, meta, openrouter, radius, xai, deepseek api key"
    );
}

fn catalog_api_key(agent_dir: &str) {
    let found: Vec<(&str, AuthType)> = login::find("DeepSeek")
        .into_iter()
        .map(|provider| (provider.id, provider.auth))
        .collect();
    check(
        "deepseek options",
        found,
        vec![("deepseek", AuthType::ApiKey)],
    );
    let login = Login::start(
        LoginRequest {
            provider: "deepseek".to_owned(),
            auth: AuthType::ApiKey,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| match prompt {
            Prompt::Secret { message } => {
                check("key prompt", message.as_str(), "Enter DeepSeek API key");
                Some("sk-deepseek-typed".to_owned())
            }
            other => panic!("unexpected prompt {other:?}"),
        },
        |_, _| {},
    );
    status.expect("deepseek api key login succeeds");
    let stored = auth_json(agent_dir);
    check("type", stored["deepseek"]["type"].as_str(), Some("api_key"));
    check(
        "key",
        stored["deepseek"]["key"].as_str(),
        Some("sk-deepseek-typed"),
    );
}

/// A stored credential's keys in the order auth.json holds them.
pub fn keys(value: &serde_json::Value) -> Vec<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

pub fn auth_json(agent_dir: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    serde_json::from_str(&text).expect("auth.json parses")
}

fn anthropic_copy_code(agent_dir: &str) {
    let login = Login::start(
        LoginRequest {
            provider: "anthropic".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, state| match prompt {
            Prompt::Select { options, .. } => {
                check("method options", options.len(), 2);
                Some("copy_code".to_owned())
            }
            Prompt::ManualCode { .. } => Some(format!("{CODE}#{state}")),
            other => panic!("unexpected prompt {other:?}"),
        },
        |_, _| {},
    );
    let outcome = status.expect("anthropic login succeeds");
    check("provider", outcome.provider.as_str(), "anthropic");
    check("auth", outcome.auth, AuthType::Oauth);
    let stored = auth_json(agent_dir);
    check("type", stored["anthropic"]["type"].as_str(), Some("oauth"));
    check(
        "access",
        stored["anthropic"]["access"].as_str(),
        Some(ACCESS),
    );
    check(
        "refresh",
        stored["anthropic"]["refresh"].as_str(),
        Some(REFRESH),
    );
    assert!(
        stored["anthropic"]["expires"].as_f64().unwrap() > 0.0,
        "expires is set"
    );
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).unwrap();
    assert!(
        text.contains("\n  \"anthropic\""),
        "auth.json is two-space pretty"
    );

    // The stored token is what a request carries.
    let models = serde_json::json!({
        "providers": {"anthropic": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "anthropic")
        .cloned()
        .expect("a catalog anthropic model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(ACCESS));
}

fn api_key(agent_dir: &str) {
    let login = Login::start(
        LoginRequest {
            provider: "anthropic".to_owned(),
            auth: AuthType::ApiKey,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| match prompt {
            Prompt::Secret { message } => {
                check("key prompt", message.as_str(), "Enter Anthropic API key");
                Some("sk-ant-api03-typed".to_owned())
            }
            other => panic!("unexpected prompt {other:?}"),
        },
        |_, _| {},
    );
    status.expect("api key login succeeds");
    let stored = auth_json(agent_dir);
    check(
        "type",
        stored["anthropic"]["type"].as_str(),
        Some("api_key"),
    );
    check(
        "key",
        stored["anthropic"]["key"].as_str(),
        Some("sk-ant-api03-typed"),
    );
    let store = pi_coding_agent::auth::AuthStore::open(&login::auth_path(agent_dir));
    check(
        "stored credential",
        store.read("anthropic"),
        Some(Credential::ApiKey {
            key: Some("sk-ant-api03-typed".to_owned()),
            env: None,
        }),
    );
}

fn logout(agent_dir: &str) {
    login::logout(agent_dir, "anthropic").expect("logout removes the credential");
    let stored = auth_json(agent_dir);
    check("entry gone", stored.get("anthropic"), None);
}

fn codex_device(agent_dir: &str) {
    let login = Login::start(
        LoginRequest {
            provider: "openai-codex".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| match prompt {
            Prompt::Select { .. } => Some("device_code".to_owned()),
            other => panic!("unexpected prompt {other:?}"),
        },
        |event, _| {
            if let UiEvent::DeviceCode {
                user_code,
                verification_uri,
                ..
            } = event
            {
                check("user code", user_code.as_str(), "DEV-1");
                check(
                    "verification uri",
                    verification_uri.as_str(),
                    "https://auth.openai.com/codex/device",
                );
            }
        },
    );
    status.expect("device code login succeeds");
    let stored = auth_json(agent_dir);
    check(
        "codex access",
        stored["openai-codex"]["access"].as_str(),
        Some(device_access_token().as_str()),
    );
    check(
        "codex account",
        stored["openai-codex"]["accountId"].as_str(),
        Some("acct1"),
    );
}

fn cancel(agent_dir: &str) {
    let login = Login::start(
        LoginRequest {
            provider: "anthropic".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    loop {
        match login.events.recv().expect("an event") {
            UiEvent::Prompt { prompt, .. } => {
                if matches!(prompt, Prompt::Select { .. }) {
                    login.cancel();
                }
            }
            UiEvent::Dismiss(_) | UiEvent::AuthUrl { .. } | UiEvent::Progress(_) => {}
            UiEvent::DeviceCode { .. } => panic!("no device code for anthropic"),
            UiEvent::Done(status) => {
                let message = status.expect_err("a cancelled login fails");
                check("cancel message", message.as_str(), "Login cancelled");
                return;
            }
        }
    }
}

/// Answers prompts as the screen would, and hands every other event to `observe` together with the
/// state the authorize URL carried, which a pasted code has to name back.
pub fn drive(
    login: &Login,
    mut answer: impl FnMut(Prompt, &str) -> Option<String>,
    mut observe: impl FnMut(&UiEvent, &str),
) -> Result<login::Outcome, String> {
    let mut state = String::new();
    loop {
        let event = login.events.recv().expect("an event");
        match &event {
            UiEvent::Prompt { id, prompt } => {
                login.answer(*id, answer(prompt.clone(), &state));
            }
            UiEvent::AuthUrl { url, .. } => {
                let query = url
                    .split_once('?')
                    .map(|(_, query)| query)
                    .unwrap_or_default();
                state = pi_durable::ai::oauth::query_param(query, "state").unwrap_or_default();
                observe(&event, &state);
            }
            UiEvent::DeviceCode { .. } | UiEvent::Progress(_) | UiEvent::Dismiss(_) => {
                observe(&event, &state)
            }
            UiEvent::Done(status) => return status.clone(),
        }
    }
}

/// Serves fake answers on a loopback port of its own until the process ends, and answers the base
/// URL requests go to.
pub fn fake_server(handler: impl Fn(&str, &str) -> (u16, String) + Send + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake server binds");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (head, body) = read_request(&mut stream);
            let request = head.lines().next().unwrap_or_default().to_owned();
            let (status, reply) = handler(&request, &body);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    base
}

fn serve_oauth(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { return };
        let (head, body) = read_request(&mut stream);
        let request = head.lines().next().unwrap_or_default().to_owned();
        let (status, reply) = match request.as_str() {
            "POST /token HTTP/1.1" => token_reply(&body),
            "POST /device/usercode HTTP/1.1" => (
                "200 OK",
                serde_json::json!({
                    "device_auth_id": "dev-1",
                    "user_code": "DEV-1",
                    "interval": "1"
                })
                .to_string(),
            ),
            "POST /device/token HTTP/1.1" => {
                let device: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                if device["device_auth_id"] == "dev-1" {
                    (
                        "200 OK",
                        serde_json::json!({
                            "authorization_code": "device-code",
                            "code_verifier": "device-verifier"
                        })
                        .to_string(),
                    )
                } else {
                    ("403 Forbidden", "{}".to_owned())
                }
            }
            _ => ("404 Not Found", "{}".to_owned()),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
    }
}

fn token_reply(body: &str) -> (&'static str, String) {
    // Anthropic posts JSON, OpenAI Codex a form; the fields read the same way.
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => {
            let mut map = serde_json::Map::new();
            for pair in body.split('&').filter(|pair| !pair.is_empty()) {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                map.insert(
                    key.to_owned(),
                    serde_json::Value::String(pi_durable::ai::oauth::form_decode(value)),
                );
            }
            serde_json::Value::Object(map)
        }
    };
    let field = |name: &str| value[name].as_str().map(str::to_owned).unwrap_or_default();
    let invalid = ("400 Bad Request", r#"{"error":"invalid_grant"}"#.to_owned());
    match field("grant_type").as_str() {
        "authorization_code" => {
            let redirect = field("redirect_uri");
            if redirect.contains("/deviceauth/callback") {
                if field("code") != "device-code" {
                    return invalid;
                }
                return (
                    "200 OK",
                    serde_json::json!({
                        "access_token": device_access_token(),
                        "refresh_token": "codex-refresh",
                        "expires_in": 3600
                    })
                    .to_string(),
                );
            }
            if !redirect.contains("oauth/code/callback")
                && !redirect.starts_with("http://localhost")
            {
                return invalid;
            }
            if field("code") != CODE || field("state") != field("code_verifier") {
                return invalid;
            }
            (
                "200 OK",
                serde_json::json!({
                    "access_token": ACCESS,
                    "refresh_token": REFRESH,
                    "expires_in": 3600,
                    "token_type": "bearer"
                })
                .to_string(),
            )
        }
        _ => (
            "400 Bad Request",
            r#"{"error":"unsupported_grant_type"}"#.to_owned(),
        ),
    }
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(position) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk).expect("read request");
        assert!(read > 0, "client closed before the head ended");
        buf.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().expect("content-length"))
        })
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let read = stream.read(&mut chunk).expect("read body");
        assert!(read > 0, "client closed before the body ended");
        buf.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..head_end + length]).into_owned();
    (head, body)
}
