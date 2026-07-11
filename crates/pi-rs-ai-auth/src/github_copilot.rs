//! GitHub Copilot OAuth device flow.
//!
//! Spec: `utils/oauth/github-copilot.ts`. GitHub's long-lived access token is
//! stored as `refresh`; short-lived Copilot tokens are refreshed from it.

use serde_json::Value;

use crate::device_code::{DeviceCodePoll, poll_device_code};

use crate::error::AuthError;
use crate::types::{
    AuthFuture, OAuthCredentials, OAuthDeviceCodeInfo, OAuthLoginCallbacks, OAuthPrompt,
    OAuthProviderInterface,
};
use pi_rs_ai_types::Model;

const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const EXPIRY_SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug)]
pub struct GitHubCopilotEndpoints {
    pub device_code_url: String,
    pub access_token_url: String,
    pub copilot_token_url: String,
}

impl GitHubCopilotEndpoints {
    fn for_domain(domain: &str) -> Self {
        Self {
            device_code_url: format!("https://{domain}/login/device/code"),
            access_token_url: format!("https://{domain}/login/oauth/access_token"),
            copilot_token_url: format!("https://api.{domain}/copilot_internal/v2/token"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GitHubCopilotFlow {
    /// Deterministic endpoint seams; built-in behavior derives URLs from the
    /// selected github.com / enterprise domain and returned Copilot token.
    pub endpoints_override: Option<GitHubCopilotEndpoints>,
    pub policy_base_url_override: Option<String>,
}

pub fn github_copilot_flow() -> GitHubCopilotFlow {
    GitHubCopilotFlow::default()
}

pub fn normalize_github_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    url::Url::parse(&value)
        .ok()?
        .host_str()
        .filter(|host| !host.is_empty())
        .map(str::to_owned)
}

pub fn github_copilot_base_url(token: Option<&str>, enterprise_domain: Option<&str>) -> String {
    if let Some(token) = token {
        if let Some(proxy_host) = token
            .split(';')
            .find_map(|part| part.strip_prefix("proxy-ep="))
            .filter(|host| !host.is_empty())
        {
            let api_host = proxy_host
                .strip_prefix("proxy.")
                .map(|host| format!("api.{host}"))
                .unwrap_or_else(|| proxy_host.to_owned());
            return format!("https://{api_host}");
        }
    }
    enterprise_domain
        .map(|domain| format!("https://copilot-api.{domain}"))
        .unwrap_or_else(|| "https://api.individual.githubcopilot.com".into())
}

fn copilot_headers(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header("User-Agent", USER_AGENT)
        .header("Editor-Version", "vscode/1.107.0")
        .header("Editor-Plugin-Version", "copilot-chat/0.35.0")
        .header("Copilot-Integration-Id", "vscode-chat")
}

async fn fetch_json(
    request: reqwest::RequestBuilder,
    callbacks: Option<&dyn OAuthLoginCallbacks>,
) -> Result<Value, AuthError> {
    let response = if let Some(callbacks) = callbacks {
        tokio::select! {
            response = request.send() => response?,
            result = callbacks.on_cancelled() => {
                result?;
                return Err(AuthError::Message("Login cancelled".into()));
            }
        }
    } else {
        request.send().await?
    };
    if !response.status().is_success() {
        let status = response.status();
        let reason = status.canonical_reason().unwrap_or("");
        let body = response.text().await.unwrap_or_default();
        return Err(AuthError::Message(format!(
            "{} {reason}: {body}",
            status.as_u16()
        )));
    }
    let body = response.text().await?;
    Ok(serde_json::from_str(&body)?)
}

impl GitHubCopilotFlow {
    fn endpoints(&self, domain: &str) -> GitHubCopilotEndpoints {
        self.endpoints_override
            .clone()
            .unwrap_or_else(|| GitHubCopilotEndpoints::for_domain(domain))
    }

    async fn refresh(
        &self,
        refresh_token: &str,
        enterprise_domain: Option<&str>,
    ) -> Result<OAuthCredentials, AuthError> {
        let domain = enterprise_domain.unwrap_or("github.com");
        let endpoints = self.endpoints(domain);
        let value = fetch_json(
            copilot_headers(
                reqwest::Client::new()
                    .get(endpoints.copilot_token_url)
                    .header("Accept", "application/json")
                    .bearer_auth(refresh_token),
            ),
            None,
        )
        .await?;
        if !value.is_object() {
            return Err(AuthError::Message("Invalid Copilot token response".into()));
        }
        let token = value.get("token").and_then(Value::as_str);
        let expires_at = value.get("expires_at").and_then(Value::as_f64);
        let (Some(token), Some(expires_at)) = (token, expires_at) else {
            return Err(AuthError::Message(
                "Invalid Copilot token response fields".into(),
            ));
        };
        let mut extra = serde_json::Map::new();
        if let Some(domain) = enterprise_domain {
            extra.insert("enterpriseUrl".into(), Value::String(domain.into()));
        }
        Ok(OAuthCredentials {
            refresh: refresh_token.into(),
            access: token.into(),
            expires: (expires_at * 1000.0) as i64 - EXPIRY_SKEW_MS,
            extra,
        })
    }

    async fn enable_model(&self, token: &str, model_id: &str, enterprise_domain: Option<&str>) {
        let base_url = self
            .policy_base_url_override
            .clone()
            .unwrap_or_else(|| github_copilot_base_url(Some(token), enterprise_domain));
        let _ = copilot_headers(
            reqwest::Client::new()
                .post(format!("{base_url}/models/{model_id}/policy"))
                .header("Content-Type", "application/json")
                .bearer_auth(token)
                .header("openai-intent", "chat-policy")
                .header("x-interaction-type", "chat-policy")
                .body(serde_json::json!({ "state": "enabled" }).to_string()),
        )
        .send()
        .await;
    }

    async fn login_device(
        &self,
        callbacks: &dyn OAuthLoginCallbacks,
    ) -> Result<OAuthCredentials, AuthError> {
        let input = callbacks
            .on_prompt(OAuthPrompt {
                message: "GitHub Enterprise URL/domain (blank for github.com)".into(),
                placeholder: Some("company.ghe.com".into()),
                allow_empty: true,
            })
            .await?;
        if callbacks.is_cancelled() {
            return Err(AuthError::Message("Login cancelled".into()));
        }
        let trimmed = input.trim();
        let enterprise_domain = normalize_github_domain(&input);
        if !trimmed.is_empty() && enterprise_domain.is_none() {
            return Err(AuthError::Message(
                "Invalid GitHub Enterprise URL/domain".into(),
            ));
        }
        let domain = enterprise_domain.as_deref().unwrap_or("github.com");
        let endpoints = self.endpoints(domain);
        let value = fetch_json(
            reqwest::Client::new()
                .post(&endpoints.device_code_url)
                .header("Accept", "application/json")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("User-Agent", USER_AGENT)
                .form(&[("client_id", CLIENT_ID), ("scope", "read:user")]),
            Some(callbacks),
        )
        .await?;
        if !value.is_object() {
            return Err(AuthError::Message("Invalid device code response".into()));
        }
        let device_code = value.get("device_code").and_then(Value::as_str);
        let user_code = value.get("user_code").and_then(Value::as_str);
        let verification_uri = value.get("verification_uri").and_then(Value::as_str);
        let interval = value.get("interval").and_then(Value::as_f64);
        let expires_in = value.get("expires_in").and_then(Value::as_f64);
        let (Some(device_code), Some(user_code), Some(verification_uri), Some(expires_in)) =
            (device_code, user_code, verification_uri, expires_in)
        else {
            return Err(AuthError::Message(
                "Invalid device code response fields".into(),
            ));
        };
        if value.get("interval").is_some() && interval.is_none() {
            return Err(AuthError::Message(
                "Invalid device code response fields".into(),
            ));
        }
        let verification_uri = url::Url::parse(verification_uri)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .ok_or_else(|| {
                AuthError::Message("Untrusted verification_uri in device code response".into())
            })?
            .to_string();
        callbacks.on_device_code(OAuthDeviceCodeInfo {
            user_code: user_code.into(),
            verification_uri,
            interval_seconds: interval,
            expires_in_seconds: Some(expires_in),
        });

        let client = reqwest::Client::new();
        let access_url = endpoints.access_token_url.clone();
        let device_code = device_code.to_owned();
        let github_access_token = poll_device_code(interval, Some(expires_in), callbacks, || {
            let client = client.clone();
            let access_url = access_url.clone();
            let device_code = device_code.clone();
            async move {
                let value = fetch_json(
                    client
                        .post(access_url)
                        .header("Accept", "application/json")
                        .header("Content-Type", "application/x-www-form-urlencoded")
                        .header("User-Agent", USER_AGENT)
                        .form(&[
                            ("client_id", CLIENT_ID),
                            ("device_code", device_code.as_str()),
                            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ]),
                    None,
                )
                .await?;
                if let Some(token) = value.get("access_token").and_then(Value::as_str) {
                    return Ok(DeviceCodePoll::Complete(token.to_owned()));
                }
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    return Ok(match error {
                        "authorization_pending" => DeviceCodePoll::Pending,
                        "slow_down" => DeviceCodePoll::SlowDown,
                        _ => {
                            let suffix = value
                                .get("error_description")
                                .and_then(Value::as_str)
                                .map(|description| format!(": {description}"))
                                .unwrap_or_default();
                            DeviceCodePoll::Failed(format!("Device flow failed: {error}{suffix}"))
                        }
                    });
                }
                Ok(DeviceCodePoll::Failed(
                    "Invalid device token response".into(),
                ))
            }
        })
        .await?;

        let credentials = self
            .refresh(&github_access_token, enterprise_domain.as_deref())
            .await?;
        callbacks.on_progress("Enabling models...");
        let mut tasks = tokio::task::JoinSet::new();
        for model_id in callbacks.provider_model_ids("github-copilot") {
            let flow = self.clone();
            let token = credentials.access.clone();
            let enterprise_domain = enterprise_domain.clone();
            tasks.spawn(async move {
                flow.enable_model(&token, &model_id, enterprise_domain.as_deref())
                    .await;
            });
        }
        while tasks.join_next().await.is_some() {}
        Ok(credentials)
    }
}

impl OAuthProviderInterface for GitHubCopilotFlow {
    fn id(&self) -> &str {
        "github-copilot"
    }
    fn name(&self) -> &str {
        "GitHub Copilot"
    }

    fn login<'a>(
        &'a self,
        callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(self.login_device(callbacks))
    }

    fn refresh_token<'a>(
        &'a self,
        credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials> {
        let enterprise_domain = credentials
            .extra
            .get("enterpriseUrl")
            .and_then(Value::as_str);
        Box::pin(self.refresh(&credentials.refresh, enterprise_domain))
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }

    fn modify_models(&self, mut models: Vec<Model>, credentials: &OAuthCredentials) -> Vec<Model> {
        let enterprise_domain = credentials
            .extra
            .get("enterpriseUrl")
            .and_then(Value::as_str);
        let base_url = github_copilot_base_url(Some(&credentials.access), enterprise_domain);
        for model in &mut models {
            if model.provider == "github-copilot" {
                model.base_url.clone_from(&base_url);
            }
        }
        models
    }
}
