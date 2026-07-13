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

fn pem_certificates(raw: &[u8]) -> Result<Vec<Vec<u8>>, ProtocolError> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let text = std::str::from_utf8(raw).map_err(|error| ProtocolError(error.to_string()))?;
    let mut certificates = Vec::new();
    let mut remaining = text;
    while let Some(begin) = remaining.find(BEGIN) {
        let content = &remaining[begin + BEGIN.len()..];
        let Some(end) = content.find(END) else {
            return Err(ProtocolError("Invalid certificate PEM block".into()));
        };
        let encoded = content[..end].lines().map(str::trim).collect::<String>();
        certificates.push(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| ProtocolError(error.to_string()))?,
        );
        remaining = &content[end + END.len()..];
    }
    if certificates.is_empty() {
        return Err(ProtocolError("Invalid certificate PEM block".into()));
    }
    Ok(certificates)
}

fn certificate_config_default_path() -> std::path::PathBuf {
    if let Some(directory) = env_non_empty("CLOUDSDK_CONFIG") {
        return std::path::PathBuf::from(directory).join("certificate_config.json");
    }
    if cfg!(windows) {
        return std::path::PathBuf::from(env_non_empty("APPDATA").unwrap_or_default())
            .join("gcloud/certificate_config.json");
    }
    std::path::PathBuf::from(env_non_empty("HOME").unwrap_or_default())
        .join(".config/gcloud/certificate_config.json")
}

async fn valid_file(path: &std::path::Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file())
}

async fn certificate_config_path(certificate: &Value) -> Result<std::path::PathBuf, ProtocolError> {
    let use_default = certificate
        .get("use_default_certificate_config")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let configured = certificate
        .get("certificate_config_location")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    if !use_default && configured.is_none() {
        return Err(ProtocolError(
            "Either `useDefaultCertificateConfig` must be true or a `certificateConfigLocation` must be provided.".into(),
        ));
    }
    if use_default && configured.is_some() {
        return Err(ProtocolError(
            "Both `useDefaultCertificateConfig` and `certificateConfigLocation` cannot be provided.".into(),
        ));
    }
    if let Some(path) = configured {
        let path = std::path::PathBuf::from(path);
        if valid_file(&path).await {
            return Ok(path);
        }
        return Err(ProtocolError(format!(
            "Provided certificate config path is invalid: {}",
            path.display()
        )));
    }
    if let Some(path) = env_non_empty("GOOGLE_API_CERTIFICATE_CONFIG") {
        let path = std::path::PathBuf::from(path);
        if valid_file(&path).await {
            return Ok(path);
        }
        return Err(ProtocolError(format!(
            "Path from environment variable \"GOOGLE_API_CERTIFICATE_CONFIG\" is invalid: {}",
            path.display()
        )));
    }
    let path = certificate_config_default_path();
    if valid_file(&path).await {
        return Ok(path);
    }
    Err(ProtocolError(format!(
        "Could not find certificate configuration file. Searched override path, the \"GOOGLE_API_CERTIFICATE_CONFIG\" env var, and the gcloud path ({}).",
        path.display()
    )))
}

async fn certificate_subject_token(
    certificate: &Value,
) -> Result<(String, reqwest::Identity), ProtocolError> {
    let config_path = certificate_config_path(certificate).await?;
    let config_raw = tokio::fs::read_to_string(&config_path).await.map_err(|_| {
        ProtocolError(format!(
            "Failed to read certificate config file at: {}",
            config_path.display()
        ))
    })?;
    let config: Value = serde_json::from_str(&config_raw).map_err(|error| {
        ProtocolError(format!(
            "Failed to parse certificate config from {}: {error}",
            config_path.display()
        ))
    })?;
    let cert_path = config
        .pointer("/cert_configs/workload/cert_path")
        .and_then(Value::as_str);
    let key_path = config
        .pointer("/cert_configs/workload/key_path")
        .and_then(Value::as_str);
    let (Some(cert_path), Some(key_path)) = (cert_path, key_path) else {
        return Err(ProtocolError(format!(
            "Certificate config file ({}) is missing required \"cert_path\" or \"key_path\" in the workload config.",
            config_path.display()
        )));
    };
    let cert = tokio::fs::read(cert_path).await.map_err(|error| {
        ProtocolError(format!(
            "Failed to read certificate file at {cert_path}: {error}"
        ))
    })?;
    let key = tokio::fs::read(key_path).await.map_err(|error| {
        ProtocolError(format!(
            "Failed to read private key file at {key_path}: {error}"
        ))
    })?;
    let leaf = pem_certificates(&cert)?
        .into_iter()
        .next()
        .ok_or_else(|| ProtocolError("Invalid certificate PEM block".into()))?;
    let mut chain = vec![leaf.clone()];
    if let Some(path) = certificate
        .get("trust_chain_path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        let raw = tokio::fs::read(path).await.map_err(|error| {
            ProtocolError(format!(
                "Failed to process certificate chain from {path}: {error}"
            ))
        })?;
        let parsed = pem_certificates(&raw)?;
        if let Some(index) = parsed.iter().position(|value| *value == leaf) {
            if index != 0 {
                return Err(ProtocolError(format!(
                    "Leaf certificate exists in the trust chain but is not the first entry (found at index {index})."
                )));
            }
            chain = parsed;
        } else {
            chain.extend(parsed);
        }
    }
    let subject = serde_json::to_string(
        &chain
            .iter()
            .map(|value| base64::engine::general_purpose::STANDARD.encode(value))
            .collect::<Vec<_>>(),
    )
    .map_err(|error| ProtocolError(error.to_string()))?;
    let mut identity_pem = cert;
    if !identity_pem.ends_with(b"\n") {
        identity_pem.push(b'\n');
    }
    identity_pem.extend(key);
    let identity = reqwest::Identity::from_pem(&identity_pem)
        .map_err(|error| ProtocolError(error.to_string()))?;
    Ok((subject, identity))
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
#[derive(Debug)]
struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    token: Option<String>,
}

