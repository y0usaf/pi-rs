//! ai/src/auth/oauth/github-copilot.ts: GitHub Copilot's device-code sign-in, the Copilot token
//! exchange that follows it, and the model catalogue calls the login and every refresh make.

use std::time::Duration;

use pi_rt::cancel::CancelToken;
use serde_json::{Map, Value, json};

use crate::http::{Request, send};
use crate::oauth::device_code::{self, Options, Poll};
use crate::oauth::interaction::{Event, Interaction, Prompt};
use crate::oauth::{Credentials, env_url, form_encode, now_ms};

/// The provider id the model catalogue and the credential store use.
const PROVIDER: &str = "github-copilot";
/// The domain a login without an enterprise URL uses.
const DEFAULT_DOMAIN: &str = "github.com";
/// The placeholder an endpoint URL in Endpoints carries where the domain goes.
const DOMAIN: &str = "{domain}";
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const EDITOR_VERSION: &str = "vscode/1.107.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const INTEGRATION_ID: &str = "vscode-chat";
const API_VERSION: &str = "2026-06-01";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The per-attempt timeout the catalogue and policy calls give a request.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// How far before the reported expiry the Copilot token is treated as dead.
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;
const ENTERPRISE_PROMPT: &str = "GitHub Enterprise URL/domain (blank for github.com)";
const INVALID_ENTERPRISE: &str = "Invalid GitHub Enterprise URL/domain";
const ENABLING_MODELS: &str = "Enabling models...";
const INVALID_CATALOG: &str = "Invalid Copilot models response";
const INVALID_DEVICE_CODE: &str = "Invalid device code response";
const INVALID_DEVICE_FIELDS: &str = "Invalid device code response fields";
const UNTRUSTED_URI: &str = "Untrusted verification_uri in device code response";
const INVALID_DEVICE_TOKEN: &str = "Invalid device token response";
const INVALID_COPILOT_TOKEN: &str = "Invalid Copilot token response";
const INVALID_COPILOT_FIELDS: &str = "Invalid Copilot token response fields";

/// Every URL, client id and scope the flow uses. The three per-domain URLs carry a {domain}
/// placeholder, which a login fills with the enterprise domain it was given or github.com.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub client_id: String,
    pub scope: String,
    pub device_code_url: String,
    pub access_token_url: String,
    pub copilot_token_url: String,
    /// The Copilot API base for a token that names no proxy host, on an account with no
    /// enterprise domain.
    pub api_base_url: String,
    /// The Copilot API base for an enterprise account whose token names no proxy host.
    pub enterprise_api_base_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            client_id: "Iv1.b507a08c87ecfe98".to_owned(),
            scope: "read:user".to_owned(),
            device_code_url: "https://{domain}/login/device/code".to_owned(),
            access_token_url: "https://{domain}/login/oauth/access_token".to_owned(),
            copilot_token_url: "https://api.{domain}/copilot_internal/v2/token".to_owned(),
            api_base_url: "https://api.individual.githubcopilot.com".to_owned(),
            enterprise_api_base_url: "https://copilot-api.{domain}".to_owned(),
        }
    }
}

impl Endpoints {
    /// The production endpoints, with the ones a check or a gateway moves through
    /// PI_OAUTH_GITHUB_COPILOT_FIELD.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            client_id: env_url("github-copilot", "client_id", &default.client_id),
            scope: env_url("github-copilot", "scope", &default.scope),
            device_code_url: env_url(
                "github-copilot",
                "device_code_url",
                &default.device_code_url,
            ),
            access_token_url: env_url(
                "github-copilot",
                "access_token_url",
                &default.access_token_url,
            ),
            copilot_token_url: env_url(
                "github-copilot",
                "copilot_token_url",
                &default.copilot_token_url,
            ),
            api_base_url: env_url("github-copilot", "api_base_url", &default.api_base_url),
            enterprise_api_base_url: env_url(
                "github-copilot",
                "enterprise_api_base_url",
                &default.enterprise_api_base_url,
            ),
        }
    }

    /// The three endpoint URLs for a domain (upstream getUrls).
    fn urls(&self, domain: &str) -> Urls {
        Urls {
            device_code_url: self.device_code_url.replace(DOMAIN, domain),
            access_token_url: self.access_token_url.replace(DOMAIN, domain),
            copilot_token_url: self.copilot_token_url.replace(DOMAIN, domain),
        }
    }
}

struct Urls {
    device_code_url: String,
    access_token_url: String,
    copilot_token_url: String,
}

