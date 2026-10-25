use serde_json::{Map, Value};

use crate::oauth::callback::{Code, Listener, wait_for_code};
use crate::oauth::interaction::{Event, Interaction, Prompt};
use crate::oauth::pkce::{self, Pkce};
use crate::oauth::{Credentials, env_url, form_encode, post_json, query_param};
use crate::utils::uuid::uuidv7;
use pi_rt::cancel::CancelToken;

const PATH_PREFIX: &str = "/oauth/callback";
const PROVIDER: &str = "OpenRouter";
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const INSTRUCTIONS: &str = "Complete sign-in in your browser. If the browser is on another machine, paste the final redirect URL here.";
const MANUAL_PROMPT: &str =
    "Complete sign-in in your browser, or paste the authorization code / redirect URL here:";
const EXCHANGING: &str = "Exchanging authorization code for an API key...";
const CANCELLED: &str = "Login cancelled";

/// Where the OpenRouter login talks to. `Default` is the production set; a check moves the URLs
/// through `PI_OAUTH_OPENROUTER_<FIELD>` and the callback host through pi's `PI_OAUTH_CALLBACK_HOST`.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorize_url: String,
    pub token_url: String,
    pub callback_host: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            authorize_url: "https://openrouter.ai/auth".to_owned(),
            token_url: "https://openrouter.ai/api/v1/auth/keys".to_owned(),
            callback_host: "127.0.0.1".to_owned(),
        }
    }
}

impl Endpoints {
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            authorize_url: env_url("openrouter", "authorize_url", &default.authorize_url),
            token_url: env_url("openrouter", "token_url", &default.token_url),
            callback_host: std::env::var("PI_OAUTH_CALLBACK_HOST")
                .ok()
                .filter(|host| !host.is_empty())
                .unwrap_or(default.callback_host),
        }
    }

    /// The redirect URI for the loopback flow on `port`, as the browser sees it.
    pub fn redirect_uri(&self, port: u16, path: &str) -> String {
        let host = if self.callback_host.contains(':') {
            format!("[{}]", self.callback_host)
        } else {
            self.callback_host.clone()
        };
        format!("http://{host}:{port}{path}")
    }

    /// The URL the user opens; `callback_url` is the loopback URI.
    pub fn authorize_link(&self, callback_url: &str, pkce: &Pkce) -> String {
        let params = [
            ("callback_url", callback_url),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        let query: Vec<String> = params
            .iter()
            .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
            .collect();
        format!("{}?{}", self.authorize_url, query.join("&"))
    }
}

/// Signs in through the loopback redirect or a pasted redirect URL, whichever comes first, and
/// returns the API key OpenRouter hands out for the authorization code.
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    if cancel.is_cancelled() {
        return Err(CANCELLED.to_owned());
    }
    let pair = pkce::generate()?;
    let path = format!("{PATH_PREFIX}/{}", uuidv7(None)?);
    let listener = Listener::bind(&endpoints.callback_host, 0)
        .map_err(|error| format!("OAuth callback server failed: {error}"))?;
    let redirect_uri = endpoints.redirect_uri(listener.port(), &path);
    interaction.notify(Event::Progress(format!(
        "Listening for OpenRouter OAuth callback on {redirect_uri}"
    )));
    interaction.notify(Event::AuthUrl {
        url: endpoints.authorize_link(&redirect_uri, &pair),
        instructions: Some(INSTRUCTIONS.to_owned()),
    });
    let answer = wait_for_code(
        interaction,
        Some(listener),
        PROVIDER,
        &path,
        None,
        Prompt::ManualCode {
            message: MANUAL_PROMPT.to_owned(),
            placeholder: redirect_uri,
        },
        cancel,
    )?;
    match answer {
        Code::Callback(code) => exchange(endpoints, &code, &pair.verifier, cancel),
        Code::Manual(input) => {
            let code =
                code_from_input(&input).ok_or_else(|| "Missing authorization code".to_owned())?;
            interaction.notify(Event::Progress(EXCHANGING.to_owned()));
            exchange(endpoints, &code, &pair.verifier, cancel)
        }
    }
}

fn exchange(
    endpoints: &Endpoints,
    code: &str,
    verifier: &str,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    if cancel.is_cancelled() {
        return Err(CANCELLED.to_owned());
    }
    let body = serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "code_challenge_method": "S256",
    });
    let (status, text) = post_json(&endpoints.token_url, &body, Some(cancel)).map_err(|error| {
        if cancel.is_cancelled() {
            CANCELLED.to_owned()
        } else {
            error
        }
    })?;
    let parsed = serde_json::from_str::<Value>(&text).ok();
    let ok = (200..300).contains(&status);
    if ok && parsed.is_none() {
        return Err("OpenRouter OAuth returned invalid JSON".to_owned());
    }
    let body = parsed.as_ref().and_then(Value::as_object);
    if !ok {
        let failed = format!("OpenRouter OAuth key exchange failed (HTTP {status})");
        return Err(match error_detail(body) {
            Some(detail) => format!("{failed}: {detail}"),
            None => failed,
        });
    }
    let key = body
        .and_then(|body| body.get("key"))
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "OpenRouter OAuth response carries no \"key\"".to_owned())?;
    Ok(Credentials::new(
        String::new(),
        key.to_owned(),
        MAX_SAFE_INTEGER,
    ))
}

fn error_detail(body: Option<&Map<String, Value>>) -> Option<String> {
    let body = body?;
    for name in ["error_description", "message", "error"] {
        if let Some(Value::String(text)) = body.get(name) {
            return Some(text.clone());
        }
    }
    body.get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The code in a pasted redirect URL, a query string, or a bare code (upstream
/// `parseAuthorizationInput`).
fn code_from_input(input: &str) -> Option<String> {
    let value = input.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(query) = url_query(value) {
        return query_param(query, "code");
    }
    if value.contains("code=") {
        return query_param(value.strip_prefix('?').unwrap_or(value), "code");
    }
    Some(value.to_owned())
}

fn url_query(value: &str) -> Option<&str> {
    let (scheme, rest) = value.split_once(':')?;
    let mut chars = scheme.chars();
    let absolute = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !absolute {
        return None;
    }
    let without_fragment = rest.split('#').next().unwrap_or(rest);
    Some(
        without_fragment
            .split_once('?')
            .map_or("", |(_, query)| query),
    )
}