async fn aws_imds_token(source: &Value) -> Result<Option<String>, ProtocolError> {
    let Some(url) = source
        .get("imdsv2_session_token_url")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let token = reqwest::Client::new()
        .put(url)
        .header("accept", "*/*")
        .header("x-aws-ec2-metadata-token-ttl-seconds", "300")
        .header("x-goog-api-client", "gl-node/22.23.1")
        .send()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?
        .error_for_status()
        .map_err(|error| ProtocolError(error.to_string()))?
        .text()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    Ok(Some(token))
}

async fn aws_metadata_text(
    url: &str,
    accept: &str,
    token: Option<&str>,
) -> Result<String, ProtocolError> {
    let mut request = reqwest::Client::new()
        .get(url)
        .header("accept", accept)
        .header("x-goog-api-client", "gl-node/22.23.1");
    if let Some(token) = token {
        request = request.header("x-aws-ec2-metadata-token", token);
    }
    request
        .send()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?
        .error_for_status()
        .map_err(|error| ProtocolError(error.to_string()))?
        .text()
        .await
        .map_err(|error| ProtocolError(error.to_string()))
}

async fn aws_region(source: &Value) -> Result<String, ProtocolError> {
    if let Some(region) =
        env_non_empty("AWS_REGION").or_else(|| env_non_empty("AWS_DEFAULT_REGION"))
    {
        return Ok(region);
    }
    let url = source
        .get("region_url")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProtocolError(
                "Unable to determine AWS region due to missing \"options.credential_source.region_url\""
                    .into(),
            )
        })?;
    let token = aws_imds_token(source).await?;
    let zone = aws_metadata_text(url, "*/*", token.as_deref()).await?;
    zone.get(..zone.len().saturating_sub(1))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProtocolError("Unable to determine AWS region".into()))
}