/// The API base a request to the account's Copilot endpoint starts at (upstream
/// getGitHubCopilotBaseUrl): the proxy-ep host inside the access token, else the enterprise
/// endpoint, else the default.
pub fn base_url(endpoints: &Endpoints, access: &str, stored: &Map<String, Value>) -> String {
    if let Some(host) = proxy_host(access) {
        return format!("https://{host}");
    }
    match stored_enterprise_domain(stored) {
        Some(domain) => endpoints.enterprise_api_base_url.replace(DOMAIN, &domain),
        None => endpoints.api_base_url.clone(),
    }
}

fn api_base(endpoints: &Endpoints, access: &str, enterprise_domain: Option<&str>) -> String {
    if let Some(host) = proxy_host(access) {
        return format!("https://{host}");
    }
    match enterprise_domain {
        Some(domain) => endpoints.enterprise_api_base_url.replace(DOMAIN, domain),
        None => endpoints.api_base_url.clone(),
    }
}

/// The api. host named by a token's proxy-ep field, whose format is
/// tid=...;exp=...;proxy-ep=proxy.individual.githubcopilot.com;...
fn proxy_host(token: &str) -> Option<String> {
    let start = token.find("proxy-ep=")? + "proxy-ep=".len();
    let host: String = token[start..].chars().take_while(|c| *c != ';').collect();
    if host.is_empty() {
        return None;
    }
    Some(match host.strip_prefix("proxy.") {
        Some(rest) => format!("api.{rest}"),
        None => host,
    })
}

struct Response {
    status: u16,
    retry_after: Option<Duration>,
    body: String,
}

impl Response {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn exchange(request: &Request, cancel: Option<&CancelToken>) -> Result<Response, String> {
    let response =
        send(request, cancel).map_err(|error| failed(cancel, format!("Error: {error}")))?;
    let status = response.status;
    let retry_after = response.retry_after();
    let body = response
        .text()
        .map_err(|error| failed(cancel, format!("Error: {error}")))?;
    Ok(Response {
        status,
        retry_after,
        body,
    })
}

/// A cancelled request reads as a cancelled login, the way upstream reports an abort.
fn failed(cancel: Option<&CancelToken>, message: String) -> String {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        device_code::CANCELLED.to_owned()
    } else {
        message
    }
}

/// Upstream's fetchJson: a refused answer reads as "status: body", and the body must be JSON.
fn fetch_json(request: Request, cancel: Option<&CancelToken>) -> Result<Value, String> {
    let response = exchange(&request, cancel)?;
    if !response.ok() {
        return Err(refused(&response));
    }
    serde_json::from_str(&response.body).map_err(|error| failed(cancel, error.to_string()))
}

fn refused(response: &Response) -> String {
    format!("{}: {}", response.status, response.body)
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", form_encode(name), form_encode(value)))
        .collect::<Vec<String>>()
        .join("&")
}

fn form_request(url: &str, body: String) -> Request {
    let mut request = Request::new("POST", url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", USER_AGENT);
    request.body = body.into_bytes();
    request
}

fn copilot_headers(request: Request) -> Request {
    request
        .header("User-Agent", USER_AGENT)
        .header("Editor-Version", EDITOR_VERSION)
        .header("Editor-Plugin-Version", EDITOR_PLUGIN_VERSION)
        .header("Copilot-Integration-Id", INTEGRATION_ID)
}

/// How many retries a call gets, and how long the whole call may take: upstream
/// fetchWithRateLimitRetry's policy. Zero retries or a zero budget means no retry at all.
#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    max_retries: u32,
    max_elapsed_ms: i64,
}

impl RetryPolicy {
    fn new(max_retries: u32, max_elapsed_ms: i64) -> Self {
        Self {
            max_retries,
            max_elapsed_ms,
        }
    }
}

/// Waits out a 429 the way upstream does: the server's retry-after when it sends one, else 500ms
/// doubling per attempt, giving up once the next wait would overshoot the budget.
fn fetch_with_rate_limit_retry(
    request: &Request,
    policy: RetryPolicy,
    cancel: Option<&CancelToken>,
) -> Result<Response, String> {
    let deadline = (policy.max_retries > 0 && policy.max_elapsed_ms > 0)
        .then(|| now_ms() + policy.max_elapsed_ms);
    let mut attempt = 0;
    loop {
        let mut attempt_request = request.clone();
        attempt_request.timeout = Some(ATTEMPT_TIMEOUT);
        let response = exchange(&attempt_request, cancel)?;
        if response.status != 429 || attempt == policy.max_retries {
            return Ok(response);
        }
        let mut delay_ms = 500.0 * 2f64.powi(attempt as i32);
        if let Some(server) = response.retry_after {
            delay_ms = server.as_millis() as f64;
        }
        let delay_ms = delay_ms.max(0.0);
        if let Some(deadline) = deadline
            && delay_ms >= (deadline - now_ms()) as f64
        {
            return Ok(response);
        }
        attempt += 1;
        let wait = Duration::from_millis(delay_ms as u64);
        if cancel.is_some_and(|token| token.wait_timeout(wait)) {
            return Err(device_code::CANCELLED.to_owned());
        }
    }
}

