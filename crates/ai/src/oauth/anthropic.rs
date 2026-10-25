use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::http::{Request, send};
use crate::oauth::callback::{Code, Listener, wait_for_code};
use crate::oauth::interaction::{Event, Interaction, Prompt};
use crate::oauth::pkce::{self, Pkce};
use crate::oauth::{Credentials, env_url, form_encode, now_ms, query_param};
use pi_rt::cancel::CancelToken;

const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
const EXPIRY_MARGIN_MS: i64 = 5 * 60 * 1000;

/// Where the Claude OAuth flow talks to. `Default` is the production set; an app can override
/// `callback_host` (from its own config) and tests point the URLs at a local server.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: String,
    pub copy_code_redirect_uri: String,
    pub callback_host: String,
    pub callback_port: u16,
    pub callback_path: String,
    pub redirect_host: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_owned(),
            authorize_url: "https://claude.ai/oauth/authorize".to_owned(),
            token_url: "https://platform.claude.com/v1/oauth/token".to_owned(),
            scopes: "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload".to_owned(),
            copy_code_redirect_uri: "https://platform.claude.com/oauth/code/callback".to_owned(),
            callback_host: "127.0.0.1".to_owned(),
            callback_port: 53692,
            callback_path: "/callback".to_owned(),
            redirect_host: "localhost".to_owned(),
        }
    }
}

impl Endpoints {
    /// The production endpoints, with the ones a check or a gateway moves through
    /// `PI_OAUTH_ANTHROPIC_<FIELD>`.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            authorize_url: env_url("anthropic", "authorize_url", &default.authorize_url),
            token_url: env_url("anthropic", "token_url", &default.token_url),
            ..default
        }
    }

    /// The redirect URI for the loopback flow on `port`, as the browser sees it.
    pub fn loopback_redirect_uri(&self, port: u16) -> String {
        format!("http://{}:{port}{}", self.redirect_host, self.callback_path)
    }

    /// The URL the user opens. `redirect_uri` is the loopback URI or `copy_code_redirect_uri`;
    /// the state is the PKCE verifier, as in pi-ai.
    pub fn authorize_link(&self, redirect_uri: &str, pkce: &Pkce) -> String {
        let params = [
            ("code", "true"),
            ("client_id", self.client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri),
            ("scope", self.scopes.as_str()),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", pkce.verifier.as_str()),
        ];
        let query: Vec<String> = params
            .iter()
            .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
            .collect();
        format!("{}?{}", self.authorize_url, query.join("&"))
    }
}

/// Fields of a pasted or redirected authorization response.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub code: Option<String>,
    pub state: Option<String>,
}

/// Reads `code`/`state` from a full redirect URL, a query string, `code#state`, or a bare code.
pub fn parse_authorization_input(input: &str) -> Parsed {
    let value = input.trim();
    if value.is_empty() {
        return Parsed::default();
    }
    if let Some(query) = absolute_url_query(value) {
        return Parsed {
            code: query_param(query, "code"),
            state: query_param(query, "state"),
        };
    }
    if value.contains('#') {
        let mut parts = value.split('#');
        return Parsed {
            code: parts.next().map(str::to_owned),
            state: parts.next().map(str::to_owned),
        };
    }
    if value.contains("code=") {
        let query = value.strip_prefix('?').unwrap_or(value);
        return Parsed {
            code: query_param(query, "code"),
            state: query_param(query, "state"),
        };
    }
    Parsed {
        code: Some(value.to_owned()),
        state: None,
    }
}

fn absolute_url_query(value: &str) -> Option<&str> {
    let (scheme, rest) = value.split_once(':')?;
    let mut chars = scheme.chars();
    let valid_scheme = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        return None;
    }
    let without_fragment = rest.split('#').next().unwrap_or(rest);
    Some(
        without_fragment
            .split_once('?')
            .map_or("", |(_, query)| query),
    )
}

/// Turns manually pasted input into `(code, state)`. A state that is present and differs from
/// `verifier` is refused, as pi-ai does.
pub fn resolve_manual_input(input: &str, verifier: &str) -> Result<(String, String), String> {
    let parsed = parse_authorization_input(input);
    if parsed
        .state
        .as_deref()
        .is_some_and(|state| !state.is_empty() && state != verifier)
    {
        return Err("OAuth state mismatch".to_owned());
    }
    let code = parsed
        .code
        .filter(|code| !code.is_empty())
        .ok_or_else(|| "Missing authorization code".to_owned())?;
    let state = parsed.state.unwrap_or_else(|| verifier.to_owned());
    Ok((code, state))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

impl TokenResponse {
    fn into_credentials(self, now: i64) -> Credentials {
        Credentials::new(
            self.refresh_token,
            self.access_token,
            now + self.expires_in * 1000 - EXPIRY_MARGIN_MS,
        )
    }
}

/// Exchanges an authorization code for tokens. `redirect_uri` must be the one sent to the
/// authorize URL.
pub fn exchange_code(
    endpoints: &Endpoints,
    code: &str,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let body = json!({
        "grant_type": "authorization_code",
        "client_id": endpoints.client_id,
        "code": code,
        "state": state,
        "redirect_uri": redirect_uri,
        "code_verifier": verifier,
    });
    let url = &endpoints.token_url;
    let text = post_json(url, &body, cancel).map_err(|details| {
        format!("Token exchange request failed. url={url}; redirect_uri={redirect_uri}; response_type=authorization_code; details={details}")
    })?;
    let data: TokenResponse = serde_json::from_str(&text).map_err(|error| {
        format!("Token exchange returned invalid JSON. url={url}; body={text}; details=SyntaxError: {error}")
    })?;
    Ok(data.into_credentials(now_ms()))
}

/// Trades a refresh token for a new access token and refresh token.
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let refresh_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = json!({
        "grant_type": "refresh_token",
        "client_id": endpoints.client_id,
        "refresh_token": refresh_token,
    });
    let url = &endpoints.token_url;
    let text = post_json(url, &body, cancel).map_err(|details| {
        format!("Anthropic token refresh request failed. url={url}; details={details}")
    })?;
    let data: TokenResponse = serde_json::from_str(&text).map_err(|error| {
        format!("Anthropic token refresh returned invalid JSON. url={url}; body={text}; details=SyntaxError: {error}")
    })?;
    Ok(data.into_credentials(now_ms()))
}

