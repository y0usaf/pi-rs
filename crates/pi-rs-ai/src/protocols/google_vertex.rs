//! Port of `providers/google-vertex.ts` (`google-vertex`).
//!
//! Vertex shares Google's message conversion and streamed response state
//! machine; this module contains only Vertex URL/auth/request policy.

use base64::Engine;
use pi_rs_ai_types::{
    AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Model, ModelThinkingLevel,
    StopReason, ThinkingLevel, Usage, clamp_thinking_level, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use super::google::{
    GoogleOptions, GoogleThinking, GoogleToolChoice, budget, build_params, close_current,
    effort_level, format_http_error, process_chunk,
};
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::{ProtocolError, merge_header_map};
use crate::transport::{
    AssistantMessageEventStream, TransportError, create_assistant_message_event_stream,
    response_sse_reader,
};

const DEFAULT_TIMEOUT_MS: u64 = 600_000;
const CREDENTIALS_MARKER: &str = "gcp-vertex-credentials";

#[derive(Clone, Default)]
pub struct GoogleVertexOptions {
    pub base: StreamOptions,
    pub tool_choice: Option<GoogleToolChoice>,
    pub thinking: Option<GoogleThinking>,
    pub project: Option<String>,
    pub location: Option<String>,
}

fn explicit_api_key(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|key| {
        !key.is_empty()
            && *key != CREDENTIALS_MARKER
            && !(key.starts_with('<') && key.ends_with('>'))
    })
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn resolve_project(options: &GoogleVertexOptions) -> Result<String, ProtocolError> {
    options
        .project
        .clone()
        .or_else(|| env_non_empty("GOOGLE_CLOUD_PROJECT"))
        .or_else(|| env_non_empty("GCLOUD_PROJECT"))
        .ok_or_else(|| {
            ProtocolError(
                "Vertex AI requires a project ID. Set GOOGLE_CLOUD_PROJECT/GCLOUD_PROJECT or pass project in options."
                    .into(),
            )
        })
}

fn resolve_location(options: &GoogleVertexOptions) -> Result<String, ProtocolError> {
    options
        .location
        .clone()
        .or_else(|| env_non_empty("GOOGLE_CLOUD_LOCATION"))
        .ok_or_else(|| {
            ProtocolError(
                "Vertex AI requires a location. Set GOOGLE_CLOUD_LOCATION or pass location in options."
                    .into(),
            )
        })
}

fn custom_base(model: &Model) -> Option<&str> {
    let value = model.base_url.trim();
    (!value.is_empty() && !value.contains("{location}")).then_some(value)
}

fn segment_is_api_version(segment: &str) -> bool {
    let Some(rest) = segment.strip_prefix('v') else {
        return false;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && (digits == rest.len()
            || rest[digits..]
                .strip_prefix("beta")
                .is_some_and(|tail| tail.chars().all(|ch| ch.is_ascii_digit())))
}

fn base_includes_api_version(base: &str) -> bool {
    base.split('/').any(segment_is_api_version)
}

fn request_url(
    model: &Model,
    api_key: bool,
    project: Option<&str>,
    location: Option<&str>,
) -> String {
    let custom = custom_base(model);
    let base = if let Some(base) = custom {
        base.trim_end_matches('/').to_string()
    } else if api_key || location == Some("global") {
        "https://aiplatform.googleapis.com".to_string()
    } else if matches!(location, Some("us" | "eu" | "asia")) {
        format!(
            "https://aiplatform.{}.rep.googleapis.com",
            location.unwrap_or_default()
        )
    } else {
        format!(
            "https://{}-aiplatform.googleapis.com",
            location.unwrap_or_default()
        )
    };
    let version = if custom.is_some_and(base_includes_api_version) {
        ""
    } else {
        "/v1"
    };
    let scope = if custom.is_none() && !api_key {
        format!(
            "/projects/{}/locations/{}",
            project.unwrap_or_default(),
            location.unwrap_or_default()
        )
    } else {
        String::new()
    };
    format!(
        "{base}{version}{scope}/publishers/google/models/{}:streamGenerateContent?alt=sse",
        model.id
    )
}

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

fn credential_path() -> Result<std::path::PathBuf, ProtocolError> {
    if let Some(path) = env_non_empty("GOOGLE_APPLICATION_CREDENTIALS") {
        return Ok(path.into());
    }
    env_non_empty("HOME")
        .map(std::path::PathBuf::from)
        .map(|home| home.join(".config/gcloud/application_default_credentials.json"))
        .ok_or_else(|| ProtocolError("Could not load the default credentials.".into()))
}

fn credential_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, ProtocolError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProtocolError(format!("ADC credentials are missing {key}")))
}