/// The hostname of an enterprise URL or bare domain, read the way new URL reads it: None when the
/// input is blank or is not a URL.
fn normalize_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let rest = match trimmed.split_once("://") {
        Some((scheme, rest)) => {
            let mut chars = scheme.chars();
            let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
            if !valid {
                return None;
            }
            rest
        }
        None => trimmed,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = match host_port.strip_prefix('[') {
        Some(inner) => inner.split(']').next().unwrap_or(""),
        None => host_port.split(':').next().unwrap_or(""),
    };
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
    valid.then(|| host.to_ascii_lowercase())
}

fn stored_enterprise_domain(stored: &Map<String, Value>) -> Option<String> {
    let enterprise = stored
        .get("enterpriseUrl")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    normalize_domain(enterprise)
}

struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval_seconds: Option<f64>,
    expires_in_seconds: i64,
}

/// The browser opens this, so only an http(s) URI with a host is allowed through.
fn verification_uri(raw: &str) -> Result<String, String> {
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| UNTRUSTED_URI.to_owned())?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return Err(UNTRUSTED_URI.to_owned());
    }
    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        return Err(UNTRUSTED_URI.to_owned());
    }
    Ok(raw.to_owned())
}

fn required_string(body: &Map<String, Value>, field: &str) -> Result<String, String> {
    body.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| INVALID_DEVICE_FIELDS.to_owned())
}

fn optional_number(body: &Map<String, Value>, field: &str) -> Result<Option<f64>, String> {
    match body.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| INVALID_DEVICE_FIELDS.to_owned()),
    }
}

fn parse_device_code(raw: &Value) -> Result<DeviceCode, String> {
    let Some(body) = raw.as_object() else {
        return Err(INVALID_DEVICE_CODE.to_owned());
    };
    Ok(DeviceCode {
        device_code: required_string(body, "device_code")?,
        user_code: required_string(body, "user_code")?,
        verification_uri: verification_uri(&required_string(body, "verification_uri")?)?,
        interval_seconds: optional_number(body, "interval")?,
        expires_in_seconds: body
            .get("expires_in")
            .and_then(Value::as_f64)
            .ok_or_else(|| INVALID_DEVICE_FIELDS.to_owned())? as i64,
    })
}

fn request_device_code(
    endpoints: &Endpoints,
    urls: &Urls,
    cancel: &CancelToken,
) -> Result<DeviceCode, String> {
    let body = form_body(&[
        ("client_id", endpoints.client_id.as_str()),
        ("scope", endpoints.scope.as_str()),
    ]);
    let raw = fetch_json(form_request(&urls.device_code_url, body), Some(cancel))?;
    parse_device_code(&raw)
}

fn poll_for_access_token(
    endpoints: &Endpoints,
    urls: &Urls,
    device: &DeviceCode,
    cancel: &CancelToken,
) -> Result<String, String> {
    let options = Options::new(device.interval_seconds, Some(device.expires_in_seconds))
        .waiting_before_first_poll();
    device_code::poll(&options, cancel, || {
        let body = form_body(&[
            ("client_id", endpoints.client_id.as_str()),
            ("device_code", device.device_code.as_str()),
            ("grant_type", DEVICE_GRANT),
        ]);
        let raw = fetch_json(form_request(&urls.access_token_url, body), Some(cancel))?;
        let Some(token) = raw.as_object() else {
            return Ok(Poll::Failed(INVALID_DEVICE_TOKEN.to_owned()));
        };
        if let Some(access) = token.get("access_token").and_then(Value::as_str) {
            return Ok(Poll::Complete(access.to_owned()));
        }
        Ok(match token.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => Poll::Pending,
            Some("slow_down") => Poll::SlowDown(token.get("interval").and_then(Value::as_f64)),
            Some(error) => Poll::Failed(format!(
                "Device flow failed: {error}{}",
                description_suffix(token)
            )),
            None => Poll::Failed(INVALID_DEVICE_TOKEN.to_owned()),
        })
    })
}

fn description_suffix(body: &Map<String, Value>) -> String {
    match body.get("error_description").and_then(Value::as_str) {
        Some(description) if !description.is_empty() => format!(": {description}"),
        _ => String::new(),
    }
}

