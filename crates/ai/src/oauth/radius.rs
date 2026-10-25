//! ai/src/auth/oauth/radius.ts: Radius's gateway sign-in, by browser or device code. Upstream's
//! offer to set up MCP access during login is not ported.

use pi_rt::cancel::CancelToken;
use serde_json::{Map, Value};

use crate::oauth::callback::Listener;
use crate::oauth::device_code::{self, Options, Poll};
use crate::oauth::interaction::{Event, Interaction, Prompt};
use crate::oauth::pkce::{self, Pkce};
use crate::oauth::{Credentials, env_url, form_encode, get_json, now_ms, post_form};

const NAME: &str = "Radius";
const BROWSER: &str = "browser";
const DEVICE_CODE: &str = "device-code";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const CALLBACK_HOST: &str = "127.0.0.1";
const CALLBACK_PORT: u16 = 1456;
const CALLBACK_PATH: &str = "/oauth/callback";
const BROWSER_INSTRUCTIONS: &str = "Continue in your browser.";
const MISSING_DEVICE_FIELDS: &str =
    "Radius OAuth device authorization response is missing required fields";
const MISSING_TOKEN_FIELDS: &str = "Radius OAuth token response is missing required fields";

/// How far before the reported expiry the token is treated as dead, so it cannot die mid-request.
const TOKEN_EXPIRY_SKEW_MS: i64 = 60_000;

/// Where the login talks to. The gateway serves the client endpoints itself; only the browser
/// authorization endpoint is discovered.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub gateway: String,
    pub client_id: String,
    pub scope: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            gateway: "https://radius.pi.dev".to_owned(),
            client_id: "pi-gateway".to_owned(),
            scope: "gateway offline_access".to_owned(),
        }
    }
}

impl Endpoints {
    /// The gateway upstream reads from `PI_RADIUS_GATEWAY`, else the default, normalised; a check
    /// moves it with `PI_OAUTH_RADIUS_GATEWAY`, which wins over both.
    pub fn from_env() -> Self {
        let default = Self::default();
        let production = std::env::var("PI_RADIUS_GATEWAY")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.gateway.clone());
        Self {
            gateway: normalize_gateway(&env_url("radius", "gateway", &production)),
            ..default
        }
    }

    /// A client endpoint on the gateway. Upstream resolves each path against the gateway, so a path
    /// or query on the gateway itself is dropped.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", origin(&self.gateway))
    }

    /// The loopback URI the browser comes back to on `port`.
    pub fn redirect_uri(&self, port: u16) -> String {
        format!("http://{CALLBACK_HOST}:{port}{CALLBACK_PATH}")
    }

    /// The URL the user opens. Upstream replaces a query on the authorization endpoint with these
    /// parameters.
    pub fn authorize_link(
        &self,
        authorization_endpoint: &str,
        redirect_uri: &str,
        pair: &Pkce,
        state: &str,
    ) -> String {
        let params = [
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", self.scope.as_str()),
            ("code_challenge", pair.challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("handoff", "url"),
            ("state", state),
        ];
        let query: Vec<String> = params
            .iter()
            .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
            .collect();
        let endpoint = authorization_endpoint
            .split('#')
            .next()
            .unwrap_or(authorization_endpoint);
        let endpoint = endpoint.split('?').next().unwrap_or(endpoint);
        format!("{endpoint}?{}", query.join("&"))
    }
}

/// `normalizeRadiusGatewayUrl`: a gateway without a scheme is https, and trailing slashes go.
fn normalize_gateway(value: &str) -> String {
    let scheme = value
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    let with_scheme = match scheme.as_deref() {
        Some("http" | "https") => value.to_owned(),
        _ => format!("https://{value}"),
    };
    with_scheme.trim_end_matches('/').to_owned()
}

/// The gateway's scheme and authority, what upstream's `new URL(path, gateway)` keeps.
fn origin(gateway: &str) -> &str {
    let after_scheme = gateway.find("://").map_or(0, |index| index + 3);
    match gateway[after_scheme..].find(['/', '?', '#']) {
        Some(index) => &gateway[..after_scheme + index],
        None => gateway,
    }
}

/// A failed answer: the readable message, and the OAuth error code when the body carried one.
struct Failure {
    oauth_error: Option<String>,
    message: String,
}

