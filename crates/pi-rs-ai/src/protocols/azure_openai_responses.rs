//! Port of `providers/azure-openai-responses.ts`.
//!
//! Azure request/configuration policy is provider wire mechanism; Responses
//! message conversion and stream processing are shared with OpenAI Responses.

use std::collections::BTreeMap;

use pi_rs_ai_types::{
    AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Model, ModelThinkingLevel,
    ProviderResponse, StopReason, ThinkingLevel, Usage, clamp_thinking_level, now_ms,
};
use reqwest::Url;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::openai_prompt_cache::clamp_openai_prompt_cache_key;
use super::openai_responses::{
    ReasoningSummary, ResponsesFlavor, convert_responses_messages_for, convert_tools,
    mapped_effort, process_responses_stream,
};
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::{ProtocolError, merge_header_map};
use crate::transport::{
    AssistantMessageEventStream, RetryOptions, RetryPolicy, TransportError,
    create_assistant_message_event_stream, post_with_retry,
};
use crate::util::headers_to_record;

const DEFAULT_AZURE_API_VERSION: &str = "v1";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 600_000;
const AZURE_TOOL_CALL_PROVIDERS: &[&str] = &[
    "openai",
    "openai-codex",
    "opencode",
    "azure-openai-responses",
];

#[derive(Clone, Default)]
pub struct AzureOpenAIResponsesOptions {
    pub base: StreamOptions,
    pub reasoning_effort: Option<ThinkingLevel>,
    pub reasoning_summary: Option<ReasoningSummary>,
    pub azure_api_version: Option<String>,
    pub azure_resource_name: Option<String>,
    pub azure_base_url: Option<String>,
    pub azure_deployment_name: Option<String>,
}

fn parse_deployment_name_map(value: Option<&str>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(value) = value else { return map };
    for entry in value.split(',') {
        let mut parts = entry.trim().split('=');
        let (Some(model), Some(deployment)) = (parts.next(), parts.next()) else {
            continue;
        };
        let model = model.trim();
        let deployment = deployment.trim();
        if !model.is_empty() && !deployment.is_empty() {
            map.insert(model.to_string(), deployment.to_string());
        }
    }
    map
}

fn resolve_deployment_name(model: &Model, options: &AzureOpenAIResponsesOptions) -> String {
    options
        .azure_deployment_name
        .clone()
        .or_else(|| {
            parse_deployment_name_map(
                std::env::var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP")
                    .ok()
                    .as_deref(),
            )
            .get(&model.id)
            .cloned()
        })
        .unwrap_or_else(|| model.id.clone())
}