async fn aws_credentials(source: &Value) -> Result<AwsCredentials, ProtocolError> {
    if let (Some(access_key_id), Some(secret_access_key)) = (
        env_non_empty("AWS_ACCESS_KEY_ID"),
        env_non_empty("AWS_SECRET_ACCESS_KEY"),
    ) {
        return Ok(AwsCredentials {
            access_key_id,
            secret_access_key,
            token: env_non_empty("AWS_SESSION_TOKEN"),
        });
    }
    let base = source.get("url").and_then(Value::as_str).ok_or_else(|| {
        ProtocolError(
            "Unable to determine AWS role name due to missing \"options.credential_source.url\""
                .into(),
        )
    })?;
    let token = aws_imds_token(source).await?;
    let role = aws_metadata_text(base, "*/*", token.as_deref()).await?;
    let raw = aws_metadata_text(
        &format!("{base}/{role}"),
        "application/json",
        token.as_deref(),
    )
    .await?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| ProtocolError(error.to_string()))?;
    Ok(AwsCredentials {
        access_key_id: credential_field(&value, "AccessKeyId")?.to_string(),
        secret_access_key: credential_field(&value, "SecretAccessKey")?.to_string(),
        token: value
            .get("Token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

fn aws_date() -> Result<(String, String), ProtocolError> {
    let value = httpdate::fmt_http_date(std::time::SystemTime::now());
    let pieces = value.split_whitespace().collect::<Vec<_>>();
    if pieces.len() != 6 {
        return Err(ProtocolError("Unable to format AWS request date".into()));
    }
    let month = match pieces[2] {
        "Jan" => "01",
        "Feb" => "02",
        "Mar" => "03",
        "Apr" => "04",
        "May" => "05",
        "Jun" => "06",
        "Jul" => "07",
        "Aug" => "08",
        "Sep" => "09",
        "Oct" => "10",
        "Nov" => "11",
        "Dec" => "12",
        _ => return Err(ProtocolError("Unable to format AWS request date".into())),
    };
    let date = format!("{}{}{}", pieces[3], month, pieces[1]);
    Ok((
        date.clone(),
        format!("{date}T{}Z", pieces[4].replace(':', "")),
    ))
}

fn sha256_hex(value: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, value)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hmac_sha256(key: &[u8], value: &str) -> Vec<u8> {
    ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
        value.as_bytes(),
    )
    .as_ref()
    .to_vec()
}

async fn aws_subject_token(credentials: &Value, source: &Value) -> Result<String, ProtocolError> {
    let environment_id = source
        .get("environment_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = environment_id
        .strip_prefix("aws")
        .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok());
    let verification_template = source
        .get("regional_cred_verification_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    if version.is_none() || verification_template.is_none() {
        return Err(ProtocolError(
            "No valid AWS \"credential_source\" provided".into(),
        ));
    }
    if version != Some(1) {
        return Err(ProtocolError(format!(
            "aws version \"{}\" is not supported in the current build.",
            version.unwrap_or_default()
        )));
    }
    let region = aws_region(source).await?;
    let aws_credentials = aws_credentials(source).await?;
    let verification_url = verification_template
        .unwrap_or_default()
        .replace("{region}", &region);
    let parsed =
        url::Url::parse(&verification_url).map_err(|error| ProtocolError(error.to_string()))?;
    let host = parsed
        .host_str()
        .map(|host| match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
        .ok_or_else(|| ProtocolError("No valid AWS \"credential_source\" provided".into()))?;
    let service = host.split('.').next().unwrap_or_default();
    let (date, amz_date) = aws_date()?;
    let mut canonical = std::collections::BTreeMap::new();
    canonical.insert("host", host.clone());
    canonical.insert("x-amz-date", amz_date.clone());
    if let Some(token) = &aws_credentials.token {
        canonical.insert("x-amz-security-token", token.clone());
    }
    let signed_headers = canonical.keys().copied().collect::<Vec<_>>().join(";");
    let canonical_headers = canonical
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    let query = parsed.query().unwrap_or_default();
    let canonical_request = format!(
        "POST\n{}\n{query}\n{canonical_headers}\n{signed_headers}\n{}",
        parsed.path(),
        sha256_hex(b"")
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let date_key = hmac_sha256(
        format!("AWS4{}", aws_credentials.secret_access_key).as_bytes(),
        &date,
    );
    let region_key = hmac_sha256(&date_key, &region);
    let service_key = hmac_sha256(&region_key, service);
    let signing_key = hmac_sha256(&service_key, "aws4_request");
    let signature = hmac_sha256(&signing_key, &string_to_sign)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        aws_credentials.access_key_id
    );
    let mut headers = vec![
        serde_json::json!({"key":"authorization", "value":authorization}),
        serde_json::json!({"key":"host", "value":host}),
        serde_json::json!({"key":"x-amz-date", "value":amz_date}),
    ];
    if let Some(token) = aws_credentials.token {
        headers.push(serde_json::json!({"key":"x-amz-security-token", "value":token}));
    }
    headers.push(serde_json::json!({
        "key":"x-goog-cloud-target-resource",
        "value":credential_field(credentials, "audience")?,
    }));
    let value = serde_json::json!({
        "url":verification_url,
        "method":"POST",
        "headers":headers,
    })
    .to_string();
    Ok(url::form_urlencoded::byte_serialize(value.as_bytes()).collect())
}

struct ExternalSubjectToken {
    token: String,
    source_type: &'static str,
    identity: Option<reqwest::Identity>,
}

async fn external_subject_token(
    credentials: &Value,
) -> Result<ExternalSubjectToken, ProtocolError> {
    let source = credentials
        .get("credential_source")
        .ok_or_else(|| ProtocolError("A credential source must be specified.".into()))?;
    if source
        .get("environment_id")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(ExternalSubjectToken {
            token: aws_subject_token(credentials, source).await?,
            source_type: "aws",
            identity: None,
        });
    }
    if let Some(executable) = source.get("executable") {
        return Ok(ExternalSubjectToken {
            token: executable_subject_token(credentials, executable).await?,
            source_type: "executable",
            identity: None,
        });
    }
    if let Some(path) = source.get("file").and_then(Value::as_str) {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| ProtocolError(error.to_string()))?;
        return Ok(ExternalSubjectToken {
            token: parse_external_subject_token(&raw, source, "file")?,
            source_type: "file",
            identity: None,
        });
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
        return Ok(ExternalSubjectToken {
            token: parse_external_subject_token(&raw, source, "URL")?,
            source_type: "url",
            identity: None,
        });
    }
    if let Some(certificate) = source.get("certificate") {
        let (token, identity) = certificate_subject_token(certificate).await?;
        return Ok(ExternalSubjectToken {
            token,
            source_type: "certificate",
            identity: Some(identity),
        });
    }
    Err(ProtocolError(
        "No valid Identity Pool \"credential_source\" provided, must be either file, url, or certificate."
            .into(),
    ))
}

struct AdcAccessToken {
    token: String,
    identity: Option<reqwest::Identity>,
}

async fn external_account_token(credentials: &Value) -> Result<AdcAccessToken, ProtocolError> {
    let audience = credential_field(credentials, "audience")?;
    let subject_token_type = credential_field(credentials, "subject_token_type")?;
    let token_url = credential_field(credentials, "token_url")?;
    let subject = external_subject_token(credentials).await?;
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
            .append_pair("subject_token", &subject.token)
            .append_pair("subject_token_type", subject_token_type);
        serializer.finish()
    };
    let impersonation_url = credentials
        .get("service_account_impersonation_url")
        .and_then(Value::as_str);
    let configured_lifetime = credentials
        .pointer("/service_account_impersonation/token_lifetime_seconds")
        .and_then(Value::as_u64);
    let mut builder = reqwest::Client::builder();
    if let Some(identity) = subject.identity.clone() {
        builder = builder.identity(identity);
    }
    let client = builder
        .build()
        .map_err(|error| ProtocolError(error.to_string()))?;
    let mut request = client
        .post(token_url)
        .header("accept", "application/json")
        .header(
            "content-type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header(
            "x-goog-api-client",
            format!(
                "gl-node/22.23.1 auth/10.6.2 google-byoid-sdk source/{} sa-impersonation/{} config-lifetime/{}",
                subject.source_type,
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
            client
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
        let mut request = client
            .get(format!("{base}{project_number}"))
            .bearer_auth(&token)
            .header("accept", "application/json");
        if subject.identity.is_none() {
            request = request.header("x-goog-api-client", "gl-node/22.23.1");
        }
        token_response(request).await?;
    }
    Ok(AdcAccessToken {
        token,
        identity: subject.identity,
    })
}

async fn adc_access_token() -> Result<AdcAccessToken, ProtocolError> {
    let path = credential_path()?;
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    let credentials: Value =
        serde_json::from_str(&raw).map_err(|error| ProtocolError(error.to_string()))?;
    let token = match credential_field(&credentials, "type")? {
        "authorized_user" | "external_account_authorized_user" => {
            refresh_user_token(&credentials).await?
        }
        "service_account" => service_account_token(&credentials).await?,
        "external_account" => return external_account_token(&credentials).await,
        kind => {
            return Err(ProtocolError(format!(
                "Unsupported Application Default Credentials type: {kind}"
            )));
        }
    };
    Ok(AdcAccessToken {
        token,
        identity: None,
    })
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
    let (project, location, auth) = if key.is_some() {
        (None, None, None)
    } else {
        let project = resolve_project(options)?;
        let location = resolve_location(options)?;
        let auth = adc_access_token().await?;
        (Some(project), Some(location), Some(auth))
    };
    let google_options = GoogleOptions {
        base: options.base.clone(),
        tool_choice: options.tool_choice,
        thinking: options.thinking.clone(),
    };
    let mut params = build_params(model, context, &google_options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model.clone()).await
    {
        params = next;
    }
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_millis(
        options.base.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
    ));
    if let Some(identity) = auth.as_ref().and_then(|value| value.identity.clone()) {
        builder = builder.identity(identity);
    }
    let client = builder
        .build()
        .map_err(|error| ProtocolError(error.to_string()))?;
    let response = client
        .post(request_url(
            model,
            key.is_some(),
            project.as_deref(),
            location.as_deref(),
        ))
        .headers(request_headers(
            model,
            options,
            key,
            auth.as_ref().map(|value| value.token.as_str()),
        )?)
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