/// Upstream's `OAuthResponseError`: the JSON `error` and `error_description`, else the body text,
/// else the status.
fn response_failure(status: u16, text: &str, message: &str) -> Failure {
    let parsed = if text.is_empty() {
        None
    } else {
        serde_json::from_str::<Value>(text).ok()
    };
    let body = parsed.as_ref().and_then(Value::as_object);
    let oauth_error = body
        .and_then(|body| body.get("error"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let description = if parsed.is_some() {
        body.and_then(|body| body.get("error_description"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        Some(text.to_owned())
    }
    .filter(|description| !description.is_empty());
    let detail = match (oauth_error.as_deref(), description.as_deref()) {
        (Some(error), Some(description)) => format!("{error}: {description}"),
        (Some(error), None) => error.to_owned(),
        (None, Some(description)) => description.to_owned(),
        (None, None) => status.to_string(),
    };
    Failure {
        oauth_error,
        message: format!("{message}: {detail}"),
    }
}

/// A cancelled request reads as a cancelled login, the way upstream reports it.
fn cancelled(cancel: Option<&CancelToken>, message: String) -> String {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        device_code::CANCELLED.to_owned()
    } else {
        message
    }
}

/// A body that is not a JSON object carries no fields.
fn object(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(body)) => Some(body),
        _ => None,
    }
}

/// The credential a token answer carries, its expiry already inside the flow's margin.
fn token_credentials(body: &Map<String, Value>) -> Result<Credentials, String> {
    let field = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let missing = || MISSING_TOKEN_FIELDS.to_owned();
    let access = field("access_token").ok_or_else(missing)?;
    let refresh = field("refresh_token").ok_or_else(missing)?;
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(missing)?;
    let credentials = Credentials::new(
        refresh,
        access,
        now_ms() + (expires_in * 1000.0) as i64 - TOKEN_EXPIRY_SKEW_MS,
    );
    Ok(match field("scope") {
        Some(scope) => credentials.with("scope", Value::String(scope)),
        None => credentials,
    })
}

/// One token exchange at the gateway; the OAuth error code survives so the device poll can tell a
/// pending authorization from a refusal.
fn request_token(
    endpoints: &Endpoints,
    fields: &[(&str, &str)],
    cancel: Option<&CancelToken>,
) -> Result<Credentials, Failure> {
    let other = |message: String| Failure {
        oauth_error: None,
        message,
    };
    let (status, text) = post_form(&endpoints.url("/v1/oauth/token"), fields, cancel)
        .map_err(|error| other(cancelled(cancel, error)))?;
    if !(200..300).contains(&status) {
        return Err(response_failure(
            status,
            &text,
            "Radius OAuth token request failed",
        ));
    }
    match object(&text) {
        Some(body) => token_credentials(&body).map_err(other),
        None => Err(other(MISSING_TOKEN_FIELDS.to_owned())),
    }
}

/// The interactive authorization endpoint, the one thing the gateway does not serve itself.
fn authorization_endpoint(
    endpoints: &Endpoints,
    cancel: Option<&CancelToken>,
) -> Result<String, String> {
    let (status, text) = get_json(&endpoints.url("/v1/oauth"), &[], cancel)
        .map_err(|error| cancelled(cancel, error))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "Could not load Radius OAuth config from {}: {status} {text}",
            endpoints.gateway
        ));
    }
    object(&text)
        .and_then(|body| {
            body.get("authorizationEndpoint")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| format!("Invalid Radius OAuth config from {}", endpoints.gateway))
}

struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in_seconds: f64,
    interval_seconds: Option<f64>,
}

fn request_device_authorization(
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<DeviceCode, String> {
    let (status, text) = post_form(
        &endpoints.url("/v1/oauth/device"),
        &[
            ("client_id", endpoints.client_id.as_str()),
            ("scope", endpoints.scope.as_str()),
        ],
        Some(cancel),
    )
    .map_err(|error| cancelled(Some(cancel), error))?;
    if !(200..300).contains(&status) {
        return Err(
            response_failure(status, &text, "Radius OAuth device authorization failed").message,
        );
    }
    let Some(body) = object(&text) else {
        return Err(MISSING_DEVICE_FIELDS.to_owned());
    };
    let field = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| MISSING_DEVICE_FIELDS.to_owned())
    };
    Ok(DeviceCode {
        device_code: field("device_code")?,
        user_code: field("user_code")?,
        verification_uri: field("verification_uri")?,
        expires_in_seconds: body
            .get("expires_in")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .ok_or_else(|| MISSING_DEVICE_FIELDS.to_owned())?,
        interval_seconds: body.get("interval").and_then(Value::as_f64),
    })
}

