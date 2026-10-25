//! ai/src/auth/oauth/openai-chatgpt.ts: Sign in with ChatGPT registers a client and keeps its tokens.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Map, Value};

use crate::oauth::interaction::{Event, Interaction, Prompt};
use crate::oauth::pkce::{self, Pkce};
use crate::oauth::{Credentials, env_url, form_encode, now_ms, post_form, query_param};
use pi_rt::cancel::CancelToken;

const POLL: Duration = Duration::from_millis(50);
const READ_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_HEAD_BYTES: usize = 8 * 1024;
const FAILED: &str = "Authentication failed";
const SUCCESS: &str = "Authentication successful";
const CANCELLED: &str = "Login cancelled";
const INSTRUCTIONS: &str = "Complete sign-in in your browser. If the callback does not complete, paste the final redirect URL here.";
const MANUAL_PROMPT: &str = "Complete login in your browser, or paste the final redirect URL here:";
const EXCHANGING: &str = "Exchanging authorization code for tokens...";
const EXPIRY_MARGIN_MS: i64 = 3 * 60 * 1000;

/// Every URL, client id and scope the flow uses. `Default` is the production set; a check moves the
/// URLs through `PI_OAUTH_OPENAI_<FIELD>` and the bind host through pi's `PI_OAUTH_CALLBACK_HOST`.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub client_id: String,
    pub agent_name_hint: String,
    pub authorize_url: String,
    pub token_url: String,
    pub resource: String,
    pub scope: String,
    pub direct_token_scope: String,
    pub callback_host: String,
    pub callback_port: u16,
    pub callback_path: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            client_id: "dynamic_agent_client".to_owned(),
            agent_name_hint: "Pi".to_owned(),
            authorize_url: "https://auth.openai.com/api/accounts/authorize".to_owned(),
            token_url: "https://auth.openai.com/api/accounts/oauth/token".to_owned(),
            resource: "https://api.openai.com/v1".to_owned(),
            scope: "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
                .to_owned(),
            direct_token_scope: "chatgpt.tokens.use.direct".to_owned(),
            callback_host: "127.0.0.1".to_owned(),
            callback_port: 1455,
            callback_path: "/auth/callback".to_owned(),
        }
    }
}

impl Endpoints {
    /// The production endpoints, with the ones a check or a gateway moves through
    /// `PI_OAUTH_OPENAI_<FIELD>`.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            authorize_url: env_url("openai", "authorize_url", &default.authorize_url),
            token_url: env_url("openai", "token_url", &default.token_url),
            resource: env_url("openai", "resource", &default.resource),
            callback_host: std::env::var("PI_OAUTH_CALLBACK_HOST")
                .ok()
                .filter(|host| !host.is_empty())
                .unwrap_or(default.callback_host),
            ..default
        }
    }

    /// The redirect URI the browser returns to. Upstream fixes its host at 127.0.0.1 whatever the
    /// server binds to.
    pub fn redirect_uri(&self) -> String {
        format!(
            "http://127.0.0.1:{}{}",
            self.callback_port, self.callback_path
        )
    }

    /// The URL the user opens. Every login registers a new client with `client_id`; OpenAI returns
    /// the issued one in the callback.
    pub fn authorize_link(&self, pkce: &Pkce, state: &str, nonce: &str, host_id: &str) -> String {
        let redirect_uri = self.redirect_uri();
        let params = [
            ("client_id", self.client_id.as_str()),
            ("agent_name_hint", self.agent_name_hint.as_str()),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri.as_str()),
            ("resource", self.resource.as_str()),
            ("scope", self.scope.as_str()),
            ("state", state),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("nonce", nonce),
        ];
        let query: Vec<String> = params
            .iter()
            .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
            .collect();
        format!("{}?{}", self.authorize_url, query.join("&"))
    }
}

