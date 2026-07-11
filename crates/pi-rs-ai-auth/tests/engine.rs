//! Behavioral parity tests for the PKCE engine (spec: `loginAnthropic`
//! / `refreshAnthropicToken` / `exchangeAuthorizationCode` in
//! `utils/oauth/anthropic.ts`) over loopback servers: a real callback
//! redirect and a raw-TCP token endpoint.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Mutex;

use pi_rs_ai_auth::{
    AuthError, AuthFuture, CallbackPages, OAuthAuthInfo, OAuthDeviceCodeInfo, OAuthLoginCallbacks,
    OAuthPrompt, OAuthSelectPrompt, PkceFlow, login_pkce, refresh_pkce,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------
// Loopback token endpoint: records request bodies, serves canned
// responses (same pattern as pi-rs-ai's transport tests; sandbox-safe).
// ---------------------------------------------------------------------

fn token_server(responses: Vec<(u16, String)>) -> (SocketAddr, mpsc::UnboundedReceiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut responses = responses.into_iter();
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => return,
            };
            let body = read_request_body(&mut sock).await;
            let _ = tx.send(body);
            let Some((status, response_body)) = responses.next() else {
                return;
            };
            let reason = if status < 300 { "OK" } else { "Bad Request" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (addr, rx)
}

async fn read_request_body(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return String::new(),
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let content_length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while buf.len() - (pos + 4) < content_length {
                let n = match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
            }
            return String::from_utf8_lossy(&buf[pos + 4..]).into_owned();
        }
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn test_flow(callback_port: u16, token_addr: SocketAddr) -> PkceFlow {
    PkceFlow {
        id: "testprov".into(),
        name: "Test Provider".into(),
        error_label: "Anthropic".into(),
        client_id: "client-123".into(),
        authorize_url: "https://auth.test/oauth/authorize".into(),
        token_url: format!("http://{token_addr}/v1/oauth/token"),
        callback_port,
        callback_path: "/callback".into(),
        scopes: "scope:a scope:b".into(),
        extra_auth_params: vec![("code".into(), "true".into())],
        instructions: "Complete login in your browser.".into(),
        pages: CallbackPages {
            success: "Done.".into(),
            denied: "Not done.".into(),
        },
    }
}

const TOKEN_JSON: &str = r#"{"access_token":"acc-1","refresh_token":"ref-1","expires_in":3600}"#;

// ---------------------------------------------------------------------
// Callbacks test double.
// ---------------------------------------------------------------------

/// on_auth pushes the auth URL; manual / prompt inputs are canned.
struct TestCallbacks {
    auth_tx: mpsc::UnboundedSender<OAuthAuthInfo>,
    manual: Mutex<Option<Result<String, AuthError>>>,
    prompt: Mutex<Option<String>>,
}

impl TestCallbacks {
    fn new() -> (Self, mpsc::UnboundedReceiver<OAuthAuthInfo>) {
        let (auth_tx, auth_rx) = mpsc::unbounded_channel();
        (
            Self {
                auth_tx,
                manual: Mutex::new(None),
                prompt: Mutex::new(None),
            },
            auth_rx,
        )
    }
}

impl OAuthLoginCallbacks for TestCallbacks {
    fn on_auth(&self, info: OAuthAuthInfo) {
        let _ = self.auth_tx.send(info);
    }

    fn on_device_code(&self, _info: OAuthDeviceCodeInfo) {}

    fn on_prompt(&self, _prompt: OAuthPrompt) -> AuthFuture<'_, String> {
        let canned = self.prompt.lock().unwrap().take();
        Box::pin(async move { canned.ok_or(AuthError::Cancelled) })
    }

    fn on_select(&self, _prompt: OAuthSelectPrompt) -> AuthFuture<'_, Option<String>> {
        Box::pin(async move { Ok(None) })
    }

    fn on_manual_code_input(&self) -> Option<AuthFuture<'_, String>> {
        let canned = self.manual.lock().unwrap().take()?;
        Some(Box::pin(async move { canned }))
    }
}