async fn token_response(request: reqwest::RequestBuilder) -> Result<Value, ProtocolError> {
    let response = request
        .send()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let message = value
            .get("error_description")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or(&text);
        return Err(ProtocolError(message.to_string()));
    }
    Ok(value)
}

async fn refresh_user_token(credentials: &Value) -> Result<String, ProtocolError> {
    let kind = credential_field(credentials, "type")?;
    let endpoint = if kind == "external_account_authorized_user" {
        credential_field(credentials, "token_url")?
    } else if kind == "authorized_user" {
        GOOGLE_OAUTH_TOKEN_URL
    } else {
        return Err(ProtocolError(format!(
            "Unsupported Application Default Credentials type: {kind}"
        )));
    };
    let client_id = credential_field(credentials, "client_id")?;
    let client_secret = credential_field(credentials, "client_secret")?;
    let refresh_token = credential_field(credentials, "refresh_token")?;
    let body = {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        if kind == "authorized_user" {
            serializer
                .append_pair("refresh_token", refresh_token)
                .append_pair("client_id", client_id)
                .append_pair("client_secret", client_secret)
                .append_pair("grant_type", "refresh_token");
        } else {
            serializer
                .append_pair("grant_type", "refresh_token")
                .append_pair("refresh_token", refresh_token);
        }
        serializer.finish()
    };
    let mut request = reqwest::Client::new()
        .post(endpoint)
        .header("accept", "application/json")
        .header(
            "content-type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header("x-goog-api-client", "gl-node/22.23.1");
    if kind == "external_account_authorized_user" {
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{client_id}:{client_secret}"));
        request = request.header("authorization", format!("Basic {basic}"));
    }
    let value = token_response(request.body(body)).await?;
    credential_field(&value, "access_token").map(str::to_string)
}

fn service_account_assertion(credentials: &Value) -> Result<String, ProtocolError> {
    let email = credential_field(credentials, "client_email")?;
    let private_key = credential_field(credentials, "private_key")?;
    let key_der = base64::engine::general_purpose::STANDARD
        .decode(
            private_key
                .lines()
                .filter(|line| !line.starts_with("-----"))
                .collect::<String>(),
        )
        .map_err(|error| ProtocolError(error.to_string()))?;
    let key = ring::signature::RsaKeyPair::from_pkcs8(&key_der)
        .map_err(|error| ProtocolError(error.to_string()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| ProtocolError(error.to_string()))?
        .as_secs();
    let header = serde_json::json!({"alg":"RS256"});
    let payload = serde_json::json!({
        "iss": email,
        "scope": CLOUD_PLATFORM_SCOPE,
        "aud": GOOGLE_OAUTH_TOKEN_URL,
        "exp": now + 3600,
        "iat": now,
    });
    let encode =
        |value: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string());
    let signing_input = format!("{}.{}", encode(&header), encode(&payload));
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signing_input.as_bytes(),
        &mut signature,
    )
    .map_err(|error| ProtocolError(error.to_string()))?;
    Ok(format!(
        "{signing_input}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn service_account_token_url(credentials: &Value) -> &str {
    credentials
        .get("token_uri")
        .and_then(Value::as_str)
        .unwrap_or(GOOGLE_OAUTH_TOKEN_URL)
}

async fn service_account_token(credentials: &Value) -> Result<String, ProtocolError> {
    let body = {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer
            .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
            .append_pair("assertion", &service_account_assertion(credentials)?);
        serializer.finish()
    };
    let value = token_response(
        reqwest::Client::new()
            .post(service_account_token_url(credentials))
            .header("accept", "application/json")
            .header(
                "content-type",
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .header("x-goog-api-client", "gl-node/22.23.1")
            .body(body),
    )
    .await?;
    credential_field(&value, "access_token").map(str::to_string)
}

fn parse_external_subject_token(
    raw: &str,
    source: &Value,
    source_name: &str,
) -> Result<String, ProtocolError> {
    let format = source.get("format");
    let format_type = format
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("text");
    if !matches!(format_type, "text" | "json") {
        return Err(ProtocolError(format!(
            "Invalid credential_source format \"{format_type}\""
        )));
    }
    if format_type == "json" {
        let field = format
            .and_then(|value| value.get("subject_token_field_name"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProtocolError(
                    "Missing subject_token_field_name for JSON credential_source format".into(),
                )
            })?;
        let value: Value =
            serde_json::from_str(raw).map_err(|error| ProtocolError(error.to_string()))?;
        return value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                ProtocolError(format!(
                    "Unable to parse the subject_token from the credential_source {source_name}"
                ))
            });
    }
    if raw.is_empty() {
        return Err(ProtocolError(format!(
            "Unable to parse the subject_token from the credential_source {source_name}"
        )));
    }
    Ok(raw.to_string())
}

const EXECUTABLE_ALLOW_ENV: &str = "GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES";
const DEFAULT_EXECUTABLE_TIMEOUT_MS: u64 = 30_000;
const MIN_EXECUTABLE_TIMEOUT_MS: u64 = 5_000;
const MAX_EXECUTABLE_TIMEOUT_MS: u64 = 120_000;

fn parse_executable_command(command: &str) -> Result<Vec<String>, ProtocolError> {
    let bytes = command.as_bytes();
    let mut pieces = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == bytes.len() {
            break;
        }
        if bytes[index] == b'"' {
            let start = index + 1;
            index = start;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            if index < bytes.len() {
                pieces.push(command[start..index].to_string());
                index += 1;
                continue;
            }
            index = start;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'"' {
            index += 1;
        }
        if start != index {
            pieces.push(command[start..index].to_string());
        } else {
            index += 1;
        }
    }
    if pieces.is_empty() {
        return Err(ProtocolError(format!(
            "Provided command: \"{command}\" could not be parsed."
        )));
    }
    Ok(pieces)
}

fn executable_response_token(
    response: &Value,
    output_file: bool,
    cached: bool,
) -> Result<Option<String>, ProtocolError> {
    let version = response
        .get("version")
        .and_then(Value::as_u64)
        .filter(|version| *version != 0)
        .ok_or_else(|| {
            ProtocolError("Executable response must contain a 'version' field.".into())
        })?;
    let success = response
        .get("success")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ProtocolError("Executable response must contain a 'success' field.".into())
        })?;
    if !success {
        let code = response
            .get("code")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProtocolError(
                    "Executable response must contain a 'code' field when unsuccessful.".into(),
                )
            })?;
        let message = response
            .get("message")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProtocolError(
                    "Executable response must contain a 'message' field when unsuccessful.".into(),
                )
            })?;
        if cached {
            return Ok(None);
        }
        return Err(ProtocolError(format!(
            "The executable failed with exit code: {code} and error message: {message}."
        )));
    }
    let token_type = response
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let token_field = match token_type {
        "urn:ietf:params:oauth:token-type:saml2" => "saml_response",
        "urn:ietf:params:oauth:token-type:id_token" | "urn:ietf:params:oauth:token-type:jwt" => {
            "id_token"
        }
        _ => {
            return Err(ProtocolError(
                "Executable response must contain a 'token_type' field when successful and it must be one of urn:ietf:params:oauth:token-type:id_token, urn:ietf:params:oauth:token-type:jwt, or urn:ietf:params:oauth:token-type:saml2."
                    .into(),
            ));
        }
    };
    let token = response
        .get(token_field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            let message = if token_field == "saml_response" {
                "Executable response must contain a 'saml_response' field when token_type=urn:ietf:params:oauth:token-type:saml2."
                    .to_string()
            } else {
                "Executable response must contain a 'id_token' field when token_type=urn:ietf:params:oauth:token-type:id_token or urn:ietf:params:oauth:token-type:jwt."
                    .to_string()
            };
            ProtocolError(message)
        })?;
    let expiration = response.get("expiration_time").and_then(Value::as_f64);
    if output_file && expiration.is_none_or(|value| value == 0.0) {
        return Err(ProtocolError(
            "The executable response must contain the `expiration_time` field for successful responses when an output_file has been specified in the configuration."
                .into(),
        ));
    }
    if let Some(expiration) = expiration {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| ProtocolError(error.to_string()))?
            .as_secs_f64()
            .round();
        if expiration < now {
            return Ok(None);
        }
    }
    if version > 1 {
        return Err(ProtocolError(
            "Version of executable is not currently supported, maximum supported version is 1."
                .into(),
        ));
    }
    Ok(Some(token.to_string()))
}

