//! OpenAI Codex (ChatGPT subscription) OAuth.
//!
//! Spec: `utils/oauth/openai-codex.ts`. The browser PKCE and device-code
//! methods share token parsing and credential extraction. Endpoints are fields
//! so deterministic tests can replay the flow against loopback servers.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::callback_server::{CallbackPages, CallbackServer};
use crate::device_code::{DeviceCodePoll, poll_device_code};
use crate::engine::{now_ms, parse_authorization_input};
use crate::error::AuthError;
use crate::pkce::generate_pkce;
use crate::types::{
    AuthFuture, OAuthAuthInfo, OAuthCredentials, OAuthDeviceCodeInfo, OAuthLoginCallbacks,
    OAuthPrompt, OAuthProviderInterface, OAuthSelectOption, OAuthSelectPrompt,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const SCOPE: &str = "openid profile email offline_access";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
const DEVICE_CODE_TIMEOUT_SECONDS: f64 = 15.0 * 60.0;
pub const OPENAI_CODEX_BROWSER_LOGIN_METHOD: &str = "browser";
pub const OPENAI_CODEX_DEVICE_CODE_LOGIN_METHOD: &str = "device_code";

/// URLs and callback port used by the Codex flow.
#[derive(Clone, Debug)]
pub struct OpenAiCodexEndpoints {
    pub authorize_url: String,
    pub token_url: String,
    pub device_user_code_url: String,
    pub device_token_url: String,
    pub device_verification_uri: String,
    pub device_redirect_uri: String,
    pub callback_port: u16,
}

impl Default for OpenAiCodexEndpoints {
    fn default() -> Self {
        Self {
            authorize_url: "https://auth.openai.com/oauth/authorize".into(),
            token_url: "https://auth.openai.com/oauth/token".into(),
            device_user_code_url: "https://auth.openai.com/api/accounts/deviceauth/usercode".into(),
            device_token_url: "https://auth.openai.com/api/accounts/deviceauth/token".into(),
            device_verification_uri: "https://auth.openai.com/codex/device".into(),
            device_redirect_uri: "https://auth.openai.com/deviceauth/callback".into(),
            callback_port: 1455,
        }
    }
}

/// Built-in Codex OAuth provider.
#[derive(Clone, Debug, Default)]
pub struct OpenAiCodexFlow {
    pub endpoints: OpenAiCodexEndpoints,
}

pub fn openai_codex_flow() -> OpenAiCodexFlow {
    OpenAiCodexFlow::default()
}

impl OpenAiCodexFlow {
    fn redirect_uri(&self) -> String {
        format!(
            "http://localhost:{}/auth/callback",
            self.endpoints.callback_port
        )
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        callbacks: Option<&dyn OAuthLoginCallbacks>,
    ) -> Result<reqwest::Response, AuthError> {
        if let Some(callbacks) = callbacks {
            tokio::select! {
                response = request.send() => response.map_err(AuthError::from),
                result = callbacks.on_cancelled() => {
                    result?;
                    Err(AuthError::Message("Login cancelled".into()))
                }
            }
        } else {
            Ok(request.send().await?)
        }
    }

    async fn read_token_response(
        response: reqwest::Response,
        operation: &str,
    ) -> Result<OAuthCredentials, AuthError> {
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let fallback = status.canonical_reason().unwrap_or("");
            let detail = if text.is_empty() { fallback } else { &text };
            return Err(AuthError::Message(format!(
                "OpenAI Codex token {operation} failed ({}): {detail}",
                status.as_u16()
            )));
        }
        let body = response.text().await?;
        let value: Value = serde_json::from_str(&body)?;
        let access = value.get("access_token").and_then(Value::as_str);
        let refresh = value.get("refresh_token").and_then(Value::as_str);
        let expires_in = value.get("expires_in").and_then(Value::as_f64);
        let (Some(access), Some(refresh), Some(expires_in)) = (access, refresh, expires_in) else {
            return Err(AuthError::Message(format!(
                "OpenAI Codex token {operation} response missing fields: {value}"
            )));
        };
        Ok(OAuthCredentials {
            access: access.into(),
            refresh: refresh.into(),
            expires: now_ms().saturating_add((expires_in * 1000.0) as i64),
            extra: serde_json::Map::new(),
        })
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        callbacks: &dyn OAuthLoginCallbacks,
    ) -> Result<OAuthCredentials, AuthError> {
        let response = self
            .send(
                reqwest::Client::new()
                    .post(&self.endpoints.token_url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(&[
                        ("grant_type", "authorization_code"),
                        ("client_id", CLIENT_ID),
                        ("code", code),
                        ("code_verifier", verifier),
                        ("redirect_uri", redirect_uri),
                    ]),
                Some(callbacks),
            )
            .await
            .map_err(|error| {
                if callbacks.is_cancelled() {
                    AuthError::Message("Login cancelled".into())
                } else {
                    error
                }
            })?;
        Self::credentials_from_token(Self::read_token_response(response, "exchange").await?)
    }

    async fn refresh(&self, refresh_token: &str) -> Result<OAuthCredentials, AuthError> {
        let response = self
            .send(
                reqwest::Client::new()
                    .post(&self.endpoints.token_url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(&[
                        ("grant_type", "refresh_token"),
                        ("refresh_token", refresh_token),
                        ("client_id", CLIENT_ID),
                    ]),
                None,
            )
            .await
            .map_err(|error| {
                AuthError::Message(format!("OpenAI Codex token refresh error: {error}"))
            })?;
        Self::credentials_from_token(Self::read_token_response(response, "refresh").await?)
    }

    fn credentials_from_token(mut token: OAuthCredentials) -> Result<OAuthCredentials, AuthError> {
        let account_id = account_id(&token.access)
            .ok_or_else(|| AuthError::Message("Failed to extract accountId from token".into()))?;
        token
            .extra
            .insert("accountId".into(), Value::String(account_id));
        Ok(token)
    }

    async fn login_browser(
        &self,
        callbacks: &dyn OAuthLoginCallbacks,
    ) -> Result<OAuthCredentials, AuthError> {
        let pkce = generate_pkce()?;
        let mut state_bytes = [0_u8; 16];
        getrandom::fill(&mut state_bytes)
            .map_err(|error| AuthError::Message(format!("failed to gather randomness: {error}")))?;
        let state = state_bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let redirect_uri = self.redirect_uri();
        let mut url = url::Url::parse(&self.endpoints.authorize_url)
            .map_err(|error| AuthError::Message(format!("invalid authorize url: {error}")))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", CLIENT_ID)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", "pi");

        // Pi deliberately falls back to manual input when port 1455 cannot bind.
        let mut server = CallbackServer::start(
            self.endpoints.callback_port,
            "/auth/callback",
            state.clone(),
            CallbackPages {
                success: "OpenAI authentication completed. You can close this window.".into(),
                denied: "OpenAI authentication did not complete.".into(),
            },
        )
        .await
        .ok();

        callbacks.on_auth(OAuthAuthInfo {
            url: url.to_string(),
            instructions: Some("A browser window should open. Complete login to finish.".into()),
        });

        let manual = callbacks.on_manual_code_input();
        let input = match (server.as_mut(), manual) {
            (Some(server), Some(manual)) => tokio::select! {
                settled = server.wait_for_code() => settled.map(|value| value.code),
                manual = manual => Some(manual?),
            },
            (Some(server), None) => server.wait_for_code().await.map(|value| value.code),
            (None, Some(manual)) => Some(manual.await?),
            (None, None) => None,
        };

        let mut code = None;
        if let Some(input) = input {
            let (parsed_code, parsed_state) = parse_authorization_input(&input);
            if parsed_state
                .as_deref()
                .is_some_and(|candidate| !candidate.is_empty() && candidate != state)
            {
                return Err(AuthError::Message("State mismatch".into()));
            }
            code = parsed_code;
        }
        if code.as_deref().is_none_or(str::is_empty) {
            let input = callbacks
                .on_prompt(OAuthPrompt {
                    message: "Paste the authorization code (or full redirect URL):".into(),
                    placeholder: None,
                    allow_empty: false,
                })
                .await?;
            let (parsed_code, parsed_state) = parse_authorization_input(&input);
            if parsed_state
                .as_deref()
                .is_some_and(|candidate| !candidate.is_empty() && candidate != state)
            {
                return Err(AuthError::Message("State mismatch".into()));
            }
            code = parsed_code;
        }
        let code = code
            .filter(|code| !code.is_empty())
            .ok_or_else(|| AuthError::Message("Missing authorization code".into()))?;
        self.exchange_code(&code, &pkce.verifier, &redirect_uri, callbacks)
            .await
    }

    async fn login_device(
        &self,
        callbacks: &dyn OAuthLoginCallbacks,
    ) -> Result<OAuthCredentials, AuthError> {
        let response = self
            .send(
                reqwest::Client::new()
                    .post(&self.endpoints.device_user_code_url)
                    .header("Content-Type", "application/json")
                    .body(json!({ "client_id": CLIENT_ID }).to_string()),
                Some(callbacks),
            )
            .await?;
        let status = response.status();
        if !status.is_success() {
            if status.as_u16() == 404 {
                return Err(AuthError::Message("OpenAI Codex device code login is not enabled for this server. Use browser login or verify the server URL.".into()));
            }
            let body = response.text().await.unwrap_or_default();
            return Err(AuthError::Message(format!(
                "OpenAI Codex device code request failed with status {}{}",
                status.as_u16(),
                if body.is_empty() {
                    String::new()
                } else {
                    format!(": {body}")
                }
            )));
        }
        let body = response.text().await?;
        let value: Value = serde_json::from_str(&body)?;
        let device_auth_id = value.get("device_auth_id").and_then(Value::as_str);
        let user_code = value.get("user_code").and_then(Value::as_str);
        let interval = value.get("interval").and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.trim().parse().ok())
        });
        let (Some(device_auth_id), Some(user_code), Some(interval)) =
            (device_auth_id, user_code, interval)
        else {
            return Err(AuthError::Message(format!(
                "Invalid OpenAI Codex device code response: {value}"
            )));
        };
        if !interval.is_finite() || interval < 0.0 {
            return Err(AuthError::Message(format!(
                "Invalid OpenAI Codex device code response: {value}"
            )));
        }
        callbacks.on_device_code(OAuthDeviceCodeInfo {
            user_code: user_code.into(),
            verification_uri: self.endpoints.device_verification_uri.clone(),
            interval_seconds: Some(interval),
            expires_in_seconds: Some(DEVICE_CODE_TIMEOUT_SECONDS),
        });

        let client = reqwest::Client::new();
        let token_url = self.endpoints.device_token_url.clone();
        let auth_id = device_auth_id.to_owned();
        let user_code = user_code.to_owned();
        let device_token = poll_device_code(
            Some(interval),
            Some(DEVICE_CODE_TIMEOUT_SECONDS),
            callbacks,
            || {
                let client = client.clone();
                let token_url = token_url.clone();
                let auth_id = auth_id.clone();
                let user_code = user_code.clone();
                async move {
                    let response = client
                        .post(token_url)
                        .header("Content-Type", "application/json")
                        .body(
                            json!({ "device_auth_id": auth_id, "user_code": user_code })
                                .to_string(),
                        )
                        .send()
                        .await?;
                    let status = response.status();
                    if status.is_success() {
                        let body = response.text().await?;
                        let value: Value = serde_json::from_str(&body)?;
                        let code = value.get("authorization_code").and_then(Value::as_str);
                        let verifier = value.get("code_verifier").and_then(Value::as_str);
                        return Ok(match (code, verifier) {
                            (Some(code), Some(verifier)) => {
                                DeviceCodePoll::Complete((code.to_owned(), verifier.to_owned()))
                            }
                            _ => DeviceCodePoll::Failed(format!(
                                "Invalid OpenAI Codex device auth token response: {value}"
                            )),
                        });
                    }
                    if matches!(status.as_u16(), 403 | 404) {
                        return Ok(DeviceCodePoll::Pending);
                    }
                    let body = response.text().await.unwrap_or_default();
                    let error_code = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
                        let error = value.get("error")?;
                        error
                            .as_str()
                            .map(str::to_owned)
                            .or_else(|| error.get("code")?.as_str().map(str::to_owned))
                    });
                    Ok(match error_code.as_deref() {
                        Some("deviceauth_authorization_pending") => DeviceCodePoll::Pending,
                        Some("slow_down") => DeviceCodePoll::SlowDown,
                        _ => DeviceCodePoll::Failed(format!(
                            "OpenAI Codex device auth failed with status {}{}",
                            status.as_u16(),
                            if body.is_empty() {
                                String::new()
                            } else {
                                format!(": {body}")
                            }
                        )),
                    })
                }
            },
        )
        .await?;
        self.exchange_code(
            &device_token.0,
            &device_token.1,
            &self.endpoints.device_redirect_uri,
            callbacks,
        )
        .await
    }
}

