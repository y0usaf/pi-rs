//! ai/src/auth/oauth/meta.ts: Meta's device-code sign-in, and the Model API key it mints.

use pi_rt::cancel::CancelToken;
use serde_json::{Map, Value, json};

use crate::http::{Request, send};
use crate::oauth::device_code::{self, Options, Poll};
use crate::oauth::interaction::{Event, Interaction};
use crate::oauth::{Credentials, REQUEST_TIMEOUT, env_url, now_ms, post_form};

/// A minted key lives about a day.
const API_KEY_LIFETIME_MS: i64 = 24 * 60 * 60 * 1000;
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Every URL and client id the flow uses.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub client_id: String,
    pub device_authorization_url: String,
    pub device_token_url: String,
    pub api_key_mint_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            client_id: "1031625952748946".to_owned(),
            device_authorization_url: "https://auth.meta.com/oidc/device/authorization/".to_owned(),
            device_token_url: "https://auth.meta.com/oidc/device/token/".to_owned(),
            api_key_mint_url: "https://api.meta.ai/muse-code/key".to_owned(),
        }
    }
}

impl Endpoints {
    /// The production endpoints, with the ones a check or a gateway moves through
    /// `PI_OAUTH_META_<FIELD>`.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            device_authorization_url: env_url(
                "meta",
                "device_authorization_url",
                &default.device_authorization_url,
            ),
            device_token_url: env_url("meta", "device_token_url", &default.device_token_url),
            api_key_mint_url: env_url("meta", "api_key_mint_url", &default.api_key_mint_url),
            ..default
        }
    }
}

/// An HTTP answer: its status, and the body when the body is a JSON object.
struct Response {
    status: u16,
    body: Option<Map<String, Value>>,
}

impl Response {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn detail(&self) -> String {
        detail(self.body.as_ref())
    }

    fn text(&self) -> String {
        match &self.body {
            Some(body) => Value::Object(body.clone()).to_string(),
            None => "null".to_owned(),
        }
    }
}

/// A body that is not a JSON object reads as no body at all, the way upstream's `readJson` does.
fn parse(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(body)) => Some(body),
        _ => None,
    }
}

/// The readable reason a failed answer carries, if it carries one.
fn detail(body: Option<&Map<String, Value>>) -> String {
    for key in ["error_description", "detail", "message", "error"] {
        let Some(value) = body
            .and_then(|body| body.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        return format!(": {value}");
    }
    String::new()
}

/// A cancelled request reads as a cancelled login, the way upstream reports it.
fn cancelled(cancel: Option<&CancelToken>, message: String) -> String {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        device_code::CANCELLED.to_owned()
    } else {
        message
    }
}

/// The browser opens this, so only an http(s) URL is trusted.
fn trusted_http_url(value: Option<&Value>) -> Option<String> {
    let value = value?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)?;
    let (scheme, rest) = value.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        return None;
    }
    Some(value)
}

fn positive_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number > 0.0)
}

fn field<'a>(body: &'a Option<Map<String, Value>>, name: &str) -> Option<&'a Value> {
    body.as_ref().and_then(|body| body.get(name))
}