async fn cached_executable_token(path: &str) -> Result<Option<String>, ProtocolError> {
    let Ok(path) = tokio::fs::canonicalize(path).await else {
        return Ok(None);
    };
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    if raw.is_empty() {
        return Ok(None);
    }
    let response: Value = serde_json::from_str(&raw).map_err(|_| {
        ProtocolError(format!(
            "The output file contained an invalid response: {raw}"
        ))
    })?;
    executable_response_token(&response, true, true)
}

async fn executable_subject_token(
    credentials: &Value,
    executable: &Value,
) -> Result<String, ProtocolError> {
    let command = executable
        .get("command")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProtocolError("No valid Pluggable Auth \"credential_source\" provided.".into())
        })?;
    let timeout_ms = executable
        .get("timeout_millis")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_EXECUTABLE_TIMEOUT_MS);
    if !(MIN_EXECUTABLE_TIMEOUT_MS..=MAX_EXECUTABLE_TIMEOUT_MS).contains(&timeout_ms) {
        return Err(ProtocolError(format!(
            "Timeout must be between {MIN_EXECUTABLE_TIMEOUT_MS} and {MAX_EXECUTABLE_TIMEOUT_MS} milliseconds."
        )));
    }
    if env_non_empty(EXECUTABLE_ALLOW_ENV).as_deref() != Some("1") {
        return Err(ProtocolError(
            "Pluggable Auth executables need to be explicitly allowed to run by setting the GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES environment Variable to 1."
                .into(),
        ));
    }
    let output_file = executable.get("output_file").and_then(Value::as_str);
    if let Some(path) = output_file
        && let Some(token) = cached_executable_token(path).await?
    {
        return Ok(token);
    }
    let pieces = parse_executable_command(command)?;
    let mut process = tokio::process::Command::new(&pieces[0]);
    process.args(&pieces[1..]).kill_on_drop(true);
    process
        .env(
            "GOOGLE_EXTERNAL_ACCOUNT_AUDIENCE",
            credential_field(credentials, "audience")?,
        )
        .env(
            "GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE",
            credential_field(credentials, "subject_token_type")?,
        )
        .env("GOOGLE_EXTERNAL_ACCOUNT_INTERACTIVE", "0");
    if let Some(path) = output_file {
        process.env("GOOGLE_EXTERNAL_ACCOUNT_OUTPUT_FILE", path);
    }
    if let Some(url) = credentials
        .get("service_account_impersonation_url")
        .and_then(Value::as_str)
        && let Some(email) = url
            .strip_suffix(":generateAccessToken")
            .and_then(|value| value.rsplit("serviceAccounts/").next())
            .filter(|value| *value != url)
    {
        process.env("GOOGLE_EXTERNAL_ACCOUNT_IMPERSONATED_EMAIL", email);
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        process.output(),
    )
    .await
    .map_err(|_| {
        ProtocolError("The executable failed to finish within the timeout specified.".into())
    })?
    .map_err(|error| ProtocolError(error.to_string()))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        let code = output
            .status
            .code()
            .map_or_else(|| "null".to_string(), |value| value.to_string());
        return Err(ProtocolError(format!(
            "The executable failed with exit code: {code} and error message: {combined}."
        )));
    }
    let response: Value = serde_json::from_str(&combined).map_err(|_| {
        ProtocolError(format!(
            "The executable returned an invalid response: {combined}"
        ))
    })?;
    executable_response_token(&response, output_file.is_some(), false)?
        .ok_or_else(|| ProtocolError("Executable response is expired.".into()))
}
async fn external_subject_token(
    credentials: &Value,
) -> Result<(String, &'static str), ProtocolError> {
    let source = credentials
        .get("credential_source")
        .ok_or_else(|| ProtocolError("A credential source must be specified.".into()))?;
    if let Some(executable) = source.get("executable") {
        return Ok((
            executable_subject_token(credentials, executable).await?,
            "executable",
        ));
    }
    if let Some(path) = source.get("file").and_then(Value::as_str) {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| ProtocolError(error.to_string()))?;
        return Ok((parse_external_subject_token(&raw, source, "file")?, "file"));
    }
    if let Some(url) = source.get("url").and_then(Value::as_str) {
        let json = source.pointer("/format/type").and_then(Value::as_str) == Some("json");
        let mut request = reqwest::Client::new()
            .get(url)
            .header("accept", if json { "application/json" } else { "*/*" })
            .header("x-goog-api-client", "gl-node/22.23.1");
        if let Some(headers) = source.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                if let Some(value) = value.as_str() {
                    request = request.header(name, value);
                }
            }
        }
        let raw = request
            .send()
            .await
            .map_err(|error| ProtocolError(error.to_string()))?
            .error_for_status()
            .map_err(|error| ProtocolError(error.to_string()))?
            .text()
            .await
            .map_err(|error| ProtocolError(error.to_string()))?;
        return Ok((parse_external_subject_token(&raw, source, "URL")?, "url"));
    }
    Err(ProtocolError(
        "No valid Identity Pool \"credential_source\" provided, must be either file, url, or certificate."
            .into(),
    ))
}

