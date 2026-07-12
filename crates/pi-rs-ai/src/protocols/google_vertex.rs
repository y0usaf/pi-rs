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

fn external_subject_token(credentials: &Value) -> Result<String, ProtocolError> {
    let source = credentials
        .get("credential_source")
        .ok_or_else(|| ProtocolError("A credential source must be specified.".into()))?;
    let path = credential_field(source, "file")?;
    let raw = std::fs::read_to_string(path).map_err(|error| ProtocolError(error.to_string()))?;
    let format = source.get("format");
    if format
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        == Some("json")
    {
        let field = format
            .and_then(|value| value.get("subject_token_field_name"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProtocolError(
                    "Missing subject_token_field_name for JSON credential_source format".into(),
                )
            })?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|error| ProtocolError(error.to_string()))?;
        return credential_field(&value, field).map(str::to_string);
    }
    if raw.is_empty() {
        return Err(ProtocolError(
            "Unable to parse the subject_token from the credential_source file".into(),
        ));
    }
    Ok(raw)
}

async fn external_account_token(credentials: &Value) -> Result<String, ProtocolError> {
    let audience = credential_field(credentials, "audience")?;
    let subject_token_type = credential_field(credentials, "subject_token_type")?;
    let token_url = credential_field(credentials, "token_url")?;
    let subject_token = external_subject_token(credentials)?;
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
                "gl-node/22.23.1 auth/10.6.2 google-byoid-sdk source/file sa-impersonation/{} config-lifetime/{}",
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
