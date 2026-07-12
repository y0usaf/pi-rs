//! Port of the SSE path in `providers/openai-codex-responses.ts`.
//! Responses message/event conversion is shared with `openai_responses`;
//! WebSocket transport and session continuation remain PLAN item 8 work.

use base64::Engine;
use pi_rs_ai_types::{
    AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Model, ModelThinkingLevel,
    ProviderResponse, StopReason, Tool, Transport, Usage, clamp_thinking_level, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::openai_prompt_cache::clamp_openai_prompt_cache_key;
use super::openai_responses::{
    ResponsesFlavor, convert_responses_messages, process_responses_stream,
};
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::{ProtocolError, merge_header, merge_header_map};
use crate::transport::{
    AssistantMessageEventStream, RetryOptions, RetryPolicy, TransportError,
    create_assistant_message_event_stream, post_with_retry,
};
use crate::util::headers_to_record;

const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_SSE_HEADER_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexReasoningSummary {
    Auto,
    Concise,
    Detailed,
    Off,
    On,
}

impl CodexReasoningSummary {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Concise => "concise",
            Self::Detailed => "detailed",
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

#[derive(Clone, Default)]
pub struct OpenAICodexResponsesOptions {
    pub base: StreamOptions,
    pub reasoning_effort: Option<ModelThinkingLevel>,
    pub reasoning_summary: Option<CodexReasoningSummary>,
    pub service_tier: Option<String>,
    pub text_verbosity: Option<String>,
}

fn thinking_level_name(level: ModelThinkingLevel) -> &'static str {
    match level {
        ModelThinkingLevel::Off => "none",
        ModelThinkingLevel::Minimal => "minimal",
        ModelThinkingLevel::Low => "low",
        ModelThinkingLevel::Medium => "medium",
        ModelThinkingLevel::High => "high",
        ModelThinkingLevel::XHigh => "xhigh",
        ModelThinkingLevel::Max => "max",
    }
}

fn convert_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": null
            })
        })
        .collect()
}

fn mapped_effort(model: &Model, effort: ModelThinkingLevel) -> Option<String> {
    if let Some(map) = &model.thinking_level_map
        && let Some(mapped) = map.get(&effort)
    {
        return mapped.clone();
    }
    Some(thinking_level_name(effort).to_string())
}

fn build_request_body(
    model: &Model,
    context: &Context,
    options: &OpenAICodexResponsesOptions,
) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(model.id));
    body.insert("store".to_string(), json!(false));
    body.insert("stream".to_string(), json!(true));
    body.insert(
        "instructions".to_string(),
        json!(
            context
                .system_prompt
                .as_deref()
                .filter(|prompt| !prompt.is_empty())
                .unwrap_or("You are a helpful assistant.")
        ),
    );
    body.insert(
        "input".to_string(),
        json!(convert_responses_messages(model, context, false)),
    );
    body.insert(
        "text".to_string(),
        json!({ "verbosity": options.text_verbosity.as_deref().unwrap_or("low") }),
    );
    body.insert(
        "include".to_string(),
        json!(["reasoning.encrypted_content"]),
    );
    if let Some(key) = clamp_openai_prompt_cache_key(options.base.session_id.as_deref()) {
        body.insert("prompt_cache_key".to_string(), json!(key));
    }
    body.insert("tool_choice".to_string(), json!("auto"));
    body.insert("parallel_tool_calls".to_string(), json!(true));
    if let Some(temperature) = options.base.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(service_tier) = &options.service_tier {
        body.insert("service_tier".to_string(), json!(service_tier));
    }
    if let Some(tools) = &context.tools
        && !tools.is_empty()
    {
        body.insert("tools".to_string(), Value::Array(convert_tools(tools)));
    }
    if let Some(effort) = options.reasoning_effort
        && let Some(effort) = mapped_effort(model, effort)
    {
        body.insert(
            "reasoning".to_string(),
            json!({
                "effort": effort,
                "summary": options.reasoning_summary.unwrap_or(CodexReasoningSummary::Auto).as_str()
            }),
        );
    }
    Value::Object(body)
}

fn extract_account_id(token: &str) -> Result<String, ProtocolError> {
    let mut parts = token.split('.');
    let (_header, encoded, signature) = (parts.next(), parts.next(), parts.next());
    let encoded = match (encoded, signature, parts.next()) {
        (Some(encoded), Some(_), None) => encoded,
        _ => {
            return Err(ProtocolError(
                "Failed to extract accountId from token".to_string(),
            ));
        }
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(encoded))
        .map_err(|_| ProtocolError("Failed to extract accountId from token".to_string()))?;
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ProtocolError("Failed to extract accountId from token".to_string()))?;
    payload
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProtocolError("Failed to extract accountId from token".to_string()))
}