fn account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| STANDARD.decode(payload))
        .ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value
        .get(JWT_CLAIM_PATH)?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

impl OAuthProviderInterface for OpenAiCodexFlow {
    fn id(&self) -> &str {
        "openai-codex"
    }
    fn name(&self) -> &str {
        "ChatGPT Plus/Pro (Codex Subscription)"
    }
    fn uses_callback_server(&self) -> bool {
        true
    }

    fn login<'a>(
        &'a self,
        callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(async move {
            let method = callbacks
                .on_select(OAuthSelectPrompt {
                    message: "Select OpenAI Codex login method:".into(),
                    options: vec![
                        OAuthSelectOption {
                            id: OPENAI_CODEX_BROWSER_LOGIN_METHOD.into(),
                            label: "Browser login (default)".into(),
                        },
                        OAuthSelectOption {
                            id: OPENAI_CODEX_DEVICE_CODE_LOGIN_METHOD.into(),
                            label: "Device code login (headless)".into(),
                        },
                    ],
                })
                .await?
                .ok_or_else(|| AuthError::Message("Login cancelled".into()))?;
            match method.as_str() {
                OPENAI_CODEX_BROWSER_LOGIN_METHOD => self.login_browser(callbacks).await,
                OPENAI_CODEX_DEVICE_CODE_LOGIN_METHOD => self.login_device(callbacks).await,
                _ => Err(AuthError::Message(format!(
                    "Unknown OpenAI Codex login method: {method}"
                ))),
            }
        })
    }

    fn refresh_token<'a>(
        &'a self,
        credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(self.refresh(&credentials.refresh))
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }
}