/// OpenAI identifies each installation ("agent host") by a stable URI such as `urn:uuid:<uuid>`.
fn agent_host_id(device_id: Option<&str>) -> Result<String, String> {
    let device_id = device_id.filter(|id| is_uuid(id)).ok_or_else(|| {
        "Sign in with ChatGPT requires a device ID (UUID) for this installation".to_owned()
    })?;
    Ok(format!("urn:uuid:{}", device_id.to_ascii_lowercase()))
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.char_indices().all(|(index, c)| match index {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn random_value() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "system random source failed".to_owned())?;
    Ok(pkce::base64url(&bytes))
}

/// What the browser's redirect carries: the code, and the client ID OpenAI issued for it.
struct Authorization {
    code: String,
    client_id: String,
}

/// The code, state and issued client ID of one callback query.
fn authorization_result(query: &str, state: &str) -> Result<Authorization, String> {
    let code = query_param(query, "code")
        .filter(|code| !code.is_empty())
        .ok_or_else(|| "Missing authorization code".to_owned())?;
    let returned = query_param(query, "state")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing OAuth state".to_owned())?;
    if returned != state {
        return Err("OAuth state mismatch".to_owned());
    }
    let client_id = query_param(query, "client_id")
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            "OpenAI OAuth registration callback did not contain an issued client ID".to_owned()
        })?;
    Ok(Authorization { code, client_id })
}

/// An absolute URL split the way `new URL()` compares: origin and path.
struct Url {
    origin: String,
    path: String,
    query: String,
}

impl Url {
    fn parse(raw: &str) -> Option<Self> {
        let (scheme, rest) = raw.split_once("://")?;
        if scheme.is_empty() || !scheme.chars().all(is_scheme_char) {
            return None;
        }
        let rest = rest.split('#').next().unwrap_or(rest);
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (host_part, target) = rest.split_at(end);
        let origin = format!(
            "{}://{}",
            scheme.to_ascii_lowercase(),
            authority(host_part, scheme)?
        );
        let (path, query) = match target.split_once('?') {
            Some((path, query)) => (path, query),
            None => (target, ""),
        };
        Some(Self {
            origin,
            path: if path.is_empty() {
                "/".to_owned()
            } else {
                path.to_owned()
            },
            query: query.to_owned(),
        })
    }
}

fn is_scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
}

/// The authority with its host lowercased and its default port dropped, as `URL.origin` has it.
fn authority(raw: &str, scheme: &str) -> Option<String> {
    let host_port = raw.rsplit_once('@').map_or(raw, |(_, host)| host);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !host.contains('[') && port.chars().all(|c| c.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (host_port, None),
    };
    if host.is_empty() {
        return None;
    }
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => "80",
        "https" => "443",
        _ => "",
    };
    let host = host.to_ascii_lowercase();
    match port.filter(|port| !port.is_empty() && *port != default_port) {
        Some(port) => Some(format!("{host}:{port}")),
        None => Some(host),
    }
}

/// Turns a pasted redirect URL into what the callback would have carried.
fn manual_result(input: &str, endpoints: &Endpoints, state: &str) -> Result<Authorization, String> {
    let redirect_uri = endpoints.redirect_uri();
    let url = Url::parse(input.trim())
        .ok_or_else(|| "Paste the full callback URL from the browser".to_owned())?;
    let expected = Url::parse(&redirect_uri)
        .ok_or_else(|| format!("The pasted callback URL must start with {redirect_uri}"))?;
    if url.origin != expected.origin || url.path != expected.path {
        return Err(format!(
            "The pasted callback URL must start with {redirect_uri}"
        ));
    }
    if let Some(error) = query_param(&url.query, "error").filter(|error| !error.is_empty()) {
        return Err(format!("ChatGPT authorization failed: {error}"));
    }
    authorization_result(&url.query, state)
}

/// A loopback listener bound to the fixed port the redirect URI names.
struct Server {
    listener: TcpListener,
}

enum Step {
    Wait,
    Done(Authorization),
    Failed(String),
}

