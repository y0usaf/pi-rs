//! ai/src/auth/oauth/kimi-coding.ts: the Kimi Code subscription's RFC 8628 device-code login.

use std::time::Duration;

use serde_json::{Map, Value};

use crate::oauth::device_code::{self, Options, Poll};
use crate::oauth::interaction::{Event, Interaction};
use crate::oauth::{Credentials, env_url, now_ms, post_form};
use pi_rt::cancel::CancelToken;

const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";
const DEVICE_CODE_TIMEOUT_SECONDS: f64 = 15.0 * 60.0;
const DEFAULT_POLL_INTERVAL_SECONDS: f64 = 5.0;
const REFRESH_MAX_RETRIES: u32 = 3;
const REFRESH_BACKOFF_MS: u64 = 1000;
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const HOST_ENV: [&str; 2] = ["KIMI_CODE_OAUTH_HOST", "KIMI_OAUTH_HOST"];
const REFRESH_ABORTED: &str = "Kimi Code token refresh aborted";

/// Where the Kimi Code flow talks to: one OAuth host, the client id, and the two paths under it.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub client_id: String,
    pub device_authorization_url: String,
    pub token_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self::at(DEFAULT_OAUTH_HOST)
    }
}

impl Endpoints {
    /// The production endpoints, with the host a gateway moves through
    /// `PI_OAUTH_KIMI_CODING_OAUTH_HOST` or through upstream's own `KIMI_CODE_OAUTH_HOST` and
    /// `KIMI_OAUTH_HOST`.
    pub fn from_env() -> Self {
        Self::at(&env_url("kimi-coding", "oauth_host", &provider_host()))
    }

    fn at(host: &str) -> Self {
        let host = host.trim_end_matches('/');
        Self {
            client_id: CLIENT_ID.to_owned(),
            device_authorization_url: format!("{host}/api/oauth/device_authorization"),
            token_url: format!("{host}/api/oauth/token"),
        }
    }
}

fn provider_host() -> String {
    HOST_ENV
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_OAUTH_HOST.to_owned())
}

struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    interval_seconds: f64,
    expires_in_seconds: f64,
}

fn read_json(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(body)) => Some(body),
        _ => None,
    }
}

fn json_text(body: Option<&Map<String, Value>>) -> String {
    body.map_or_else(
        || "null".to_owned(),
        |body| serde_json::to_string(body).unwrap_or_else(|_| "null".to_owned()),
    )
}

/// The colon-joined text upstream appends to a failure when the body is not empty.
fn detail(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

fn error_description(body: Option<&Map<String, Value>>) -> String {
    body.and_then(|body| body.get("error_description"))
        .and_then(Value::as_str)
        .map_or_else(String::new, |description| format!(": {description}"))
}

fn positive_number(body: &Map<String, Value>, name: &str) -> Option<f64> {
    body.get(name)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
}

/// The browser opens this, so only an http(s) URL with a host is trusted.
fn trusted_http_url(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        return None;
    }
    Some(value.to_owned())
}

/// A cancelled request reads as a cancelled login, the way upstream reports it.
fn cancelled(cancel: Option<&CancelToken>, message: String) -> String {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        device_code::CANCELLED.to_owned()
    } else {
        message
    }
}

fn credentials(body: Option<&Map<String, Value>>, operation: &str) -> Result<Credentials, String> {
    let field = |name: &str| {
        body.and_then(|body| body.get(name))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    let expires_in = body
        .and_then(|body| body.get("expires_in"))
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0);
    match (field("access_token"), field("refresh_token"), expires_in) {
        (Some(access), Some(refresh), Some(expires_in)) => Ok(Credentials::new(
            refresh.to_owned(),
            access.to_owned(),
            now_ms() + (expires_in * 1000.0) as i64,
        )),
        _ => Err(format!(
            "Kimi Code token {operation} response missing fields: {}",
            json_text(body)
        )),
    }
}

fn device_authorization(body: Option<&Map<String, Value>>) -> Result<DeviceAuthorization, String> {
    let invalid = || {
        format!(
            "Invalid Kimi Code device authorization response: {}",
            json_text(body)
        )
    };
    let Some(body) = body else {
        return Err(invalid());
    };
    let required = |name: &str| body.get(name).and_then(Value::as_str);
    let (
        Some(device_code),
        Some(user_code),
        Some(verification_uri),
        Some(verification_uri_complete),
    ) = (
        required("device_code"),
        required("user_code"),
        required("verification_uri"),
        required("verification_uri_complete"),
    )
    else {
        return Err(invalid());
    };
    let verification_uri_complete =
        trusted_http_url(verification_uri_complete).ok_or_else(invalid)?;
    trusted_http_url(verification_uri).ok_or_else(invalid)?;
    Ok(DeviceAuthorization {
        device_code: device_code.to_owned(),
        user_code: user_code.to_owned(),
        verification_uri_complete,
        interval_seconds: positive_number(body, "interval")
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS),
        expires_in_seconds: positive_number(body, "expires_in")
            .unwrap_or(DEVICE_CODE_TIMEOUT_SECONDS),
    })
}