fn normalize_azure_base_url(base_url: &str) -> Result<String, ProtocolError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let mut url = Url::parse(trimmed)
        .map_err(|_| ProtocolError(format!("Invalid Azure OpenAI base URL: {base_url}")))?;
    let azure_host = url.host_str().is_some_and(|host| {
        host.ends_with(".openai.azure.com") || host.ends_with(".cognitiveservices.azure.com")
    });
    let path = url.path().trim_end_matches('/');
    if azure_host && matches!(path, "" | "/" | "/openai") {
        url.set_path("/openai/v1");
        url.set_query(None);
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn resolve_azure_config(
    model: &Model,
    options: &AzureOpenAIResponsesOptions,
) -> Result<(String, String), ProtocolError> {
    let api_version = options
        .azure_api_version
        .clone()
        .or_else(|| std::env::var("AZURE_OPENAI_API_VERSION").ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_AZURE_API_VERSION.to_string());
    let explicit_base = options
        .azure_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("AZURE_OPENAI_BASE_URL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    let resource = options
        .azure_resource_name
        .clone()
        .or_else(|| std::env::var("AZURE_OPENAI_RESOURCE_NAME").ok())
        .filter(|value| !value.is_empty());
    let base_url = explicit_base
        .or_else(|| {
            resource.map(|name| format!("https://{name}.openai.azure.com/openai/v1"))
        })
        .or_else(|| (!model.base_url.is_empty()).then(|| model.base_url.clone()))
        .ok_or_else(|| {
            ProtocolError("Azure OpenAI base URL is required. Set AZURE_OPENAI_BASE_URL or AZURE_OPENAI_RESOURCE_NAME, or pass azureBaseUrl, azureResourceName, or model.baseUrl.".to_string())
        })?;
    Ok((normalize_azure_base_url(&base_url)?, api_version))
}

fn request_url(base_url: &str, api_version: &str) -> Result<String, ProtocolError> {
    let mut url = Url::parse(&format!("{}/responses", base_url.trim_end_matches('/')))
        .map_err(|error| ProtocolError(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("api-version", api_version);
    Ok(url.to_string())
}

fn build_params(
    model: &Model,
    context: &Context,
    options: &AzureOpenAIResponsesOptions,
    deployment_name: &str,
) -> Value {
    let mut object = Map::new();
    object.insert("model".to_string(), json!(deployment_name));
    object.insert(
        "input".to_string(),
        json!(convert_responses_messages_for(
            model,
            context,
            true,
            AZURE_TOOL_CALL_PROVIDERS,
        )),
    );
    object.insert("stream".to_string(), json!(true));
    if let Some(key) = clamp_openai_prompt_cache_key(options.base.session_id.as_deref()) {
        object.insert("prompt_cache_key".to_string(), Value::String(key));
    }
    object.insert("store".to_string(), json!(false));
    if let Some(max_tokens) = options.base.max_tokens {
        object.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = options.base.temperature {
        object.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(tools) = &context.tools
        && !tools.is_empty()
    {
        object.insert("tools".to_string(), Value::Array(convert_tools(tools)));
    }
    if model.reasoning {
        if options.reasoning_effort.is_some() || options.reasoning_summary.is_some() {
            let effort = options.reasoning_effort.map_or_else(
                || "medium".to_string(),
                |effort| mapped_effort(model, effort),
            );
            let summary = options
                .reasoning_summary
                .unwrap_or(ReasoningSummary::Auto)
                .as_str();
            object.insert(
                "reasoning".to_string(),
                json!({ "effort": effort, "summary": summary }),
            );
            object.insert(
                "include".to_string(),
                json!(["reasoning.encrypted_content"]),
            );
        } else {
            let explicit_off = model
                .thinking_level_map
                .as_ref()
                .and_then(|map| map.get(&ModelThinkingLevel::Off));
            if !matches!(explicit_off, Some(None)) {
                let effort = explicit_off
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| "none".to_string());
                object.insert("reasoning".to_string(), json!({ "effort": effort }));
            }
        }
    }
    Value::Object(object)
}

fn headers(
    model: &Model,
    options: &AzureOpenAIResponsesOptions,
    api_key: &str,
) -> Result<HeaderMap, ProtocolError> {
    let mut values = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("accept".to_string(), "application/json".to_string()),
        ("api-key".to_string(), api_key.to_string()),
    ];
    merge_header_map(&mut values, model.headers.as_ref());
    merge_header_map(&mut values, options.base.headers.as_ref());
    let mut headers = HeaderMap::new();
    for (key, value) in values {
        headers.insert(
            HeaderName::from_bytes(key.as_bytes())
                .map_err(|error| ProtocolError(error.to_string()))?,
            HeaderValue::from_str(&value).map_err(|error| ProtocolError(error.to_string()))?,
        );
    }
    Ok(headers)
}

fn format_azure_error(error: &TransportError) -> String {
    match error {
        TransportError::Status { status, body, .. } => {
            let message = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.clone());
            format!("Azure OpenAI API error ({status}): {status} {message}")
        }
        _ => error.to_string(),
    }
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &AzureOpenAIResponsesOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let api_key = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let deployment = resolve_deployment_name(model, options);
    let (base_url, api_version) = resolve_azure_config(model, options)?;
    let mut params = build_params(model, context, options, &deployment);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model.clone()).await
    {
        params = next;
    }
    let response = post_with_retry(
        &reqwest::Client::new(),
        &request_url(&base_url, &api_version)?,
        &headers(model, options, api_key)?,
        &params.to_string(),
        &RetryOptions {
            max_retries: options.base.max_retries.unwrap_or(0),
            header_timeout_ms: options.base.timeout_ms.unwrap_or(DEFAULT_OPENAI_TIMEOUT_MS),
            policy: RetryPolicy::AnthropicSdk,
            ..Default::default()
        },
        options.base.signal.as_ref(),
    )
    .await
    .map_err(|error| ProtocolError(format_azure_error(&error)))?;
    if let Some(hook) = &options.base.on_response {
        hook(
            ProviderResponse {
                status: response.status().as_u16(),
                headers: headers_to_record(response.headers()),
            },
            model.clone(),
        )
        .await;
    }
    stream.push(AssistantMessageEvent::Start {
        partial: output.clone(),
    });
    process_responses_stream(
        response,
        options.base.signal.clone(),
        output,
        stream,
        model,
        None,
        ResponsesFlavor::OpenAi,
    )
    .await?;
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(crate::transport::AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request was aborted".to_string()));
    }
    if matches!(output.stop_reason, StopReason::Aborted | StopReason::Error) {
        return Err(ProtocolError("An unknown error occurred".to_string()));
    }
    Ok(())
}

pub fn stream_azure_openai_responses(
    model: &Model,
    context: &Context,
    options: Option<AzureOpenAIResponsesOptions>,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let task_stream = stream.clone();
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
        match drive(&model, &context, &options, &task_stream, &mut output).await {
            Ok(()) => task_stream.push(AssistantMessageEvent::Done {
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
                task_stream.push(AssistantMessageEvent::Error {
                    reason: output.stop_reason,
                    error: output,
                });
            }
        }
        task_stream.end();
    });
    stream
}

pub fn stream_simple_azure_openai_responses(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let api_key = options
        .as_ref()
        .and_then(|options| options.base.api_key.as_deref())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?
        .to_string();
    let base = build_base_options(model, options.as_ref(), Some(&api_key));
    let reasoning_effort = options
        .as_ref()
        .and_then(|options| options.reasoning)
        .and_then(
            |level| match clamp_thinking_level(model, ModelThinkingLevel::from(level)) {
                ModelThinkingLevel::Off => None,
                ModelThinkingLevel::Minimal => Some(ThinkingLevel::Minimal),
                ModelThinkingLevel::Low => Some(ThinkingLevel::Low),
                ModelThinkingLevel::Medium => Some(ThinkingLevel::Medium),
                ModelThinkingLevel::High => Some(ThinkingLevel::High),
                ModelThinkingLevel::XHigh => Some(ThinkingLevel::XHigh),
                ModelThinkingLevel::Max => Some(ThinkingLevel::Max),
            },
        );
    Ok(stream_azure_openai_responses(
        model,
        context,
        Some(AzureOpenAIResponsesOptions {
            base,
            reasoning_effort,
            ..Default::default()
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::normalize_azure_base_url;

    #[test]
    fn normalizes_only_bare_azure_hosts() {
        assert_eq!(
            normalize_azure_base_url("https://x.openai.azure.com").unwrap(),
            "https://x.openai.azure.com/openai/v1"
        );
        assert_eq!(
            normalize_azure_base_url("https://x.cognitiveservices.azure.com/openai/?old=1")
                .unwrap(),
            "https://x.cognitiveservices.azure.com/openai/v1"
        );
        assert_eq!(
            normalize_azure_base_url("https://example.test/custom/").unwrap(),
            "https://example.test/custom"
        );
    }
}
