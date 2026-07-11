//! The PKCE authorization-code engine — flows as data.
//!
//! Spec: `utils/oauth/anthropic.ts` login/refresh mechanics, with the
//! provider-specific constants lifted into [`PkceFlow`] per the locked
//! `pi-rs-ai` row (one PKCE engine + flows-as-data; irreducibly weird
//! flows — codex, copilot — arrive in WS5 as code sharing this
//! machinery). Error-message strings match the spec verbatim.
//!
//! Recorded divergences:
//! - the spec's `state` is always the PKCE verifier (anthropic's
//!   choice); a flow needing a random state parameterizes this in WS5;
//! - the token response is decoded with required fields — the spec's
//!   `JSON.parse` cast would silently produce corrupt credentials on a
//!   missing field, which we refuse (missing fields report as the
//!   invalid-JSON error).

use serde::Deserialize;
use serde_json::json;

use crate::callback_server::{CallbackPages, CallbackServer};
use crate::error::AuthError;
use crate::pkce::generate_pkce;
use crate::types::{OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthPrompt};

/// Spec: tokens are treated as expired 5 minutes early
/// (`expires_in * 1000 - 5 * 60 * 1000`).
const EXPIRY_SKEW_MS: i64 = 5 * 60 * 1000;

/// One authorization-code + PKCE flow, as data. The anthropic flow is
/// the first row (`crate::anthropic::anthropic_flow`).
#[derive(Clone)]
pub struct PkceFlow {
    /// Spec: `OAuthProviderInterface.id`.
    pub id: String,
    /// Spec: `OAuthProviderInterface.name`.
    pub name: String,
    /// Prefix for refresh error messages (spec: "Anthropic token
    /// refresh request failed. …").
    pub error_label: String,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub callback_port: u16,
    pub callback_path: String,
    pub scopes: String,
    /// Extra authorize-URL query params prepended before the standard
    /// ones (spec order — anthropic sends `code=true` first).
    pub extra_auth_params: Vec<(String, String)>,
    /// Spec: the `instructions` string passed to `onAuth`.
    pub instructions: String,
    /// Callback-server page bodies (flow-specific wording).
    pub pages: CallbackPages,
}

impl PkceFlow {
    /// Spec: `REDIRECT_URI` — always `localhost`, independent of the
    /// bind host override.
    pub fn redirect_uri(&self) -> String {
        format!(
            "http://localhost:{}{}",
            self.callback_port, self.callback_path
        )
    }
}

/// Milliseconds since the epoch (spec: `Date.now()`).
pub(crate) fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

/// Spec: `parseAuthorizationInput` — accepts a full redirect URL, a
/// `code#state` pair, query-string syntax, or a bare code. Returns
/// `(code, state)`; empty strings are preserved (the spec's truthiness
/// checks happen at the call sites).
pub fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }

    if let Ok(url) = url::Url::parse(value) {
        let mut code = None;
        let mut state = None;
        for (key, val) in url.query_pairs() {
            match key.as_ref() {
                "code" if code.is_none() => code = Some(val.into_owned()),
                "state" if state.is_none() => state = Some(val.into_owned()),
                _ => {}
            }
        }
        return (code, state);
    }

    if value.contains('#') {
        // Spec: `value.split("#", 2)` — JS limit truncates, it does not
        // keep the remainder.
        let mut parts = value.split('#');
        let code = parts.next().map(str::to_owned);
        let state = parts.next().map(str::to_owned);
        return (code, state);
    }

    if value.contains("code=") {
        let mut code = None;
        let mut state = None;
        for (key, val) in url::form_urlencoded::parse(value.as_bytes()) {
            match key.as_ref() {
                "code" if code.is_none() => code = Some(val.into_owned()),
                "state" if state.is_none() => state = Some(val.into_owned()),
                _ => {}
            }
        }
        return (code, state);
    }

    (Some(value.to_owned()), None)
}

/// Spec truthiness: `parsed.state && parsed.state !== verifier`.
fn check_state_mismatch(state: Option<&str>, verifier: &str) -> Result<(), AuthError> {
    match state {
        Some(state) if !state.is_empty() && state != verifier => {
            Err(AuthError::Message("OAuth state mismatch".into()))
        }
        _ => Ok(()),
    }
}