fn query_param(url: &str, key: &str) -> Option<String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

// ---------------------------------------------------------------------
// Login via the callback server.
// ---------------------------------------------------------------------

#[tokio::test]
async fn login_via_callback_server() {
    let (token_addr, mut bodies) = token_server(vec![(200, TOKEN_JSON.into())]);
    let port = free_port();
    let flow = test_flow(port, token_addr);
    let (callbacks, mut auth_rx) = TestCallbacks::new();

    let login = tokio::spawn(async move { login_pkce(&flow, &callbacks).await });

    // The auth URL carries the spec's param set; state == verifier.
    let info = auth_rx.recv().await.unwrap();
    assert!(
        info.url
            .starts_with("https://auth.test/oauth/authorize?code=true&client_id=client-123&")
    );
    assert_eq!(
        info.instructions.as_deref(),
        Some("Complete login in your browser.")
    );
    let state = query_param(&info.url, "state").unwrap();
    let challenge = query_param(&info.url, "code_challenge").unwrap();
    assert_eq!(challenge, pi_rs_ai_auth::challenge_for(&state));
    assert_eq!(
        query_param(&info.url, "redirect_uri").unwrap(),
        format!("http://localhost:{port}/callback")
    );
    assert_eq!(query_param(&info.url, "scope").unwrap(), "scope:a scope:b");
    assert_eq!(
        query_param(&info.url, "code_challenge_method").unwrap(),
        "S256"
    );
    assert_eq!(query_param(&info.url, "response_type").unwrap(), "code");

    // Complete the browser redirect.
    let response = reqwest::get(format!(
        "http://127.0.0.1:{port}/callback?code=auth-code&state={state}"
    ))
    .await
    .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    let credentials = login.await.unwrap().unwrap();
    assert_eq!(credentials.access, "acc-1");
    assert_eq!(credentials.refresh, "ref-1");
    // Spec: now + expires_in*1000 - 5min skew.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let expected = now + 3_600_000 - 300_000;
    assert!((credentials.expires - expected).abs() < 5_000);

    // Token exchange body matches the spec's grant.
    let body: serde_json::Value = serde_json::from_str(&bodies.recv().await.unwrap()).unwrap();
    assert_eq!(body["grant_type"], "authorization_code");
    assert_eq!(body["client_id"], "client-123");
    assert_eq!(body["code"], "auth-code");
    assert_eq!(body["state"], state);
    assert_eq!(body["code_verifier"], state);
    assert_eq!(
        body["redirect_uri"],
        format!("http://localhost:{port}/callback").as_str()
    );
}

// ---------------------------------------------------------------------
// Manual code input.
// ---------------------------------------------------------------------

