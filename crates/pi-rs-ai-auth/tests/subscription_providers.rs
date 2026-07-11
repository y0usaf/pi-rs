//! Deterministic auth-state/request replays for the two subscription providers
//! beyond Anthropic. Shapes are derived from the pinned Codex and Copilot OAuth
//! implementations under `ref/pi/packages/ai/src/utils/oauth/`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pi_rs_ai_auth::{
    AuthError, AuthFuture, GitHubCopilotEndpoints, GitHubCopilotFlow, OAuthAuthInfo,
    OAuthDeviceCodeInfo, OAuthLoginCallbacks, OAuthPrompt, OAuthProviderInterface,
    OAuthSelectPrompt, OpenAiCodexEndpoints, OpenAiCodexFlow, github_copilot_base_url,
    normalize_github_domain,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

#[derive(Debug)]
struct CapturedRequest {
    target: String,
    headers: String,
    body: String,
}

fn http_server(
    responses: Vec<(u16, String)>,
) -> (SocketAddr, mpsc::UnboundedReceiver<CapturedRequest>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            tx.send(request).unwrap();
            let reason = if status < 300 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    (addr, rx)
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        let count = socket.read(&mut chunk).await.unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..split]).into_owned();
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::to_owned)
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while bytes.len() < split + 4 + length {
            let count = socket.read(&mut chunk).await.unwrap();
            bytes.extend_from_slice(&chunk[..count]);
        }
        let target = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap()
            .to_owned();
        return CapturedRequest {
            target,
            headers,
            body: String::from_utf8_lossy(&bytes[split + 4..split + 4 + length]).into_owned(),
        };
    }
    panic!("incomplete request")
}

#[derive(Default)]
struct Callbacks {
    prompts: Mutex<VecDeque<String>>,
    selection: Mutex<Option<String>>,
    manual: Mutex<Option<String>>,
    auth: Mutex<Vec<OAuthAuthInfo>>,
    device_codes: Mutex<Vec<OAuthDeviceCodeInfo>>,
    progress: Mutex<Vec<String>>,
    model_ids: Vec<String>,
}

impl OAuthLoginCallbacks for Callbacks {
    fn on_auth(&self, info: OAuthAuthInfo) {
        self.auth.lock().unwrap().push(info);
    }

    fn on_device_code(&self, info: OAuthDeviceCodeInfo) {
        self.device_codes.lock().unwrap().push(info);
    }

    fn on_prompt(&self, _prompt: OAuthPrompt) -> AuthFuture<'_, String> {
        let value = self.prompts.lock().unwrap().pop_front();
        Box::pin(async move { value.ok_or(AuthError::Cancelled) })
    }

    fn on_select(&self, _prompt: OAuthSelectPrompt) -> AuthFuture<'_, Option<String>> {
        let value = self.selection.lock().unwrap().take();
        Box::pin(async move { Ok(value) })
    }

    fn on_progress(&self, message: &str) {
        self.progress.lock().unwrap().push(message.into());
    }

    fn provider_model_ids(&self, provider: &str) -> Vec<String> {
        assert_eq!(provider, "github-copilot");
        self.model_ids.clone()
    }

    fn on_manual_code_input(&self) -> Option<AuthFuture<'_, String>> {
        let value = self.manual.lock().unwrap().take()?;
        Some(Box::pin(async move { Ok(value) }))
    }
}

fn jwt(account_id: &str) -> String {
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
    });
    format!("x.{}.y", URL_SAFE_NO_PAD.encode(payload.to_string()))
}

fn form(body: &str) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn codex_device_login_replays_select_device_poll_exchange_and_account_id() {
    let access = jwt("acct-123");
    let responses = vec![
        (200, r#"{"device_auth_id":"dev-1","user_code":"ABCD","interval":"0"}"#.into()),
        (200, r#"{"authorization_code":"code-1","code_verifier":"verify-1"}"#.into()),
        (
            200,
            serde_json::json!({ "access_token": access, "refresh_token": "refresh-1", "expires_in": 3600 }).to_string(),
        ),
    ];
    let (addr, mut requests) = http_server(responses);
    let base = format!("http://{addr}");
    let flow = OpenAiCodexFlow {
        endpoints: OpenAiCodexEndpoints {
            authorize_url: format!("{base}/oauth/authorize"),
            token_url: format!("{base}/oauth/token"),
            device_user_code_url: format!("{base}/device/usercode"),
            device_token_url: format!("{base}/device/token"),
            device_verification_uri: format!("{base}/codex/device"),
            device_redirect_uri: format!("{base}/device/callback"),
            callback_port: 1455,
        },
    };
    let callbacks = Callbacks::default();
    *callbacks.selection.lock().unwrap() = Some("device_code".into());

    let credentials = flow.login(&callbacks).await.unwrap();
    assert_eq!(credentials.refresh, "refresh-1");
    assert_eq!(credentials.extra["accountId"], "acct-123");
    assert_eq!(callbacks.device_codes.lock().unwrap()[0].user_code, "ABCD");

    let first = requests.recv().await.unwrap();
    assert_eq!(first.target, "/device/usercode");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first.body).unwrap()["client_id"],
        "app_EMoamEEZ73f0CkXaXp7hrann"
    );
    let second = requests.recv().await.unwrap();
    assert_eq!(second.target, "/device/token");
    let third = requests.recv().await.unwrap();
    assert_eq!(third.target, "/oauth/token");
    let exchange = form(&third.body);
    assert_eq!(exchange["grant_type"], "authorization_code");
    assert_eq!(exchange["code"], "code-1");
    assert_eq!(exchange["code_verifier"], "verify-1");
    assert_eq!(exchange["redirect_uri"], format!("{base}/device/callback"));
}