async fn external_account_token(credentials: &Value) -> Result<String, ProtocolError> {
    let audience = credential_field(credentials, "audience")?;
    let subject_token_type = credential_field(credentials, "subject_token_type")?;
    let token_url = credential_field(credentials, "token_url")?;
    let (subject_token, source_type) = external_subject_token(credentials).await?;
    let body = {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer
            .append_pair(
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange",
            )
            .append_pair("audience", audience)
            .append_pair("scope", CLOUD_PLATFORM_SCOPE)
            .append_pair(
                "requested_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            )
            .append_pair("subject_token", &subject_token)
            .append_pair("subject_token_type", subject_token_type);
        serializer.finish()
    };
    let impersonation_url = credentials
        .get("service_account_impersonation_url")
        .and_then(Value::as_str);
    let configured_lifetime = credentials
        .pointer("/service_account_impersonation/token_lifetime_seconds")
        .and_then(Value::as_u64);
    let mut request = reqwest::Client::new()
        .post(token_url)
        .header("accept", "application/json")
        .header(
            "content-type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header(
            "x-goog-api-client",
            format!(
                "gl-node/22.23.1 auth/10.6.2 google-byoid-sdk source/{source_type} sa-impersonation/{} config-lifetime/{}",
                impersonation_url.is_some(),
                configured_lifetime.is_some()
            ),
        );

    if let Some(client_id) = credentials.get("client_id").and_then(Value::as_str) {
        let secret = credentials
            .get("client_secret")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let basic =
            base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{secret}"));
        request = request.header("authorization", format!("Basic {basic}"));
    }
    let value = token_response(request.body(body)).await?;
    let mut token = credential_field(&value, "access_token")?.to_string();
    if let Some(url) = impersonation_url {
        let body = serde_json::json!({
            "scope": [CLOUD_PLATFORM_SCOPE],
            "lifetime": format!("{}s", configured_lifetime.unwrap_or(3600)),
        })
        .to_string();
        let value = token_response(
            reqwest::Client::new()
                .post(url)
                .bearer_auth(&token)
                .header("accept", "application/json")
                .header("content-type", "application/json")
                .header("x-goog-api-client", "gl-node/22.23.1")
                .body(body),
        )
        .await?;
        token = credential_field(&value, "accessToken")?.to_string();
    }
    if let Some(project_number) = audience
        .split("/projects/")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .filter(|value| !value.is_empty())
    {
        let base = credentials
            .get("cloud_resource_manager_url")
            .and_then(Value::as_str)
            .unwrap_or("https://cloudresourcemanager.googleapis.com/v1/projects/");
        token_response(
            reqwest::Client::new()
                .get(format!("{base}{project_number}"))
                .bearer_auth(&token)
                .header("accept", "application/json")
                .header("x-goog-api-client", "gl-node/22.23.1"),
        )
        .await?;
    }
    Ok(token)
}

async fn adc_access_token() -> Result<String, ProtocolError> {
    let path = credential_path()?;
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    let credentials: Value =
        serde_json::from_str(&raw).map_err(|error| ProtocolError(error.to_string()))?;
    match credential_field(&credentials, "type")? {
        "authorized_user" | "external_account_authorized_user" => {
            refresh_user_token(&credentials).await
        }
        "service_account" => service_account_token(&credentials).await,
        "external_account" => external_account_token(&credentials).await,
        kind => Err(ProtocolError(format!(
            "Unsupported Application Default Credentials type: {kind}"
        ))),
    }
}

fn request_headers(
    model: &Model,
    options: &GoogleVertexOptions,
    key: Option<&str>,
    bearer: Option<&str>,
) -> Result<HeaderMap, ProtocolError> {
    let mut values = vec![
        ("accept".to_string(), "*/*".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
        (
            "x-goog-api-client".to_string(),
            "google-genai-sdk/1.52.0 gl-node/v22.23.1".to_string(),
        ),
    ];
    merge_header_map(&mut values, model.headers.as_ref());
    merge_header_map(&mut values, options.base.headers.as_ref());
    if !values
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        && let Some(token) = bearer
    {
        values.push(("authorization".into(), format!("Bearer {token}")));
    }
    if !values
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("x-goog-api-key"))
        && let Some(key) = key
    {
        values.push(("x-goog-api-key".into(), key.to_string()));
    }
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| ProtocolError(error.to_string()))?,
            HeaderValue::from_str(&value).map_err(|error| ProtocolError(error.to_string()))?,
        );
    }
    Ok(headers)
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &GoogleVertexOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(crate::transport::AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request aborted".into()));
    }
    let key = explicit_api_key(options.base.api_key.as_deref());
    let (project, location, bearer) = if key.is_some() {
        (None, None, None)
    } else {
        let project = resolve_project(options)?;
        let location = resolve_location(options)?;
        let bearer = adc_access_token().await?;
        (Some(project), Some(location), Some(bearer))
    };
    let google_options = GoogleOptions {
        base: options.base.clone(),
        tool_choice: options.tool_choice,
        thinking: options.thinking.clone(),
    };
    let mut params = build_params(model, context, &google_options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model)
    {
        params = next;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            options.base.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
        ))
        .build()
        .map_err(|error| ProtocolError(error.to_string()))?;
    let response = client
        .post(request_url(
            model,
            key.is_some(),
            project.as_deref(),
            location.as_deref(),
        ))
        .headers(request_headers(model, options, key, bearer.as_deref())?)
        .body(params.to_string())
        .send()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(ProtocolError(format_http_error(TransportError::Status {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or_default().to_string(),
            body: response.text().await.unwrap_or_default(),
        })));
    }
    stream.push(AssistantMessageEvent::Start {
        partial: output.clone(),
    });
    let mut reader = response_sse_reader(response, options.base.signal.clone());
    let mut current = None;
    while let Some(event) = reader
        .next()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?
    {
        let chunk =
            serde_json::from_str(&event.data).map_err(|error| ProtocolError(error.to_string()))?;
        process_chunk(model, &chunk, stream, output, &mut current)?;
    }
    if let Some(index) = current {
        close_current(stream, output, index);
    }
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(crate::transport::AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request was aborted".into()));
    }
    if matches!(output.stop_reason, StopReason::Aborted | StopReason::Error) {
        return Err(ProtocolError("An unknown error occurred".into()));
    }
    Ok(())
}