impl Server {
    /// Binds `host` on `port` and nothing else: another login or the Codex CLI holding it is an
    /// error, not a reason to move.
    fn start(host: &str, port: u16) -> Result<Self, String> {
        let listener = TcpListener::bind((host, port)).map_err(|error| {
            if error.kind() == io::ErrorKind::AddrInUse {
                format!(
                    "Port {port} is in use, probably by an unfinished login in another pi session or by the Codex CLI. Cancel that login and try again."
                )
            } else {
                format!("OAuth callback server failed: {error}")
            }
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("OAuth callback server failed: {error}"))?;
        Ok(Self { listener })
    }

    /// Serves until a request carries the state and a code, or the provider reports an error.
    /// A request that cannot complete the sign-in gets an error page and the wait goes on.
    fn wait(&self, path: &str, state: &str, cancel: &CancelToken) -> Result<Authorization, String> {
        loop {
            if cancel.is_cancelled() {
                return Err(CANCELLED.to_owned());
            }
            match self.listener.accept() {
                Ok((stream, _)) => match serve(stream, path, state) {
                    Ok(Step::Done(authorization)) => return Ok(authorization),
                    Ok(Step::Failed(error)) => return Err(error),
                    Ok(Step::Wait) | Err(_) => {}
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if cancel.wait_timeout(POLL) {
                        return Err(CANCELLED.to_owned());
                    }
                }
                Err(error) => return Err(format!("OAuth callback server failed: {error}")),
            }
        }
    }
}

fn serve(mut stream: TcpStream, path: &str, state: &str) -> io::Result<Step> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let head = read_head(&mut stream)?;
    let target = head
        .lines()
        .next()
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("")
        .to_owned();
    let (route, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    if route != path {
        respond(
            &mut stream,
            "404 Not Found",
            &page(FAILED, "Callback route not found.", None),
        )?;
        return Ok(Step::Wait);
    }
    if let Some(error) = query_param(query, "error").filter(|error| !error.is_empty()) {
        let details = format!("Error: {error}");
        respond(
            &mut stream,
            "400 Bad Request",
            &page(FAILED, "ChatGPT was not connected.", Some(&details)),
        )?;
        return Ok(Step::Failed(format!(
            "ChatGPT authorization failed: {error}"
        )));
    }
    match authorization_result(query, state) {
        Ok(authorization) => {
            respond(
                &mut stream,
                "200 OK",
                &page(
                    SUCCESS,
                    "ChatGPT authentication completed. You can close this window.",
                    None,
                ),
            )?;
            Ok(Step::Done(authorization))
        }
        Err(message) => {
            respond(
                &mut stream,
                "400 Bad Request",
                &page(FAILED, &message, None),
            )?;
            Ok(Step::Wait)
        }
    }
}

fn read_head(stream: &mut TcpStream) -> io::Result<String> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") && head.len() < MAX_HEAD_BYTES {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn page(heading: &str, message: &str, details: Option<&str>) -> String {
    let details = details
        .map(|details| format!("<pre>{}</pre>", escape(details)))
        .unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{h}</title></head>\
<body style=\"background:#09090b;color:#fafafa;font-family:system-ui,sans-serif;text-align:center;padding:24px\">\
<h1>{h}</h1><p>{m}</p>{details}</body></html>",
        h = escape(heading),
        m = escape(message),
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The browser callback and the pasted redirect URL race; whichever answers first settles the
/// login, and the other stops.
fn wait_for_answer(
    interaction: &mut dyn Interaction,
    server: Server,
    endpoints: &Endpoints,
    state: &str,
) -> Result<Authorization, String> {
    let dismiss = CancelToken::new();
    let stop = CancelToken::new();
    let received: Arc<Mutex<Option<Result<Authorization, String>>>> = Arc::new(Mutex::new(None));
    let (shared, wake) = (received.clone(), dismiss.clone());
    let (stop_in_thread, path, expected) = (
        stop.clone(),
        endpoints.callback_path.clone(),
        state.to_owned(),
    );
    let waiter = std::thread::spawn(move || {
        let result = server.wait(&path, &expected, &stop_in_thread);
        *shared.lock().unwrap() = Some(result);
        wake.cancel();
    });
    let answer = interaction.prompt(
        Prompt::ManualCode {
            message: MANUAL_PROMPT.to_owned(),
            placeholder: endpoints.redirect_uri(),
        },
        &dismiss,
    );
    stop.cancel();
    let _ = waiter.join();
    match answer {
        Ok(Some(input)) => manual_result(&input, endpoints, state),
        Ok(None) => match received.lock().unwrap().take() {
            Some(Ok(authorization)) => Ok(authorization),
            Some(Err(error)) => Err(error),
            None => Err(CANCELLED.to_owned()),
        },
        Err(error) => Err(error),
    }
}

fn cancelled(cancel: Option<&CancelToken>, message: String) -> String {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        CANCELLED.to_owned()
    } else {
        message
    }
}