/// Spec: `loginAnthropic`, generalized over the flow row.
pub async fn login_pkce(
    flow: &PkceFlow,
    callbacks: &dyn OAuthLoginCallbacks,
) -> Result<OAuthCredentials, AuthError> {
    let pkce = generate_pkce()?;
    let verifier = pkce.verifier;
    // Spec: the PKCE verifier doubles as the OAuth state.
    let mut server = CallbackServer::start(
        flow.callback_port,
        &flow.callback_path,
        verifier.clone(),
        flow.pages.clone(),
    )
    .await?;
    let redirect_uri = flow.redirect_uri();

    // Spec param order: flow extras first (`code=true`), then the
    // standard authorization-code + PKCE params.
    let mut auth_url = url::Url::parse(&flow.authorize_url)
        .map_err(|e| AuthError::Message(format!("invalid authorize url: {e}")))?;
    {
        let mut params = auth_url.query_pairs_mut();
        for (key, value) in &flow.extra_auth_params {
            params.append_pair(key, value);
        }
        params
            .append_pair("client_id", &flow.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", &flow.scopes)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &verifier);
    }

    callbacks.on_auth(OAuthAuthInfo {
        url: auth_url.to_string(),
        instructions: Some(flow.instructions.clone()),
    });

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;

    if let Some(manual) = callbacks.on_manual_code_input() {
        // Spec: race the callback server against manual input (manual
        // completion cancels the wait; a manual error is thrown).
        tokio::select! {
            settled = server.wait_for_code() => {
                if let Some(settled) = settled {
                    code = Some(settled.code);
                    state = Some(settled.state);
                }
            }
            input = manual => {
                let input = input?;
                let (parsed_code, parsed_state) = parse_authorization_input(&input);
                check_state_mismatch(parsed_state.as_deref(), &verifier)?;
                code = parsed_code;
                // Spec: `parsed.state ?? verifier` (empty string kept).
                state = parsed_state.or_else(|| Some(verifier.clone()));
            }
        }
    } else if let Some(settled) = server.wait_for_code().await {
        code = Some(settled.code);
        state = Some(settled.state);
    }

    // Spec: `if (!code)` — empty counts as missing.
    if code.as_deref().is_none_or(str::is_empty) {
        let input = callbacks
            .on_prompt(OAuthPrompt {
                message: "Paste the authorization code or full redirect URL:".into(),
                placeholder: Some(redirect_uri.clone()),
                allow_empty: false,
            })
            .await?;
        let (parsed_code, parsed_state) = parse_authorization_input(&input);
        check_state_mismatch(parsed_state.as_deref(), &verifier)?;
        code = parsed_code;
        state = parsed_state.or_else(|| Some(verifier.clone()));
    }

    let code = code
        .filter(|code| !code.is_empty())
        .ok_or_else(|| AuthError::Message("Missing authorization code".into()))?;
    let state = state
        .filter(|state| !state.is_empty())
        .ok_or_else(|| AuthError::Message("Missing OAuth state".into()))?;

    callbacks.on_progress("Exchanging authorization code for tokens...");
    exchange_authorization_code(flow, &code, &state, &verifier, &redirect_uri).await
    // Spec `finally`: the server closes when it drops.
}

/// Spec: `refreshAnthropicToken`, generalized over the flow row.
pub async fn refresh_pkce(
    flow: &PkceFlow,
    refresh_token: &str,
) -> Result<OAuthCredentials, AuthError> {
    let body = post_json(
        &flow.token_url,
        &json!({
            "grant_type": "refresh_token",
            "client_id": flow.client_id,
            "refresh_token": refresh_token,
        }),
    )
    .await
    .map_err(|e| {
        AuthError::Message(format!(
            "{} token refresh request failed. url={}; details={e}",
            flow.error_label, flow.token_url
        ))
    })?;

    parse_token_response(&body).map_err(|e| {
        AuthError::Message(format!(
            "{} token refresh returned invalid JSON. url={}; body={body}; details={e}",
            flow.error_label, flow.token_url
        ))
    })
}

/// Spec: `exchangeAuthorizationCode`.
async fn exchange_authorization_code(
    flow: &PkceFlow,
    code: &str,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthCredentials, AuthError> {
    let body = post_json(
        &flow.token_url,
        &json!({
            "grant_type": "authorization_code",
            "client_id": flow.client_id,
            "code": code,
            "state": state,
            "redirect_uri": redirect_uri,
            "code_verifier": verifier,
        }),
    )
    .await
    .map_err(|e| {
        AuthError::Message(format!(
            "Token exchange request failed. url={}; redirect_uri={redirect_uri}; response_type=authorization_code; details={e}",
            flow.token_url
        ))
    })?;

    parse_token_response(&body).map_err(|e| {
        AuthError::Message(format!(
            "Token exchange returned invalid JSON. url={}; body={body}; details={e}",
            flow.token_url
        ))
    })
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

fn parse_token_response(body: &str) -> Result<OAuthCredentials, serde_json::Error> {
    let token: TokenResponse = serde_json::from_str(body)?;
    Ok(OAuthCredentials {
        refresh: token.refresh_token,
        access: token.access_token,
        expires: now_ms() + token.expires_in * 1000 - EXPIRY_SKEW_MS,
        extra: serde_json::Map::new(),
    })
}

/// Spec: `postJson` — a plain 30s-timeout POST, no retry; non-2xx is the
/// spec's `HTTP request failed. status=…; url=…; body=…` error.
async fn post_json(url: &str, body: &serde_json::Value) -> Result<String, AuthError> {
    let response = reqwest::Client::new()
        .post(url)
        .timeout(std::time::Duration::from_secs(30))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(body.to_string())
        .send()
        .await?;

    let status = response.status().as_u16();
    let response_body = response.text().await?;

    if !(200..300).contains(&status) {
        return Err(AuthError::Message(format!(
            "HTTP request failed. status={status}; url={url}; body={response_body}"
        )));
    }

    Ok(response_body)
}