/// Whether an API key is an OAuth access token (sent as a bearer token, not `x-api-key`).
pub fn is_oauth_token(key: &str) -> bool {
    key.contains("sk-ant-oat")
}

const BROWSER: &str = "browser";
const COPY_CODE: &str = "copy_code";
const PROVIDER: &str = "Anthropic";
const BROWSER_INSTRUCTIONS: &str = "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here.";
const COPY_CODE_INSTRUCTIONS: &str =
    "Complete login in your browser, then copy the code Anthropic shows and paste it here.";
const MANUAL_PROMPT: &str =
    "Complete login in your browser, or paste the authorization code / redirect URL here:";
const COPY_CODE_PROMPT: &str = "Paste the code Anthropic shows after you sign in:";
const EXCHANGING: &str = "Exchanging authorization code for tokens...";

/// Asks which Anthropic login method to use and runs it (upstream `anthropicOAuth.login`).
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let method = interaction
        .prompt(
            Prompt::Select {
                message: "Select Anthropic login method:".to_owned(),
                options: vec![
                    (BROWSER.to_owned(), "Browser login (default)".to_owned()),
                    (
                        COPY_CODE.to_owned(),
                        "Copy code login (headless)".to_owned(),
                    ),
                ],
            },
            cancel,
        )?
        .ok_or_else(|| "Login cancelled".to_owned())?;
    match method.as_str() {
        BROWSER => login_browser(interaction, endpoints, cancel),
        COPY_CODE => login_copy_code(interaction, endpoints, cancel),
        other => Err(format!("Unknown Anthropic login method: {other}")),
    }
}

/// The loopback login: the browser's redirect or a pasted code, whichever comes first.
pub fn login_browser(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let pkce = pkce::generate()?;
    let listener = Listener::bind(&endpoints.callback_host, endpoints.callback_port).ok();
    let redirect_uri = match &listener {
        Some(listener) => endpoints.loopback_redirect_uri(listener.port()),
        None => endpoints.loopback_redirect_uri(endpoints.callback_port),
    };
    interaction.notify(Event::AuthUrl {
        url: endpoints.authorize_link(&redirect_uri, &pkce),
        instructions: Some(BROWSER_INSTRUCTIONS.to_owned()),
    });
    let answer = wait_for_code(
        interaction,
        listener,
        PROVIDER,
        &endpoints.callback_path,
        Some(&pkce.verifier),
        Prompt::ManualCode {
            message: MANUAL_PROMPT.to_owned(),
            placeholder: redirect_uri.clone(),
        },
        cancel,
    )?;
    let (code, state) = match answer {
        Code::Callback(code) => (code, pkce.verifier.clone()),
        Code::Manual(input) => resolve_manual_input(&input, &pkce.verifier)?,
    };
    interaction.notify(Event::Progress(EXCHANGING.to_owned()));
    exchange_code(
        endpoints,
        &code,
        &state,
        &pkce.verifier,
        &redirect_uri,
        Some(cancel),
    )
}

/// The headless login: Anthropic redirects to a page that shows a code to paste back.
pub fn login_copy_code(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
) -> Result<Credentials, String> {
    let pkce = pkce::generate()?;
    interaction.notify(Event::AuthUrl {
        url: endpoints.authorize_link(&endpoints.copy_code_redirect_uri, &pkce),
        instructions: Some(COPY_CODE_INSTRUCTIONS.to_owned()),
    });
    let input = interaction
        .prompt(
            Prompt::ManualCode {
                message: COPY_CODE_PROMPT.to_owned(),
                placeholder: "code#state".to_owned(),
            },
            cancel,
        )?
        .ok_or_else(|| "Login cancelled".to_owned())?;
    let (code, state) = resolve_manual_input(&input, &pkce.verifier)?;
    interaction.notify(Event::Progress(EXCHANGING.to_owned()));
    exchange_code(
        endpoints,
        &code,
        &state,
        &pkce.verifier,
        &endpoints.copy_code_redirect_uri,
        Some(cancel),
    )
}

fn post_json(url: &str, body: &Value, cancel: Option<&CancelToken>) -> Result<String, String> {
    let request = Request::new("POST", url)
        .header("Accept", "application/json")
        .json(body)
        .timeout(Some(TOKEN_TIMEOUT));
    let response = send(&request, cancel).map_err(|error| format!("Error: {error}"))?;
    let status = response.status;
    let ok = response.ok();
    let text = response.text().map_err(|error| format!("Error: {error}"))?;
    if !ok {
        return Err(format!(
            "HTTP request failed. status={status}; url={url}; body={text}"
        ));
    }
    Ok(text)
}