#[tokio::test]
async fn login_via_manual_bare_code() {
    let (token_addr, mut bodies) = token_server(vec![(200, TOKEN_JSON.into())]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    *callbacks.manual.lock().unwrap() = Some(Ok("manual-code".into()));

    let credentials = login_pkce(&flow, &callbacks).await.unwrap();
    assert_eq!(credentials.access, "acc-1");

    // Spec: bare code → state defaults to the verifier.
    let body: serde_json::Value = serde_json::from_str(&bodies.recv().await.unwrap()).unwrap();
    assert_eq!(body["code"], "manual-code");
    assert_eq!(body["state"], body["code_verifier"]);
}

#[tokio::test]
async fn manual_state_mismatch_rejects() {
    let (token_addr, _bodies) = token_server(vec![]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    *callbacks.manual.lock().unwrap() = Some(Ok("some-code#wrong-state".into()));

    let err = login_pkce(&flow, &callbacks).await.unwrap_err();
    assert_eq!(err.to_string(), "OAuth state mismatch");
}

#[tokio::test]
async fn manual_error_propagates() {
    let (token_addr, _bodies) = token_server(vec![]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    *callbacks.manual.lock().unwrap() = Some(Err(AuthError::Cancelled));

    let err = login_pkce(&flow, &callbacks).await.unwrap_err();
    assert!(matches!(err, AuthError::Cancelled));
}

// ---------------------------------------------------------------------
// Prompt fallback.
// ---------------------------------------------------------------------

#[tokio::test]
async fn empty_manual_input_falls_back_to_prompt() {
    let (token_addr, mut bodies) = token_server(vec![(200, TOKEN_JSON.into())]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    // Spec: empty manual input parses to no code → onPrompt path.
    *callbacks.manual.lock().unwrap() = Some(Ok(String::new()));
    *callbacks.prompt.lock().unwrap() = Some("prompt-code".into());

    let credentials = login_pkce(&flow, &callbacks).await.unwrap();
    assert_eq!(credentials.access, "acc-1");
    let body: serde_json::Value = serde_json::from_str(&bodies.recv().await.unwrap()).unwrap();
    assert_eq!(body["code"], "prompt-code");
}

#[tokio::test]
async fn missing_code_after_prompt_rejects() {
    let (token_addr, _bodies) = token_server(vec![]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    *callbacks.manual.lock().unwrap() = Some(Ok(String::new()));
    *callbacks.prompt.lock().unwrap() = Some(String::new());

    let err = login_pkce(&flow, &callbacks).await.unwrap_err();
    assert_eq!(err.to_string(), "Missing authorization code");
}

// ---------------------------------------------------------------------
// Token endpoint errors — spec message strings.
// ---------------------------------------------------------------------

#[tokio::test]
async fn exchange_failure_matches_spec_message() {
    let (token_addr, _bodies) = token_server(vec![(400, "nope".into())]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    *callbacks.manual.lock().unwrap() = Some(Ok("manual-code".into()));

    let err = login_pkce(&flow, &callbacks).await.unwrap_err().to_string();
    let token_url = format!("http://{token_addr}/v1/oauth/token");
    let redirect_uri = flow.redirect_uri();
    assert_eq!(
        err,
        format!(
            "Token exchange request failed. url={token_url}; redirect_uri={redirect_uri}; response_type=authorization_code; details=HTTP request failed. status=400; url={token_url}; body=nope"
        )
    );
}

#[tokio::test]
async fn exchange_invalid_json_matches_spec_message() {
    let (token_addr, _bodies) = token_server(vec![(200, "not-json".into())]);
    let flow = test_flow(free_port(), token_addr);
    let (callbacks, _auth_rx) = TestCallbacks::new();
    *callbacks.manual.lock().unwrap() = Some(Ok("manual-code".into()));

    let err = login_pkce(&flow, &callbacks).await.unwrap_err().to_string();
    let token_url = format!("http://{token_addr}/v1/oauth/token");
    assert!(err.starts_with(&format!(
        "Token exchange returned invalid JSON. url={token_url}; body=not-json; details="
    )));
}

// ---------------------------------------------------------------------
// Refresh.
// ---------------------------------------------------------------------

#[tokio::test]
async fn refresh_success_and_body() {
    let (token_addr, mut bodies) = token_server(vec![(200, TOKEN_JSON.into())]);
    let flow = test_flow(free_port(), token_addr);

    let credentials = refresh_pkce(&flow, "old-refresh").await.unwrap();
    assert_eq!(credentials.access, "acc-1");
    assert_eq!(credentials.refresh, "ref-1");

    let body: serde_json::Value = serde_json::from_str(&bodies.recv().await.unwrap()).unwrap();
    assert_eq!(body["grant_type"], "refresh_token");
    assert_eq!(body["client_id"], "client-123");
    assert_eq!(body["refresh_token"], "old-refresh");
}

#[tokio::test]
async fn refresh_failure_matches_spec_message() {
    let (token_addr, _bodies) = token_server(vec![(401, "denied".into())]);
    let flow = test_flow(free_port(), token_addr);

    let err = refresh_pkce(&flow, "old-refresh")
        .await
        .unwrap_err()
        .to_string();
    let token_url = format!("http://{token_addr}/v1/oauth/token");
    assert_eq!(
        err,
        format!(
            "Anthropic token refresh request failed. url={token_url}; details=HTTP request failed. status=401; url={token_url}; body=denied"
        )
    );
}