fn resolve_codex_url(base_url: &str) -> String {
    let raw = if base_url.trim().is_empty() {
        DEFAULT_CODEX_BASE_URL
    } else {
        base_url.trim()
    };
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

fn insert_headers(entries: Vec<(String, String)>) -> Result<HeaderMap, ProtocolError> {
    let mut headers = HeaderMap::new();
    for (key, value) in entries {
        headers.insert(
            HeaderName::from_bytes(key.as_bytes())
                .map_err(|error| ProtocolError(error.to_string()))?,
            HeaderValue::from_str(&value).map_err(|error| ProtocolError(error.to_string()))?,
        );
    }
    Ok(headers)
}

fn build_sse_headers(
    model: &Model,
    options: &OpenAICodexResponsesOptions,
    account_id: &str,
    token: &str,
) -> Result<HeaderMap, ProtocolError> {
    let mut headers = Vec::new();
    merge_header_map(&mut headers, model.headers.as_ref());
    merge_header_map(&mut headers, options.base.headers.as_ref());
    merge_header(&mut headers, "authorization", &format!("Bearer {token}"));
    merge_header(&mut headers, "chatgpt-account-id", account_id);
    merge_header(&mut headers, "originator", "pi");
    merge_header(&mut headers, "user-agent", "pi (browser)");
    merge_header(&mut headers, "openai-beta", "responses=experimental");
    merge_header(&mut headers, "accept", "text/event-stream");
    merge_header(&mut headers, "content-type", "application/json");
    if let Some(session_id) = &options.base.session_id {
        merge_header(&mut headers, "session-id", session_id);
        merge_header(&mut headers, "x-client-request-id", session_id);
    }
    insert_headers(headers)
}

fn format_http_error(error: &TransportError) -> String {
    let TransportError::Status {
        status,
        status_text,
        body,
    } = error
    else {
        return error.to_string();
    };
    let fallback = if body.is_empty() {
        if status_text.is_empty() {
            "Request failed"
        } else {
            status_text
        }
    } else {
        body
    };
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return fallback.to_string();
    };
    let Some(provider_error) = parsed.get("error") else {
        return fallback.to_string();
    };
    let code = provider_error
        .get("code")
        .or_else(|| provider_error.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if code.to_ascii_lowercase().contains("usage_limit_reached")
        || code.to_ascii_lowercase().contains("usage_not_included")
        || code.to_ascii_lowercase().contains("rate_limit_exceeded")
        || *status == 429
    {
        let plan = provider_error
            .get("plan_type")
            .and_then(Value::as_str)
            .map(|plan| format!(" ({} plan)", plan.to_ascii_lowercase()))
            .unwrap_or_default();
        return format!("You have hit your ChatGPT usage limit{plan}.");
    }
    provider_error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &OpenAICodexResponsesOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let token = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let account_id = extract_account_id(token)?;
    if !matches!(options.base.transport, Some(Transport::Sse)) {
        return Err(ProtocolError(
            "OpenAI Codex WebSocket transport is not implemented".to_string(),
        ));
    }
    let headers = build_sse_headers(model, options, &account_id, token)?;
    let mut body = build_request_body(model, context, options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(body.clone(), model)
    {
        body = next;
    }
    let response = post_with_retry(
        &reqwest::Client::new(),
        &resolve_codex_url(&model.base_url),
        &headers,
        &body.to_string(),
        &RetryOptions {
            max_retries: options.base.max_retries.unwrap_or(0),
            max_retry_delay_ms: options.base.max_retry_delay_ms,
            header_timeout_ms: DEFAULT_SSE_HEADER_TIMEOUT_MS,
            policy: RetryPolicy::Codex,
        },
        options.base.signal.as_ref(),
    )
    .await
    .map_err(|error| ProtocolError(format_http_error(&error)))?;
    if let Some(hook) = &options.base.on_response {
        hook(
            &ProviderResponse {
                status: response.status().as_u16(),
                headers: headers_to_record(response.headers()),
            },
            model,
        );
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
        options.service_tier.as_deref(),
        ResponsesFlavor::Codex,
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
    Ok(())
}

pub fn stream_openai_codex_responses(
    model: &Model,
    context: &Context,
    options: Option<OpenAICodexResponsesOptions>,
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

pub fn stream_simple_openai_codex_responses(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let api_key = options
        .as_ref()
        .and_then(|options| options.base.api_key.as_deref())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let reasoning_effort = options
        .as_ref()
        .and_then(|options| options.reasoning)
        .map(|level| clamp_thinking_level(model, ModelThinkingLevel::from(level)));
    Ok(stream_openai_codex_responses(
        model,
        context,
        Some(OpenAICodexResponsesOptions {
            base: build_base_options(model, options.as_ref(), Some(api_key)),
            reasoning_effort,
            ..Default::default()
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_url_shapes_match_spec() {
        assert_eq!(resolve_codex_url("https://x"), "https://x/codex/responses");
        assert_eq!(
            resolve_codex_url("https://x/codex"),
            "https://x/codex/responses"
        );
        assert_eq!(
            resolve_codex_url("https://x/codex/responses/"),
            "https://x/codex/responses"
        );
    }
}
