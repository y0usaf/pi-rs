//! ai/src/auth/oauth/xai.ts: xAI's OAuth device-code login.

use serde_json::{Map, Value};

use crate::oauth::device_code::{self, Options, Poll};
use crate::oauth::interaction::{Event, Interaction};
use crate::oauth::{Credentials, env_url, now_ms, post_form};
use pi_rt::cancel::CancelToken;

/// How far before the reported expiry the token is treated as dead, so it cannot die mid-request.
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;
const DEFAULT_TOKEN_LIFETIME_SECONDS: f64 = 3600.0;
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const UNTRUSTED_URI: &str = "Untrusted verification URI in xAI OAuth response";

/// Every URL, client id and scope the flow uses.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub client_id: String,
    pub scope: String,
    pub device_code_url: String,
    pub token_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            client_id: "b1a00492-073a-47ea-816f-4c329264a828".to_owned(),
            scope: "openid profile email offline_access grok-cli:access api:access".to_owned(),
            device_code_url: "https://auth.x.ai/oauth2/device/code".to_owned(),
            token_url: "https://auth.x.ai/oauth2/token".to_owned(),
        }
    }
}

impl Endpoints {
    /// The production endpoints, with the ones a check or a gateway moves through
    /// `PI_OAUTH_XAI_<FIELD>`.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            device_code_url: env_url("xai", "device_code_url", &default.device_code_url),
            token_url: env_url("xai", "token_url", &default.token_url),
            ..default
        }
    }
}

struct Response {
    status: u16,
    body: Map<String, Value>,
}

impl Response {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A form POST whose body is read as a JSON object; a body that is not an object is empty.
fn post(
    url: &str,
    fields: &[(&str, &str)],
    cancel: Option<&CancelToken>,
) -> Result<Response, String> {
    let (status, text) =
        post_form(url, fields, cancel).map_err(|error| cancelled(cancel, error))?;
    let body = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(body)) => body,
        Ok(_) => Map::new(),
        Err(_) => {
            return Err(cancelled(
                cancel,
                format!("xAI OAuth returned invalid JSON (HTTP {status})"),
            ));
        }
    };
    Ok(Response { status, body })
}

/// A cancelled request reads as a cancelled login, the way upstream reports it.
fn cancelled(cancel: Option<&CancelToken>, message: String) -> String {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        device_code::CANCELLED.to_owned()
    } else {
        message
    }
}

fn request_failure(action: &str, response: &Response) -> String {
    let detail: Vec<&str> = ["error", "error_description"]
        .iter()
        .filter_map(|field| response.body.get(*field).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .collect();
    let mut message = format!("xAI OAuth {action} failed (HTTP {})", response.status);
    if !detail.is_empty() {
        message.push_str(&format!(": {}", detail.join(": ")));
    }
    message
}

fn required_string(body: &Map<String, Value>, field: &str) -> Result<String, String> {
    body.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("Invalid xAI OAuth response field: {field}"))
}

fn positive_number(body: &Map<String, Value>, field: &str) -> Result<f64, String> {
    body.get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| format!("Invalid xAI OAuth response field: {field}"))
}

/// The browser opens this, so only an https URI with a host is allowed through.
fn verification_uri(raw: &str) -> Result<String, String> {
    let rest = raw
        .split_once("://")
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        .map(|(_, rest)| rest)
        .ok_or_else(|| UNTRUSTED_URI.to_owned())?;
    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        return Err(UNTRUSTED_URI.to_owned());
    }
    Ok(raw.to_owned())
}

struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    interval_seconds: Option<f64>,
    expires_in_seconds: f64,
}

fn parse_device_code(body: &Map<String, Value>) -> Result<DeviceCode, String> {
    let interval_seconds = body
        .get("interval")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0);
    let verification_uri_complete = body
        .get("verification_uri_complete")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(verification_uri)
        .transpose()?;
    Ok(DeviceCode {
        device_code: required_string(body, "device_code")?,
        user_code: required_string(body, "user_code")?,
        verification_uri: verification_uri(&required_string(body, "verification_uri")?)?,
        verification_uri_complete,
        interval_seconds,
        expires_in_seconds: positive_number(body, "expires_in")?,
    })
}

fn credentials(
    body: &Map<String, Value>,
    previous_refresh: Option<&str>,
) -> Result<Credentials, String> {
    let access = required_string(body, "access_token")?;
    let refresh = match (body.get("refresh_token"), previous_refresh) {
        (None, Some(previous)) => previous.to_owned(),
        _ => required_string(body, "refresh_token")?,
    };
    let expires_in = match body.get("expires_in") {
        None => DEFAULT_TOKEN_LIFETIME_SECONDS,
        Some(_) => positive_number(body, "expires_in")?,
    };
    Ok(Credentials::new(
        refresh,
        access,
        now_ms() + (expires_in * 1000.0) as i64 - REFRESH_SKEW_MS,
    ))
}

fn request_device_code(endpoints: &Endpoints, cancel: &CancelToken) -> Result<DeviceCode, String> {
    let response = post(
        &endpoints.device_code_url,
        &[
            ("client_id", endpoints.client_id.as_str()),
            ("scope", endpoints.scope.as_str()),
            ("referrer", "pi"),
        ],
        Some(cancel),
    )?;
    if !response.ok() {
        return Err(request_failure("device authorization", &response));
    }
    parse_device_code(&response.body)
}

fn poll_for_tokens(
    endpoints: &Endpoints,
    device: &DeviceCode,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let options = Options::new(
        device.interval_seconds,
        Some(device.expires_in_seconds as i64),
    )
    .waiting_before_first_poll();
    device_code::poll(&options, cancel, || {
        let response = post(
            &endpoints.token_url,
            &[
                ("grant_type", DEVICE_GRANT),
                ("client_id", endpoints.client_id.as_str()),
                ("device_code", device.device_code.as_str()),
            ],
            Some(cancel),
        )?;
        if response.ok() {
            return credentials(&response.body, None).map(Poll::Complete);
        }
        Ok(match response.body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => Poll::Pending,
            Some("slow_down") => {
                Poll::SlowDown(response.body.get("interval").and_then(Value::as_f64))
            }
            Some("access_denied" | "authorization_denied") => {
                Poll::Failed("xAI device authorization was denied".to_owned())
            }
            Some("expired_token") => Poll::Failed("xAI device code expired".to_owned()),
            _ => Poll::Failed(request_failure("device token polling", &response)),
        })
    })
}

/// Shows the user code, then waits for the browser to complete the sign-in.
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let device = request_device_code(endpoints, cancel)?;
    interaction.notify(Event::DeviceCode {
        user_code: device.user_code.clone(),
        verification_uri: device
            .verification_uri_complete
            .clone()
            .unwrap_or_else(|| device.verification_uri.clone()),
        interval_seconds: device
            .interval_seconds
            .map_or(0, |seconds| seconds.floor() as u64),
        expires_in_seconds: device.expires_in_seconds as u64,
    });
    poll_for_tokens(endpoints, &device, cancel)
}

/// Exchanges a stored refresh token for a new access token.
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let refresh_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let response = post(
        &endpoints.token_url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", endpoints.client_id.as_str()),
            ("refresh_token", refresh_token),
        ],
        cancel,
    )?;
    if !response.ok() {
        return Err(request_failure("token refresh", &response));
    }
    credentials(&response.body, Some(refresh_token))
}