/// A token POST whose body is the JSON object it returned.
fn token_request(
    url: &str,
    fields: &[(&str, &str)],
    cancel: Option<&CancelToken>,
) -> Result<Map<String, Value>, String> {
    let (status, text) =
        post_form(url, fields, cancel).map_err(|error| cancelled(cancel, error))?;
    if !(200..300).contains(&status) {
        return Err(cancelled(
            cancel,
            format!("OpenAI OAuth token request failed ({status}): {text}"),
        ));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(body)) => Ok(body),
        _ => Err("OpenAI OAuth token response must be an object".to_owned()),
    }
}

fn required_string(body: &Map<String, Value>, field: &str) -> Result<String, String> {
    body.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("OpenAI OAuth token response has invalid {field}"))
}

fn credential(
    body: &Map<String, Value>,
    client_id: &str,
    direct_token_scope: &str,
) -> Result<Credentials, String> {
    let access = required_string(body, "access_token")?;
    let refresh = required_string(body, "refresh_token")?;
    let scope = required_string(body, "scope")?;
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(|| "OpenAI OAuth token response has invalid expires_in".to_owned())?;
    let scopes: Vec<&str> = scope.split_whitespace().collect();
    if !scopes.contains(&direct_token_scope) {
        return Err(format!(
            "OpenAI OAuth grant did not include {direct_token_scope}"
        ));
    }
    Ok(Credentials::new(
        refresh,
        access,
        now_ms() + (expires_in * 1000.0) as i64 - EXPIRY_MARGIN_MS,
    )
    .with("clientId", Value::String(client_id.to_owned()))
    .with(
        "scopes",
        Value::Array(scopes.into_iter().map(|scope| scope.into()).collect()),
    ))
}

fn exchange_code(
    endpoints: &Endpoints,
    authorization: &Authorization,
    verifier: &str,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let redirect_uri = endpoints.redirect_uri();
    let token = token_request(
        &endpoints.token_url,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", authorization.client_id.as_str()),
            ("code", authorization.code.as_str()),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri.as_str()),
            ("resource", endpoints.resource.as_str()),
        ],
        cancel,
    )?;
    if token
        .get("id_token")
        .and_then(Value::as_str)
        .is_none_or(|id_token| id_token.trim().is_empty())
    {
        return Err("OpenAI OAuth token response did not contain an ID token".to_owned());
    }
    credential(
        &token,
        &authorization.client_id,
        &endpoints.direct_token_scope,
    )
}

/// Trades the stored refresh token for a new access token, keeping the client ID the login issued.
pub fn refresh(
    endpoints: &Endpoints,
    stored: &Map<String, Value>,
    cancel: Option<&CancelToken>,
) -> Result<Credentials, String> {
    let client_id = stored
        .get("clientId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            "Stored OpenAI OAuth credential does not contain an issued client ID; reconnect ChatGPT"
                .to_owned()
        })?;
    let refresh_token = stored
        .get("refresh")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let token = token_request(
        &endpoints.token_url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", endpoints.resource.as_str()),
        ],
        cancel,
    )?;
    credential(&token, client_id, &endpoints.direct_token_scope)
}

/// Opens the sign-in page, then waits for the browser's redirect or a pasted redirect URL.
pub fn login(
    interaction: &mut dyn Interaction,
    endpoints: &Endpoints,
    cancel: &CancelToken,
    device_id: Option<&str>,
) -> Result<Credentials, String> {
    if cancel.is_cancelled() {
        return Err(CANCELLED.to_owned());
    }
    let host_id = agent_host_id(device_id)?;
    let pair = pkce::generate()?;
    let state = random_value()?;
    let nonce = random_value()?;
    let server = Server::start(&endpoints.callback_host, endpoints.callback_port)?;
    interaction.notify(Event::AuthUrl {
        url: endpoints.authorize_link(&pair, &state, &nonce, &host_id),
        instructions: Some(INSTRUCTIONS.to_owned()),
    });
    let authorization = wait_for_answer(interaction, server, endpoints, &state)?;
    interaction.notify(Event::Progress(EXCHANGING.to_owned()));
    exchange_code(endpoints, &authorization, &pair.verifier, Some(cancel))
}