pub fn stream_google_vertex(
    model: &Model,
    context: &Context,
    options: Option<GoogleVertexOptions>,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let task = stream.clone();
    let model = model.clone();
    let context = context.clone();
    let options = options.unwrap_or_default();
    tokio::spawn(async move {
        let mut output = AssistantMessage {
            role: AssistantRole::Assistant,
            content: Vec::new(),
            api: model.api.clone(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            response_model: None,
            response_id: None,
            diagnostics: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: now_ms(),
        };
        match drive(&model, &context, &options, &task, &mut output).await {
            Ok(()) => task.push(AssistantMessageEvent::Done {
                reason: output.stop_reason,
                message: output,
            }),
            Err(error) => {
                output.stop_reason = if options
                    .base
                    .signal
                    .as_ref()
                    .is_some_and(crate::transport::AbortSignal::is_aborted)
                {
                    StopReason::Aborted
                } else {
                    StopReason::Error
                };
                output.error_message = Some(error.to_string());
                task.push(AssistantMessageEvent::Error {
                    reason: output.stop_reason,
                    error: output,
                });
            }
        }
        task.end();
    });
    stream
}

pub fn stream_simple_google_vertex(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let base = build_base_options(model, options.as_ref(), None);
    let thinking = if let Some(reasoning) = options.as_ref().and_then(|value| value.reasoning) {
        let clamped = clamp_thinking_level(model, ModelThinkingLevel::from(reasoning));
        let effort = match clamped {
            ModelThinkingLevel::Minimal => ThinkingLevel::Minimal,
            ModelThinkingLevel::Low => ThinkingLevel::Low,
            ModelThinkingLevel::Medium => ThinkingLevel::Medium,
            ModelThinkingLevel::High
            | ModelThinkingLevel::XHigh
            | ModelThinkingLevel::Max
            | ModelThinkingLevel::Off => ThinkingLevel::High,
        };
        if model.id.to_ascii_lowercase().contains("gemini-3") {
            GoogleThinking {
                enabled: true,
                budget_tokens: None,
                level: Some(effort_level(model, effort)),
            }
        } else {
            GoogleThinking {
                enabled: true,
                budget_tokens: Some(budget(
                    model,
                    effort,
                    options
                        .as_ref()
                        .and_then(|value| value.thinking_budgets.as_ref()),
                )),
                level: None,
            }
        }
    } else {
        GoogleThinking {
            enabled: false,
            budget_tokens: None,
            level: None,
        }
    };
    Ok(stream_google_vertex(
        model,
        context,
        Some(GoogleVertexOptions {
            base,
            thinking: Some(thinking),
            ..GoogleVertexOptions::default()
        }),
    ))
}