#[tokio::test]
async fn codex_browser_login_replays_random_state_pkce_and_manual_exchange() {
    let access = jwt("acct-browser");
    let (addr, mut requests) = http_server(vec![(
        200,
        serde_json::json!({
            "access_token": access,
            "refresh_token": "refresh-browser",
            "expires_in": 3600
        })
        .to_string(),
    )]);
    let base = format!("http://{addr}");
    let callback_port = free_port();
    let flow = OpenAiCodexFlow {
        endpoints: OpenAiCodexEndpoints {
            authorize_url: format!("{base}/oauth/authorize"),
            token_url: format!("{base}/oauth/token"),
            device_user_code_url: format!("{base}/device/usercode"),
            device_token_url: format!("{base}/device/token"),
            device_verification_uri: format!("{base}/codex/device"),
            device_redirect_uri: format!("{base}/device/callback"),
            callback_port,
        },
    };
    let callbacks = Callbacks::default();
    *callbacks.selection.lock().unwrap() = Some("browser".into());
    *callbacks.manual.lock().unwrap() = Some("browser-code".into());

    let credentials = flow.login(&callbacks).await.unwrap();
    assert_eq!(credentials.extra["accountId"], "acct-browser");
    let auth_url = url::Url::parse(&callbacks.auth.lock().unwrap()[0].url).unwrap();
    let params: std::collections::HashMap<_, _> = auth_url.query_pairs().into_owned().collect();
    assert_eq!(params["originator"], "pi");
    assert_eq!(params["codex_cli_simplified_flow"], "true");
    assert_eq!(params["state"].len(), 32);
    assert_ne!(
        params["code_challenge"],
        pi_rs_ai_auth::challenge_for(&params["state"])
    );

    let request = requests.recv().await.unwrap();
    let exchange = form(&request.body);
    assert_eq!(exchange["code"], "browser-code");
    assert_eq!(
        exchange["redirect_uri"],
        format!("http://localhost:{callback_port}/auth/callback")
    );
    assert_ne!(exchange["code_verifier"], params["state"]);
}

#[tokio::test]
async fn github_login_replays_enterprise_prompt_device_poll_and_copilot_refresh() {
    let expires_at = pi_rs_ai_types::now_ms() / 1000 + 3600;
    let responses = vec![
        (
            200,
            r#"{"device_code":"dev-2","user_code":"WXYZ","verification_uri":"https://github.com/login/device","interval":1,"expires_in":900}"#.into(),
        ),
        (200, r#"{"access_token":"github-access"}"#.into()),
        (200, serde_json::json!({ "token": "tid=1;proxy-ep=proxy.individual.githubcopilot.com;", "expires_at": expires_at }).to_string()),
        (200, "{}".into()),
    ];
    let (addr, mut requests) = http_server(responses);
    let base = format!("http://{addr}");
    let flow = GitHubCopilotFlow {
        endpoints_override: Some(GitHubCopilotEndpoints {
            device_code_url: format!("{base}/login/device/code"),
            access_token_url: format!("{base}/login/oauth/access_token"),
            copilot_token_url: format!("{base}/copilot/token"),
        }),
        policy_base_url_override: Some(base.clone()),
    };
    let callbacks = Callbacks {
        model_ids: vec!["gpt-test".into()],
        ..Callbacks::default()
    };
    callbacks.prompts.lock().unwrap().push_back(String::new());

    let credentials = flow.login(&callbacks).await.unwrap();
    assert_eq!(credentials.refresh, "github-access");
    assert!(credentials.extra.get("enterpriseUrl").is_none());
    assert_eq!(callbacks.device_codes.lock().unwrap()[0].user_code, "WXYZ");
    assert_eq!(
        callbacks.progress.lock().unwrap().as_slice(),
        ["Enabling models..."]
    );

    let device = requests.recv().await.unwrap();
    assert_eq!(device.target, "/login/device/code");
    assert!(
        device
            .headers
            .to_ascii_lowercase()
            .contains("user-agent: githubcopilotchat/0.35.0")
    );
    assert_eq!(form(&device.body)["scope"], "read:user");
    let poll = requests.recv().await.unwrap();
    assert_eq!(form(&poll.body)["device_code"], "dev-2");
    let refresh = requests.recv().await.unwrap();
    assert_eq!(refresh.target, "/copilot/token");
    assert!(
        refresh
            .headers
            .contains("authorization: Bearer github-access")
    );
    let policy = requests.recv().await.unwrap();
    assert_eq!(policy.target, "/models/gpt-test/policy");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&policy.body).unwrap(),
        serde_json::json!({ "state": "enabled" })
    );
}

#[test]
fn github_domain_and_dynamic_base_url_match_the_spec() {
    assert_eq!(
        normalize_github_domain(" https://company.ghe.com/path ").as_deref(),
        Some("company.ghe.com")
    );
    assert_eq!(
        normalize_github_domain("company.ghe.com").as_deref(),
        Some("company.ghe.com")
    );
    assert_eq!(normalize_github_domain(""), None);
    assert_eq!(
        github_copilot_base_url(
            Some("tid=x;proxy-ep=proxy.business.githubcopilot.com;"),
            None
        ),
        "https://api.business.githubcopilot.com"
    );
    assert_eq!(
        github_copilot_base_url(None, Some("company.ghe.com")),
        "https://copilot-api.company.ghe.com"
    );
}