fn text_field(body: &Option<Map<String, Value>>, name: &str) -> Option<String> {
    field(body, name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval_seconds: Option<f64>,
    expires_in_seconds: Option<f64>,
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
    let body = parse(&text);
    let response = Response { status, body };
    if !response.ok() {
        return Err(format!(
            "Meta device authorization failed with status {}{}",
            response.status,
            response.detail()
        ));
    }
    let verification_uri = trusted_http_url(field(&response.body, "verification_uri_complete"))
        .or_else(|| trusted_http_url(field(&response.body, "verification_uri")));
    let (Some(device_code), Some(user_code), Some(verification_uri)) = (
        text_field(&response.body, "device_code"),
        text_field(&response.body, "user_code"),
        verification_uri,
    ) else {
        return Err(format!(
            "Invalid Meta device authorization response: {}",
            response.text()
        ));
    };
    Ok(DeviceAuthorization {
        device_code,
        user_code,
        verification_uri,
        interval_seconds: positive_number(field(&response.body, "interval")),
        expires_in_seconds: positive_number(field(&response.body, "expires_in")),
    })
}

fn poll_for_identity_token(
    endpoints: &Endpoints,
    device: &DeviceAuthorization,
    cancel: &CancelToken,
) -> Result<String, String> {
    let options = Options::new(
        device.interval_seconds,
        device.expires_in_seconds.map(|seconds| seconds as i64),
    )
    .waiting_before_first_poll();
    device_code::poll(&options, cancel, || {
        let (status, text) = post_form(
            &endpoints.device_token_url,
            &[
                ("grant_type", DEVICE_GRANT),
                ("device_code", device.device_code.as_str()),
                ("client_id", endpoints.client_id.as_str()),
            ],
            Some(cancel),
        )
        .map_err(|error| cancelled(Some(cancel), error))?;
        let response = Response {
            status,
            body: parse(&text),
        };
        if response.ok()
            && let Some(access_token) = text_field(&response.body, "access_token")
        {
            return Ok(Poll::Complete(access_token));
        }
        Ok(
            match field(&response.body, "error").and_then(Value::as_str) {
                Some("authorization_pending") => Poll::Pending,
                Some("slow_down") => {
                    Poll::SlowDown(positive_number(field(&response.body, "interval")))
                }
                Some("access_denied") => Poll::Failed("Meta login was denied.".to_owned()),
                Some("expired_token") => Poll::Failed(
                    "Meta device authorization expired. Please restart login.".to_owned(),
                ),
                _ => Poll::Failed(format!(
                    "Meta device token request failed with status {}{}",
                    response.status,
                    response.detail()
                )),
            },
        )
    })
}

/// Exchanges an identity token for a Model API key. Keys are valid for about a day.
fn mint_api_key(
    identity_token: &str,
    endpoints: &Endpoints,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let request = Request::new("POST", endpoints.api_key_mint_url.as_str())
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {identity_token}"))
        .json(&json!({}))
        .header("x-api-version", "1.0.0")
        .timeout(Some(REQUEST_TIMEOUT));
    let response =
        send(&request, cancel).map_err(|error| cancelled(cancel, format!("Error: {error}")))?;
    let status = response.status;
    let text = response
        .text()
        .map_err(|error| cancelled(cancel, format!("Error: {error}")))?;
    let response = Response {
        status,
        body: parse(&text),
    };
    if response.status == 401 || response.status == 403 {
        return Err(format!(
            "Meta session expired (status {}). Run `/login meta` to sign in again.{}",
            response.status,
            response.detail()
        ));
    }
    if !response.ok() {
        return Err(format!(
            "Meta API key mint failed with status {}{}",
            response.status,
            response.detail()
        ));
    }
    let Some(api_key) = text_field(&response.body, "api_key") else {
        return Err(
            match trusted_http_url(field(&response.body, "action_url")) {
                Some(url) => format!("Meta did not issue an API key. Complete setup at {url}"),
                None => "Meta did not issue an API key.".to_owned(),
            },
        );
    };
    Ok(Credentials::new(
        identity_token.to_owned(),
        api_key,
        now_ms() + API_KEY_LIFETIME_MS,
    ))
}

/// Shows the user code, then polls until the browser completes the sign-in, then mints the key.
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let device = start_device_authorization(endpoints, cancel)?;
    interaction.notify(Event::DeviceCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        interval_seconds: device
            .interval_seconds
            .map_or(0, |seconds| seconds.floor() as u64),
        expires_in_seconds: device
            .expires_in_seconds
            .map_or(0, |seconds| seconds.floor() as u64),
    });
    let identity_token = poll_for_identity_token(endpoints, &device, cancel)?;
    interaction.notify(Event::Progress(
        "Enabling Meta Model API access...".to_owned(),
    ));
    mint_api_key(&identity_token, endpoints, Some(cancel))
}

/// Mints a fresh key from the stored identity token. The identity token itself is not renewable.
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let identity_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    mint_api_key(identity_token, endpoints, cancel)
}