/// Trades the GitHub access token for a Copilot token (upstream refreshGitHubCopilotAccessToken).
fn copilot_credentials(
    urls: &Urls,
    github_access: &str,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let request = copilot_headers(Request::new("GET", urls.copilot_token_url.as_str()))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {github_access}"));
    let raw = fetch_json(request, cancel)?;
    let Some(body) = raw.as_object() else {
        return Err(INVALID_COPILOT_TOKEN.to_owned());
    };
    let access = body
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| INVALID_COPILOT_FIELDS.to_owned())?;
    let expires_at = body
        .get("expires_at")
        .and_then(Value::as_f64)
        .ok_or_else(|| INVALID_COPILOT_FIELDS.to_owned())?;
    Ok(Credentials::new(
        github_access.to_owned(),
        access.to_owned(),
        (expires_at * 1000.0) as i64 - REFRESH_SKEW_MS,
    ))
}

struct Catalog {
    available_model_ids: Vec<String>,
    policy_model_ids: Vec<String>,
}

struct CatalogItem {
    id: String,
    picker_enabled: bool,
    policy_state: Option<String>,
}

/// The ids the shipped catalogue lists for this provider: upstream only asks for a policy on
/// models pi knows.
fn known_model_ids() -> Vec<String> {
    let models: Vec<Value> = serde_json::from_str(include_str!("../catalog/models.json"))
        .unwrap_or_else(|error| panic!("catalog models.json: not a JSON array: {error}"));
    models
        .iter()
        .filter(|model| model.get("provider").and_then(Value::as_str) == Some(PROVIDER))
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Reads the account's model catalogue (upstream parseGitHubCopilotModelCatalog): what the picker
/// may offer, and what has a policy this login should enable.
fn parse_catalog(raw: &Value, allow_policy_fallback: bool) -> Result<Catalog, String> {
    let Some(Value::Array(items)) = raw.as_object().and_then(|body| body.get("data")) else {
        return Err(INVALID_CATALOG.to_owned());
    };
    let account: Vec<CatalogItem> = items
        .iter()
        .filter_map(|item| {
            let item = item.as_object()?;
            let id = item.get("id").and_then(Value::as_str)?;
            let supports = item
                .get("capabilities")
                .and_then(Value::as_object)
                .and_then(|capabilities| capabilities.get("supports"))
                .and_then(Value::as_object);
            if supports
                .and_then(|supports| supports.get("tool_calls"))
                .and_then(Value::as_bool)
                == Some(false)
            {
                return None;
            }
            Some(CatalogItem {
                id: id.to_owned(),
                picker_enabled: item.get("model_picker_enabled").and_then(Value::as_bool)
                    == Some(true),
                policy_state: item
                    .get("policy")
                    .and_then(Value::as_object)
                    .and_then(|policy| policy.get("state"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect();
    let picker: Vec<&CatalogItem> = account
        .iter()
        .filter(|model| model.picker_enabled && model.policy_state.as_deref() != Some("disabled"))
        .collect();
    let use_policy_fallback = allow_policy_fallback && picker.is_empty();
    let available = if !picker.is_empty() || !allow_policy_fallback {
        picker
    } else {
        account
            .iter()
            .filter(|model| model.policy_state.as_deref() == Some("enabled"))
            .collect()
    };
    let known = known_model_ids();
    let policy: Vec<&CatalogItem> = account
        .iter()
        .filter(|model| {
            model.policy_state.as_deref() == Some("unconfigured")
                && known.iter().any(|id| id == &model.id)
                && (model.picker_enabled || use_policy_fallback)
        })
        .collect();
    Ok(Catalog {
        available_model_ids: available.iter().map(|model| model.id.clone()).collect(),
        policy_model_ids: policy.iter().map(|model| model.id.clone()).collect(),
    })
}

/// Reads the account's model catalogue (upstream fetchGitHubCopilotModels).
fn fetch_models(
    endpoints: &Endpoints,
    access: &str,
    enterprise_domain: Option<&str>,
    policy: RetryPolicy,
    cancel: Option<&CancelToken>,
) -> Result<Catalog, String> {
    let base = api_base(endpoints, access, enterprise_domain);
    let request = copilot_headers(Request::new("GET", format!("{base}/models")))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {access}"))
        .header("X-GitHub-Api-Version", API_VERSION);
    let response = fetch_with_rate_limit_retry(&request, policy, cancel)?;
    if !response.ok() {
        return Err(refused(&response));
    }
    let raw: Value = serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
    parse_catalog(&raw, base == endpoints.api_base_url)
}

/// Enables the models a login asked for, stopping the batch when the account is rate limited
/// (upstream enableGitHubCopilotModels). Each policy update is best effort.
fn enable_models(
    endpoints: &Endpoints,
    access: &str,
    model_ids: &[String],
    enterprise_domain: Option<&str>,
    cancel: &CancelToken,
) -> Result<Vec<String>, String> {
    let base = api_base(endpoints, access, enterprise_domain);
    let mut enabled = Vec::new();
    for model_id in model_ids {
        let request = copilot_headers(Request::new(
            "POST",
            format!("{base}/models/{model_id}/policy"),
        ))
        .header("Authorization", format!("Bearer {access}"))
        .header("openai-intent", "chat-policy")
        .header("x-interaction-type", "chat-policy")
        .json(&json!({ "state": "enabled" }));
        match fetch_with_rate_limit_retry(&request, RetryPolicy::new(2, 5000), Some(cancel)) {
            Ok(response) => {
                if response.status == 429 {
                    break;
                }
                if response.ok() {
                    enabled.push(model_id.clone());
                }
            }
            Err(error) => {
                if cancel.is_cancelled() {
                    return Err(error);
                }
            }
        }
    }
    Ok(enabled)
}

/// The credential upstream returns: its token fields, the enterprise domain when there is one, and
/// the model ids the account can use.
fn with_models(
    credentials: &Credentials,
    enterprise_domain: Option<&str>,
    available: &[String],
    enabled: &[String],
) -> Credentials {
    let mut ids: Vec<String> = Vec::new();
    for id in available.iter().chain(enabled) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let mut credentials = credentials.clone();
    if let Some(domain) = enterprise_domain {
        credentials = credentials.with("enterpriseUrl", Value::String(domain.to_owned()));
    }
    credentials.with(
        "availableModelIds",
        Value::Array(ids.iter().cloned().map(Value::String).collect()),
    )
}

/// Asks for the enterprise domain, then signs in with GitHub's device code and exchanges the
/// result for a Copilot token (upstream loginGitHubCopilot).
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let input = interaction
        .prompt(
            Prompt::Text {
                message: ENTERPRISE_PROMPT.to_owned(),
            },
            cancel,
        )?
        .ok_or_else(|| device_code::CANCELLED.to_owned())?;
    if cancel.is_cancelled() {
        return Err(device_code::CANCELLED.to_owned());
    }
    let enterprise_domain = normalize_domain(&input);
    if !input.trim().is_empty() && enterprise_domain.is_none() {
        return Err(INVALID_ENTERPRISE.to_owned());
    }
    let urls = endpoints.urls(enterprise_domain.as_deref().unwrap_or(DEFAULT_DOMAIN));
    let device = request_device_code(endpoints, &urls, cancel)?;
    interaction.notify(Event::DeviceCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        interval_seconds: device
            .interval_seconds
            .map_or(0, |seconds| seconds.floor() as u64),
        expires_in_seconds: device.expires_in_seconds.max(0) as u64,
    });
    let github_access = poll_for_access_token(endpoints, &urls, &device, cancel)?;
    let credentials = copilot_credentials(&urls, &github_access, Some(cancel))?;
    let models = fetch_models(
        endpoints,
        &credentials.access,
        enterprise_domain.as_deref(),
        RetryPolicy::new(2, 5000),
        Some(cancel),
    )?;
    let mut enabled: Vec<String> = Vec::new();
    if !models.policy_model_ids.is_empty() {
        interaction.notify(Event::Progress(ENABLING_MODELS.to_owned()));
        enabled = enable_models(
            endpoints,
            &credentials.access,
            &models.policy_model_ids,
            enterprise_domain.as_deref(),
            cancel,
        )?;
    }
    Ok(with_models(
        &credentials,
        enterprise_domain.as_deref(),
        &models.available_model_ids,
        &enabled,
    ))
}

/// Exchanges the stored GitHub token for a fresh Copilot token and re-reads the model catalogue
/// (upstream refreshGitHubCopilotToken).
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let refresh_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let enterprise_domain = stored_enterprise_domain(stored);
    let urls = endpoints.urls(enterprise_domain.as_deref().unwrap_or(DEFAULT_DOMAIN));
    let credentials = copilot_credentials(&urls, refresh_token, cancel)?;
    let models = fetch_models(
        endpoints,
        &credentials.access,
        enterprise_domain.as_deref(),
        RetryPolicy::new(0, 0),
        cancel,
    )?;
    Ok(with_models(
        &credentials,
        enterprise_domain.as_deref(),
        &models.available_model_ids,
        &[],
    ))
}
