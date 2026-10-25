pub mod anthropic;
pub mod callback;
pub mod device_code;
pub mod github_copilot;
pub mod interaction;
pub mod kimi_coding;
pub mod meta;
pub mod openai_codex;
pub mod openrouter;
pub mod pkce;
pub mod xai;

use pi_rt::cancel::CancelToken;
use serde_json::{Map, Value};

use crate::http::{Request, send};

/// OAuth credentials as a flow returns them. `expires_ms` is when the access token should be
/// refreshed, already inside the margin the flow uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Credentials {
    pub refresh: String,
    pub access: String,
    pub expires_ms: i64,
    /// The provider's own stored fields, in the order it writes them after `expires`.
    pub extras: Vec<(&'static str, Value)>,
}

impl Credentials {
    pub fn new(refresh: String, access: String, expires_ms: i64) -> Self {
        Self {
            refresh,
            access,
            expires_ms,
            extras: Vec::new(),
        }
    }

    pub fn with(mut self, name: &'static str, value: Value) -> Self {
        self.extras.push((name, value));
        self
    }
}

/// The credential one provider stores, as `auth-storage.ts` writes it: `type` first, then the
/// token fields in the order upstream's flow returns them, then the provider's own.
pub fn stored(provider: &str, credentials: &Credentials) -> Value {
    let access_first = matches!(
        provider,
        "openai-codex" | "openai" | "kimi-coding" | "openrouter" | "xai" | "radius"
    );
    let mut map = Map::new();
    map.insert("type".to_owned(), Value::String("oauth".to_owned()));
    if access_first {
        map.insert("access".to_owned(), credentials.access.clone().into());
        map.insert("refresh".to_owned(), credentials.refresh.clone().into());
    } else {
        map.insert("refresh".to_owned(), credentials.refresh.clone().into());
        map.insert("access".to_owned(), credentials.access.clone().into());
    }
    map.insert("expires".to_owned(), credentials.expires_ms.into());
    for (name, value) in &credentials.extras {
        map.insert((*name).to_owned(), value.clone());
    }
    Value::Object(map)
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_millis() as i64
}

/// Encodes like `URLSearchParams`: unreserved bytes stay, space becomes `+`, the rest is `%XX`.
pub fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Decodes one `application/x-www-form-urlencoded` component. Malformed escapes stay literal.
pub fn form_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'+' {
            out.push(b' ');
        } else if byte == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            match u8::from_str_radix(hex, 16) {
                Ok(decoded) => {
                    out.push(decoded);
                    index += 2;
                }
                Err(_) => out.push(byte),
            }
        } else {
            out.push(byte);
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The timeout every OAuth request uses.
pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A form-encoded POST, answering the status and the body so the flow words its own error.
pub fn post_form(
    url: &str,
    pairs: &[(&str, &str)],
    cancel: Option<&CancelToken>,
) -> Result<(u16, String), String> {
    let body: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
        .collect();
    let mut request = Request::new("POST", url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .timeout(Some(REQUEST_TIMEOUT));
    request.body = body.join("&").into_bytes();
    exchange(request, cancel)
}

/// A JSON POST with the same answer.
pub fn post_json(
    url: &str,
    body: &Value,
    cancel: Option<&CancelToken>,
) -> Result<(u16, String), String> {
    let request = Request::new("POST", url)
        .header("Accept", "application/json")
        .json(body)
        .timeout(Some(REQUEST_TIMEOUT));
    exchange(request, cancel)
}

/// A GET with the headers the flow needs, and the same answer.
pub fn get_json(
    url: &str,
    headers: &[(&str, &str)],
    cancel: Option<&CancelToken>,
) -> Result<(u16, String), String> {
    let mut request = Request::new("GET", url)
        .header("Accept", "application/json")
        .timeout(Some(REQUEST_TIMEOUT));
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    exchange(request, cancel)
}

fn exchange(request: Request, cancel: Option<&CancelToken>) -> Result<(u16, String), String> {
    let response = send(&request, cancel).map_err(|error| format!("Error: {error}"))?;
    let status = response.status;
    let text = response.text().map_err(|error| format!("Error: {error}"))?;
    Ok((status, text))
}

/// An endpoint a check or a gateway moves, from `PI_OAUTH_<PROVIDER>_<FIELD>`: provider
/// `kimi-coding` and field `token_url` read `PI_OAUTH_KIMI_CODING_TOKEN_URL`, and an unset or empty
/// value keeps the production one.
pub fn env_url(provider: &str, field: &str, default: &str) -> String {
    let name = format!("PI_OAUTH_{}_{}", env_part(provider), env_part(field));
    std::env::var(&name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn env_part(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// First value of `name` in a query string (`a=1&b=2`), decoded, like `URLSearchParams.get`.
pub fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (form_decode(key) == name).then(|| form_decode(value))
        })
}
