//! Meta's device-code login: the identity token stays, and the Model API key it mints expires daily.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_coding_agent::login::{self, AuthType, Login, LoginRequest, UiEvent};
use pi_durable::ai::types::Model;

use super::{auth_json, check, drive, fake_server, keys};

const IDENTITY: &str = "meta-identity-token";
const KEY: &str = "meta-api-key-1";
const KEY_2: &str = "meta-api-key-2";

pub fn run(agent_dir: &str) {
    let mints = Arc::new(AtomicUsize::new(0));
    let counter = mints.clone();
    let base = fake_server(move |request, body| match request {
        "POST /oidc/device/authorization/ HTTP/1.1" => {
            let form = form(body);
            check(
                "device client id",
                form.get("client_id").map(String::as_str),
                Some("1031625952748946"),
            );
            (
                200,
                serde_json::json!({
                    "device_code": "meta-device",
                    "user_code": "META-1",
                    "verification_uri": "https://auth.meta.com/device",
                    "interval": 1,
                    "expires_in": 600
                })
                .to_string(),
            )
        }
        "POST /oidc/device/token/ HTTP/1.1" => {
            let form = form(body);
            check(
                "device code",
                form.get("device_code").map(String::as_str),
                Some("meta-device"),
            );
            (
                200,
                serde_json::json!({ "access_token": IDENTITY }).to_string(),
            )
        }
        "POST /muse-code/key HTTP/1.1" => {
            let call = counter.fetch_add(1, Ordering::SeqCst);
            let _ = body;
            let key = if call == 0 { KEY } else { KEY_2 };
            (200, serde_json::json!({ "api_key": key }).to_string())
        }
        _ => (404, "{}".to_owned()),
    });
    unsafe {
        std::env::set_var(
            "PI_OAUTH_META_DEVICE_AUTHORIZATION_URL",
            format!("{base}/oidc/device/authorization/"),
        );
        std::env::set_var(
            "PI_OAUTH_META_DEVICE_TOKEN_URL",
            format!("{base}/oidc/device/token/"),
        );
        std::env::set_var(
            "PI_OAUTH_META_API_KEY_MINT_URL",
            format!("{base}/muse-code/key"),
        );
    }

    let login = Login::start(
        LoginRequest {
            provider: "meta".to_owned(),
            auth: AuthType::Oauth,
        },
        agent_dir,
    );
    let status = drive(
        &login,
        |prompt, _| panic!("unexpected prompt {prompt:?}"),
        |event, _| {
            if let UiEvent::DeviceCode { user_code, .. } = event {
                check("user code", user_code.as_str(), "META-1");
            }
        },
    );
    status.expect("Meta login succeeds");
    let text = std::fs::read_to_string(login::auth_path(agent_dir)).expect("auth.json exists");
    let stored = auth_json(agent_dir);
    check(
        "key order",
        keys(&stored["meta"]),
        vec!["type", "refresh", "access", "expires"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    check(
        "refresh holds the identity token",
        stored["meta"]["refresh"].as_str(),
        Some(IDENTITY),
    );
    check(
        "access holds the key",
        stored["meta"]["access"].as_str(),
        Some(KEY),
    );
    let expires = stored["meta"]["expires"].as_f64().expect("expires is set");
    let day = 24.0 * 60.0 * 60.0 * 1000.0;
    let now = pi_durable::ai::oauth::now_ms() as f64;
    assert!(
        (expires - now - day).abs() < 60_000.0,
        "the key expires in about a day: {expires} vs {now}"
    );
    assert!(
        text.contains("\n  \"meta\""),
        "auth.json is two-space pretty"
    );

    check_request_token(agent_dir, KEY);
    expire(agent_dir);
    check_request_token(agent_dir, KEY_2);
    let after = auth_json(agent_dir);
    check(
        "reminted key",
        after["meta"]["access"].as_str(),
        Some(KEY_2),
    );
    check(
        "identity token survives",
        after["meta"]["refresh"].as_str(),
        Some(IDENTITY),
    );
    println!("  meta ok");
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
    stored["meta"]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
}

fn check_request_token(agent_dir: &str, want: &str) {
    let models = serde_json::json!({
        "providers": {"meta": {"baseUrl": "http://127.0.0.1:1"}}
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
        .find(|model| model.provider == "meta")
        .cloned()
        .expect("a catalog meta model");
    let auth = registry.resolve_auth(&model).expect("auth resolves");
    check("request token", auth.api_key.as_deref(), Some(want));
}
