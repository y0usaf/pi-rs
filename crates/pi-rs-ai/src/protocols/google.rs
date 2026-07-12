//! Port of `providers/google.ts` (`google-generative-ai`).

use std::sync::atomic::{AtomicU64, Ordering};

use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Model,
    ModelThinkingLevel, StopReason, TextContent, ThinkingContent, ThinkingLevel, ThinkingType,
    ToolCall, ToolCallType, Usage, calculate_cost, clamp_thinking_level, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::google_shared::{convert_messages, convert_tools, map_stop_reason};
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::{ProtocolError, merge_header_map};
use crate::transport::{
    AssistantMessageEventStream, TransportError, create_assistant_message_event_stream,
    response_sse_reader,
};
use crate::util::sanitize_surrogates;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_TIMEOUT_MS: u64 = 600_000;
static TOOL_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoogleThinkingLevel {
    Unspecified,
    Minimal,
    Low,
    Medium,
    High,
}
impl GoogleThinkingLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "THINKING_LEVEL_UNSPECIFIED",
            Self::Minimal => "MINIMAL",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GoogleThinking {
    pub enabled: bool,
    pub budget_tokens: Option<i64>,
    pub level: Option<GoogleThinkingLevel>,
}

#[derive(Clone, Copy, Debug)]
pub enum GoogleToolChoice {
    Auto,
    None,
    Any,
}
impl GoogleToolChoice {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::None => "NONE",
            Self::Any => "ANY",
        }
    }
}

#[derive(Clone, Default)]
pub struct GoogleOptions {
    pub base: StreamOptions,
    pub tool_choice: Option<GoogleToolChoice>,
    pub thinking: Option<GoogleThinking>,
}

fn is_gemini3_pro(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.starts_with("gemini-3-pro") || id.starts_with("gemini-3.") && id.contains("-pro")
}
fn is_gemini3_flash(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.starts_with("gemini-3-flash") || id.starts_with("gemini-3.") && id.contains("-flash")
}
fn is_gemma4(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("gemma-4") || id.contains("gemma4")
}

pub(crate) fn disabled_thinking(model: &Model) -> Value {
    if is_gemini3_pro(&model.id) {
        json!({"thinkingLevel":"LOW"})
    } else if is_gemini3_flash(&model.id) || is_gemma4(&model.id) {
        json!({"thinkingLevel":"MINIMAL"})
    } else {
        json!({"thinkingBudget":0})
    }
}

pub(crate) fn build_params(model: &Model, context: &Context, options: &GoogleOptions) -> Value {
    let mut root = Map::new();
    root.insert(
        "contents".into(),
        Value::Array(convert_messages(model, context)),
    );
    if let Some(prompt) = context.system_prompt.as_deref() {
        root.insert(
            "systemInstruction".into(),
            json!({"parts":[{"text":sanitize_surrogates(prompt)}],"role":"user"}),
        );
    }
    if let Some(tools) = context
        .tools
        .as_deref()
        .and_then(|tools| convert_tools(tools, false))
    {
        root.insert("tools".into(), tools);
    }
    if context
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
        && let Some(choice) = options.tool_choice
    {
        root.insert(
            "toolConfig".into(),
            json!({"functionCallingConfig":{"mode":choice.as_str()}}),
        );
    }
    let mut generation = Map::new();
    if let Some(temperature) = options.base.temperature {
        generation.insert("temperature".into(), json!(temperature));
    }
    if let Some(tokens) = options.base.max_tokens {
        generation.insert("maxOutputTokens".into(), json!(tokens));
    }
    if model.reasoning
        && let Some(thinking) = &options.thinking
    {
        if thinking.enabled {
            let mut config = Map::new();
            config.insert("includeThoughts".into(), json!(true));
            if let Some(level) = thinking.level {
                config.insert("thinkingLevel".into(), json!(level.as_str()));
            } else if let Some(budget) = thinking.budget_tokens {
                config.insert("thinkingBudget".into(), json!(budget));
            }
            generation.insert("thinkingConfig".into(), Value::Object(config));
        } else {
            generation.insert("thinkingConfig".into(), disabled_thinking(model));
        }
    }
    root.insert("generationConfig".into(), Value::Object(generation));
    Value::Object(root)
}

fn request_url(model: &Model) -> String {
    let configured = model.base_url.trim_end_matches('/');
    let base_url = if configured.is_empty() {
        DEFAULT_BASE_URL
    } else {
        configured
    };
    format!(
        "{base_url}/models/{}:streamGenerateContent?alt=sse",
        model.id
    )
}

fn headers(model: &Model, options: &GoogleOptions, key: &str) -> Result<HeaderMap, ProtocolError> {
    let mut values = vec![
        ("accept".to_string(), "*/*".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
        (
            "x-goog-api-client".to_string(),
            "google-genai-sdk/1.52.0 gl-node/v22.23.1".to_string(),
        ),
        ("x-goog-api-key".to_string(), key.to_string()),
    ];
    merge_header_map(&mut values, model.headers.as_ref());
    merge_header_map(&mut values, options.base.headers.as_ref());
    let mut result = HeaderMap::new();
    for (name, value) in values {
        result.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(|e| ProtocolError(e.to_string()))?,
            HeaderValue::from_str(&value).map_err(|e| ProtocolError(e.to_string()))?,
        );
    }
    Ok(result)
}