fn poll_for_token(
    endpoints: &Endpoints,
    device: &DeviceCode,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let options = Options::new(
        device.interval_seconds,
        Some(device.expires_in_seconds as i64),
    );
    device_code::poll(&options, cancel, || {
        let fields = [
            ("grant_type", DEVICE_GRANT),
            ("client_id", endpoints.client_id.as_str()),
            ("device_code", device.device_code.as_str()),
        ];
        match request_token(endpoints, &fields, Some(cancel)) {
            Ok(credentials) => Ok(Poll::Complete(credentials)),
            Err(failure) => Ok(match failure.oauth_error.as_deref() {
                Some("authorization_pending") => Poll::Pending,
                Some("slow_down") => Poll::SlowDown(None),
                Some("expired_token") => Poll::Failed("Device authorization expired.".to_owned()),
                Some("access_denied") => {
                    Poll::Failed("Device authorization was denied.".to_owned())
                }
                _ => return Err(failure.message),
            }),
        }
    })
}

fn login_with_device_code(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let device = request_device_authorization(endpoints, cancel)?;
    interaction.notify(Event::DeviceCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        interval_seconds: device
            .interval_seconds
            .map_or(0, |seconds| seconds.floor() as u64),
        expires_in_seconds: device.expires_in_seconds as u64,
    });
    poll_for_token(endpoints, &device, cancel)
}

fn login_with_browser(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    authorization_endpoint: &str,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let pair = pkce::generate()?;
    let state = new_state()?;
    let listener = Listener::bind(CALLBACK_HOST, CALLBACK_PORT)
        .map_err(|error| format!("OAuth callback server failed: {error}"))?;
    let redirect_uri = endpoints.redirect_uri(listener.port());
    interaction.notify(Event::Progress(format!(
        "Listening for OAuth callback on {redirect_uri}"
    )));
    interaction.notify(Event::AuthUrl {
        url: endpoints.authorize_link(authorization_endpoint, &redirect_uri, &pair, &state),
        instructions: Some(BROWSER_INSTRUCTIONS.to_owned()),
    });
    let code = listener.wait_code(NAME, CALLBACK_PATH, Some(&state), cancel)?;
    request_token(
        endpoints,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", endpoints.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code", code.as_str()),
            ("code_verifier", pair.verifier.as_str()),
        ],
        Some(cancel),
    )
    .map_err(|failure| failure.message)
}

/// A fresh opaque `state` for the browser round trip, 16 random bytes as hex.
fn new_state() -> Result<String, String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "could not read random bytes for the OAuth state".to_owned())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Asks which sign-in method to use and runs it (upstream `createRadiusOAuth`). The gateway is
/// taken as given; the module never reads it from the provider entry.
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let method = interaction
        .prompt(
            Prompt::Select {
                message: format!("Sign in to {NAME}:"),
                options: vec![
                    (
                        BROWSER.to_owned(),
                        "Sign in with browser (recommended)".to_owned(),
                    ),
                    (
                        DEVICE_CODE.to_owned(),
                        "Sign in with device code (when signing in from another device)".to_owned(),
                    ),
                ],
            },
            cancel,
        )?
        .ok_or_else(|| device_code::CANCELLED.to_owned())?;
    match method.as_str() {
        BROWSER => {
            let endpoint = authorization_endpoint(endpoints, Some(cancel))?;
            login_with_browser(interaction, endpoints, &endpoint, cancel)
        }
        DEVICE_CODE => login_with_device_code(interaction, endpoints, cancel),
        other => Err(format!("Unknown {NAME} sign-in method: {other}")),
    }
}

/// Exchanges a stored refresh token through the gateway, without discovery.
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let refresh_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    request_token(
        endpoints,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", endpoints.client_id.as_str()),
            ("refresh_token", refresh_token),
        ],
        cancel,
    )
    .map_err(|failure| failure.message)
}
