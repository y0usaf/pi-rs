//! GitHub Copilot's login: GitHub's device code, the Copilot token it exchanges for, and the model
//! catalogue the credential carries.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::oauth::interaction::Prompt;
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const GITHUB_TOKEN: &str = "gho-login-token";
const COPILOT_TOKEN: &str = "copilot-access-1";
const COPILOT_TOKEN_2: &str = "copilot-access-2";

pub fn run(agent_dir: &str) {
    let tokens = Arc::new(AtomicUsize::new(0));
    let counter = tokens.clone();
    let base = fake_server(move |request, body| {
        match request {
        "POST /login/device/code HTTP/1.1" => {
            let form = form(body);
            check(
                "device client id",
                form.get("client_id").map(String::as_str),
                Some("Iv1.b507a08c87ecfe98"),
            );
            check("device scope", form.get("scope").map(String::as_str), Some("read:user"));
            (
                200,
                serde_json::json!({
                    "device_code": "gh-device",
                    "user_code": "GH-1",
                    "verification_uri": "https://github.com/login/device",
                    "expires_in": 600,
                    "interval": 1
                })
                .to_string(),
            )
        }
        "POST /login/oauth/access_token HTTP/1.1" => {
            let form = form(body);
            check(
                "device code",
                form.get("device_code").map(String::as_str),
                Some("gh-device"),
            );
            (
                200,
                serde_json::json!({ "access_token": GITHUB_TOKEN }).to_string(),
            )
        }
        "GET /copilot_internal/v2/token HTTP/1.1" => {
            let call = counter.fetch_add(1, Ordering::SeqCst);
            let token = if call == 0 { COPILOT_TOKEN } else { COPILOT_TOKEN_2 };
            let expires_at = (pi_durable::ai::oauth::now_ms() / 1000) + 3600;
            (
                200,
                serde_json::json!({ "token": token, "expires_at": expires_at }).to_string(),
            )
        }
        "GET /models HTTP/1.1" => (
            200,
            serde_json::json!({
                "data": [
                    { "id": "gpt-5", "model_picker_enabled": true, "policy": { "state": "enabled" } },
                    { "id": "claude-sonnet", "model_picker_enabled": true }
                ]
            })
            .to_string(),
        ),
        _ => (404, "{}".to_owned()),
    }
    });
    unsafe {
        std::env::set_var(
            "PI_OAUTH_GITHUB_COPILOT_DEVICE_CODE_URL",
            format!("{base}/login/device/code"),
        );
        std::env::set_var(
            "PI_OAUTH_GITHUB_COPILOT_ACCESS_TOKEN_URL",
            format!("{base}/login/oauth/access_token"),
        );
        std::env::set_var(
            "PI_OAUTH_GITHUB_COPILOT_COPILOT_TOKEN_URL",
            format!("{base}/copilot_internal/v2/token"),
        );
        std::env::set_var("PI_OAUTH_GITHUB_COPILOT_API_BASE_URL", &base);
    }

    let login = Login::start(
        LoginRequest {
            provider: "github-copilot".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| match prompt {
            Prompt::Text { message } => {
                check(
                    "enterprise prompt",
                    message.as_str(),
                    "GitHub Enterprise URL/domain (blank for github.com)",
                );
                Some(String::new())
            }
            other => panic!("unexpected prompt {other:?}"),
        },
        |event, _| {
            if let UiEvent::DeviceCode { user_code, .. } = event {
                check("user code", user_code.as_str(), "GH-1");
            }
        },
    );
    status.expect("Copilot login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "key order",
        keys(&stored["github-copilot"]),
        vec!["type", "refresh", "access", "expires", "availableModelIds"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check(
        "refresh holds the GitHub token",
        stored["github-copilot"]["refresh"].as_str(),
        Some(GITHUB_TOKEN),
    );
    check(
        "access holds the Copilot token",
        stored["github-copilot"]["access"].as_str(),
        Some(COPILOT_TOKEN),
    );
    check(
        "model ids",
        stored["github-copilot"]["availableModelIds"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default(),
        "gpt-5,claude-sonnet".to_owned(),
    );
    assert!(
        text.contains("\n  \"github-copilot\""),
        "auth.json is two-space pretty"
    );

    check_request_token(agent_dir, COPILOT_TOKEN);
    expire(agent_dir);
    check_request_token(agent_dir, COPILOT_TOKEN_2);
    let after = auth_json(agent_dir);
    check(
        "refreshed token",
        after["github-copilot"]["access"].as_str(),
        Some(COPILOT_TOKEN_2),
    );
    println!("  github-copilot ok");
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
    stored["github-copilot"]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
}

fn check_request_token(agent_dir: &str, want: &str) {
    let models = serde_json::json!({
        "providers": {"github-copilot": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "github-copilot")
        .cloned()
        .expect("a catalog github-copilot model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(want));
}