fn start_device_authorization(
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<DeviceAuthorization, String> {
    let (status, text) = post_form(
        &endpoints.device_authorization_url,
        &[("client_id", endpoints.client_id.as_str())],
        Some(cancel),
    )
    .map_err(|error| cancelled(Some(cancel), error))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "Kimi Code device authorization failed with status {status}{}",
            detail(&text)
        ));
    }
    device_authorization(read_json(&text).as_ref())
}

fn poll_for_token(
    endpoints: &Endpoints,
    device: &DeviceAuthorization,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let options = Options::new(
        Some(device.interval_seconds),
        Some(device.expires_in_seconds as i64),
    )
    .waiting_before_first_poll();
    device_code::poll(&options, cancel, || {
        let (status, text) = post_form(
            &endpoints.token_url,
            &[
                ("client_id", endpoints.client_id.as_str()),
                ("device_code", device.device_code.as_str()),
                ("grant_type", DEVICE_GRANT),
            ],
            Some(cancel),
        )
        .map_err(|error| cancelled(Some(cancel), error))?;
        if status >= 500 {
            return Ok(Poll::Failed(format!(
                "Kimi Code device token request failed with status {status}{}",
                detail(&text)
            )));
        }
        let body = read_json(&text);
        if (200..300).contains(&status)
            && body
                .as_ref()
                .is_some_and(|body| body.get("access_token").and_then(Value::as_str).is_some())
        {
            return credentials(body.as_ref(), "poll").map(Poll::Complete);
        }
        let error = body
            .as_ref()
            .and_then(|body| body.get("error"))
            .and_then(Value::as_str);
        Ok(match error {
            Some("authorization_pending") => Poll::Pending,
            Some("slow_down") => Poll::SlowDown(
                body.as_ref()
                    .and_then(|body| positive_number(body, "interval")),
            ),
            Some("expired_token") => Poll::Failed(
                "Kimi Code device authorization expired. Please restart login.".to_owned(),
            ),
            Some("access_denied") => Poll::Failed("Kimi Code login was denied.".to_owned()),
            Some(error) => Poll::Failed(format!(
                "Kimi Code device token request failed (status {status}): {error}{}",
                error_description(body.as_ref())
            )),
            None => Poll::Failed(format!(
                "Kimi Code device token request failed (status {status})"
            )),
        })
    })
}

/// Shows the user code, then polls until the browser completes the sign-in or the code expires.
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let device = start_device_authorization(endpoints, cancel)?;
    interaction.notify(Event::DeviceCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri_complete.clone(),
        interval_seconds: device.interval_seconds as u64,
        expires_in_seconds: device.expires_in_seconds as u64,
    });
    poll_for_token(endpoints, &device, cancel)
}

/// Exchanges a stored refresh token, retrying the retryable statuses with a doubling backoff.
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let refresh_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut last_error: Option<String> = None;
    for attempt in 0..=REFRESH_MAX_RETRIES {
        if attempt > 0 && wait(REFRESH_BACKOFF_MS << (attempt - 1), cancel) {
            return Err(REFRESH_ABORTED.to_owned());
        }
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(REFRESH_ABORTED.to_owned());
        }
        let response = post_form(
            &endpoints.token_url,
            &[
                ("client_id", endpoints.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
            cancel,
        );
        let (status, text) = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let body = read_json(&text);
        if (200..300).contains(&status) {
            return credentials(body.as_ref(), "refresh");
        }
        let error = body
            .as_ref()
            .and_then(|body| body.get("error"))
            .and_then(Value::as_str);
        if status == 401 || status == 403 || error == Some("invalid_grant") {
            return Err(format!(
                "Kimi Code token refresh unauthorized (status {status}){}",
                error_description(body.as_ref())
            ));
        }
        if (status == 429 || status >= 500) && attempt < REFRESH_MAX_RETRIES {
            last_error = Some(format!(
                "Kimi Code token refresh failed with status {status}"
            ));
            continue;
        }
        return Err(format!(
            "Kimi Code token refresh failed with status {status}{}",
            detail(&json_text(body.as_ref()))
        ));
    }
    Err(last_error.unwrap_or_else(|| "Kimi Code token refresh failed".to_owned()))
}

/// Waits, and answers whether the user cancelled meanwhile.
fn wait(millis: u64, cancel: Option<&CancelToken>) -> bool {
    let duration = Duration::from_millis(millis);
    match cancel {
        Some(cancel) => cancel.wait_timeout(duration),
        None => {
            std::thread::sleep(duration);
            false
        }
    }
}