pub(crate) fn format_http_error(error: TransportError) -> String {
    match error {
        TransportError::Status {
            status,
            status_text,
            body,
        } => {
            if serde_json::from_str::<Value>(&body).is_ok() {
                body
            } else {
                json!({"error":{"message":body,"code":status,"status":status_text}}).to_string()
            }
        }
        other => other.to_string(),
    }
}

pub(crate) fn close_current(
    stream: &AssistantMessageEventStream,
    output: &AssistantMessage,
    index: usize,
) {
    match output.content.get(index) {
        Some(AssistantContent::Text(text)) => stream.push(AssistantMessageEvent::TextEnd {
            content_index: index,
            content: text.text.clone(),
            partial: output.clone(),
        }),
        Some(AssistantContent::Thinking(thinking)) => {
            stream.push(AssistantMessageEvent::ThinkingEnd {
                content_index: index,
                content: thinking.thinking.clone(),
                partial: output.clone(),
            })
        }
        _ => {}
    }
}

fn retain_signature(current: &mut Option<String>, incoming: Option<&str>) {
    if let Some(value) = incoming.filter(|value| !value.is_empty()) {
        *current = Some(value.to_string());
    }
}

pub(crate) fn process_chunk(
    model: &Model,
    chunk: &Value,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
    current: &mut Option<usize>,
) -> Result<(), ProtocolError> {
    if output.response_id.is_none() {
        output.response_id = chunk
            .get("responseId")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
    }
    let candidate = chunk.pointer("/candidates/0");
    if let Some(parts) = candidate
        .and_then(|v| v.pointer("/content/parts"))
        .and_then(Value::as_array)
    {
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                let thinking = part.get("thought").and_then(Value::as_bool) == Some(true);
                let same = current
                    .and_then(|index| output.content.get(index))
                    .is_some_and(|block| {
                        matches!(
                            (thinking, block),
                            (true, AssistantContent::Thinking(_))
                                | (false, AssistantContent::Text(_))
                        )
                    });
                if !same {
                    if let Some(index) = current.take() {
                        close_current(stream, output, index);
                    }
                    let index = output.content.len();
                    if thinking {
                        output
                            .content
                            .push(AssistantContent::Thinking(ThinkingContent {
                                r#type: ThinkingType::Thinking,
                                thinking: String::new(),
                                thinking_signature: None,
                                redacted: None,
                            }));
                        stream.push(AssistantMessageEvent::ThinkingStart {
                            content_index: index,
                            partial: output.clone(),
                        });
                    } else {
                        output
                            .content
                            .push(AssistantContent::Text(TextContent::new("")));
                        stream.push(AssistantMessageEvent::TextStart {
                            content_index: index,
                            partial: output.clone(),
                        });
                    }
                    *current = Some(index);
                }
                let index = current.unwrap_or(0);
                match output.content.get_mut(index) {
                    Some(AssistantContent::Thinking(block)) => {
                        block.thinking.push_str(text);
                        retain_signature(
                            &mut block.thinking_signature,
                            part.get("thoughtSignature").and_then(Value::as_str),
                        );
                        stream.push(AssistantMessageEvent::ThinkingDelta {
                            content_index: index,
                            delta: text.to_string(),
                            partial: output.clone(),
                        });
                    }
                    Some(AssistantContent::Text(block)) => {
                        block.text.push_str(text);
                        retain_signature(
                            &mut block.text_signature,
                            part.get("thoughtSignature").and_then(Value::as_str),
                        );
                        stream.push(AssistantMessageEvent::TextDelta {
                            content_index: index,
                            delta: text.to_string(),
                            partial: output.clone(),
                        });
                    }
                    _ => {}
                }
            }
            if let Some(function) = part.get("functionCall") {
                if let Some(index) = current.take() {
                    close_current(stream, output, index);
                }
                let provided = function
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty());
                let duplicate = provided.is_some_and(|id| {
                    output.content.iter().any(
                        |block| matches!(block, AssistantContent::ToolCall(call) if call.id == id),
                    )
                });
                let name = function.get("name").and_then(Value::as_str).unwrap_or("");
                let id = if provided.is_none() || duplicate {
                    format!(
                        "{name}_{}_{}",
                        now_ms(),
                        TOOL_CALL_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
                    )
                } else {
                    provided.unwrap_or_default().to_string()
                };
                let arguments = function
                    .get("args")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let call = ToolCall {
                    r#type: ToolCallType::ToolCall,
                    id,
                    name: name.to_string(),
                    arguments,
                    thought_signature: part
                        .get("thoughtSignature")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                };
                let index = output.content.len();
                output
                    .content
                    .push(AssistantContent::ToolCall(call.clone()));
                stream.push(AssistantMessageEvent::ToolCallStart {
                    content_index: index,
                    partial: output.clone(),
                });
                stream.push(AssistantMessageEvent::ToolCallDelta {
                    content_index: index,
                    delta: serde_json::to_string(&call.arguments)
                        .map_err(|e| ProtocolError(e.to_string()))?,
                    partial: output.clone(),
                });
                stream.push(AssistantMessageEvent::ToolCallEnd {
                    content_index: index,
                    tool_call: call,
                    partial: output.clone(),
                });
            }
        }
    }
    if let Some(reason) = candidate
        .and_then(|v| v.get("finishReason"))
        .and_then(Value::as_str)
    {
        output.stop_reason = map_stop_reason(reason);
        if output
            .content
            .iter()
            .any(|block| matches!(block, AssistantContent::ToolCall(_)))
        {
            output.stop_reason = StopReason::ToolUse;
        }
    }
    if let Some(usage) = chunk.get("usageMetadata") {
        let prompt = usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cached = usage
            .get("cachedContentTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let candidates = usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let thoughts = usage
            .get("thoughtsTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        output.usage = Usage {
            input: prompt.saturating_sub(cached),
            output: candidates + thoughts,
            cache_read: cached,
            cache_write: 0,
            total_tokens: usage
                .get("totalTokenCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            ..Usage::default()
        };
        calculate_cost(model, &mut output.usage);
    }
    Ok(())
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &GoogleOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let key = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(crate::transport::AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request aborted".into()));
    }
    let mut params = build_params(model, context, options);
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
        .map_err(|e| ProtocolError(e.to_string()))?;
    let response = client
        .post(request_url(model))
        .headers(headers(model, options, key)?)
        .body(params.to_string())
        .send()
        .await
        .map_err(|e| ProtocolError(e.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let error = TransportError::Status {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or_default().to_string(),
            body: response.text().await.unwrap_or_default(),
        };
        return Err(ProtocolError(format_http_error(error)));
    }
    stream.push(AssistantMessageEvent::Start {
        partial: output.clone(),
    });
    let mut reader = response_sse_reader(response, options.base.signal.clone());
    let mut current = None;
    while let Some(event) = reader
        .next()
        .await
        .map_err(|e| ProtocolError(e.to_string()))?
    {
        let chunk: Value =
            serde_json::from_str(&event.data).map_err(|e| ProtocolError(e.to_string()))?;
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

pub fn stream_google(
    model: &Model,
    context: &Context,
    options: Option<GoogleOptions>,
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

pub(crate) fn effort_level(model: &Model, effort: ThinkingLevel) -> GoogleThinkingLevel {
    if is_gemini3_pro(&model.id) {
        return if matches!(effort, ThinkingLevel::Minimal | ThinkingLevel::Low) {
            GoogleThinkingLevel::Low
        } else {
            GoogleThinkingLevel::High
        };
    }
    if is_gemma4(&model.id) {
        return if matches!(effort, ThinkingLevel::Minimal | ThinkingLevel::Low) {
            GoogleThinkingLevel::Minimal
        } else {
            GoogleThinkingLevel::High
        };
    }
    match effort {
        ThinkingLevel::Minimal => GoogleThinkingLevel::Minimal,
        ThinkingLevel::Low => GoogleThinkingLevel::Low,
        ThinkingLevel::Medium => GoogleThinkingLevel::Medium,
        _ => GoogleThinkingLevel::High,
    }
}
pub(crate) fn budget(
    model: &Model,
    effort: ThinkingLevel,
    custom: Option<&pi_rs_ai_types::ThinkingBudgets>,
) -> i64 {
    let custom_value = match effort {
        ThinkingLevel::Minimal => custom.and_then(|v| v.minimal),
        ThinkingLevel::Low => custom.and_then(|v| v.low),
        ThinkingLevel::Medium => custom.and_then(|v| v.medium),
        _ => custom.and_then(|v| v.high),
    };
    if let Some(value) = custom_value {
        return value as i64;
    }
    let values = if model.id.contains("2.5-pro") {
        [128, 2048, 8192, 32768]
    } else if model.id.contains("2.5-flash-lite") {
        [512, 2048, 8192, 24576]
    } else if model.id.contains("2.5-flash") {
        [128, 2048, 8192, 24576]
    } else {
        return -1;
    };
    values[match effort {
        ThinkingLevel::Minimal => 0,
        ThinkingLevel::Low => 1,
        ThinkingLevel::Medium => 2,
        _ => 3,
    }]
}

pub fn stream_simple_google(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let key = options
        .as_ref()
        .and_then(|o| o.base.api_key.as_deref())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?
        .to_string();
    let base = build_base_options(model, options.as_ref(), Some(&key));
    let thinking = if let Some(reasoning) = options.as_ref().and_then(|o| o.reasoning) {
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
        if is_gemini3_pro(&model.id) || is_gemini3_flash(&model.id) || is_gemma4(&model.id) {
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
                    options.as_ref().and_then(|o| o.thinking_budgets.as_ref()),
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
    Ok(stream_google(
        model,
        context,
        Some(GoogleOptions {
            base,
            thinking: Some(thinking),
            tool_choice: None,
        }),
    ))
}
