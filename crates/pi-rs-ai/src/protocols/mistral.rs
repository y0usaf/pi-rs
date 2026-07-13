//! Port of `providers/mistral.ts` (`mistral-conversations`).
//! Request/stream mapping is implemented over the shared HTTP/SSE transport.

use std::collections::HashMap;

use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, Context, Message,
    Model, ModelThinkingLevel, ProviderResponse, StopReason, TextContent, ThinkingContent,
    ThinkingLevel, ThinkingType, ToolCall, ToolCallType, Usage, UserContent, calculate_cost,
    clamp_thinking_level, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::transform_messages::transform_messages;
use super::{ProtocolError, merge_header_map};
use crate::transport::{
    AbortSignal, AssistantMessageEventStream, create_assistant_message_event_stream,
    response_sse_reader,
};
use crate::util::{headers_to_record, parse_streaming_json, sanitize_surrogates};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const TOOL_CALL_ID_LENGTH: usize = 9;
const MAX_ERROR_BODY_CHARS: usize = 4_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MistralToolChoice {
    Auto,
    None,
    Any,
    Required,
    Function { name: String },
}

impl MistralToolChoice {
    fn value(&self) -> Value {
        match self {
            Self::Auto => json!("auto"),
            Self::None => json!("none"),
            Self::Any => json!("any"),
            Self::Required => json!("required"),
            Self::Function { name } => json!({"type":"function","function":{"name":name}}),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MistralReasoningEffort {
    None,
    High,
}

impl MistralReasoningEffort {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Default)]
pub struct MistralOptions {
    pub base: StreamOptions,
    pub tool_choice: Option<MistralToolChoice>,
    pub prompt_mode_reasoning: bool,
    pub reasoning_effort: Option<MistralReasoningEffort>,
}

fn short_hash(value: &str) -> String {
    let mut h1 = 0xdead_beefu32;
    let mut h2 = 0x41c6_ce57u32;
    for ch in value.encode_utf16() {
        h1 = (h1 ^ u32::from(ch)).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ u32::from(ch)).wrapping_mul(1_597_334_677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(3_266_489_909);
    format!("{}{}", radix36(h2), radix36(h1))
}

fn radix36(mut value: u32) -> String {
    if value == 0 {
        return "0".into();
    }
    let mut chars = Vec::new();
    while value > 0 {
        let digit = (value % 36) as u8;
        chars.push(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        } as char);
        value /= 36;
    }
    chars.iter().rev().collect()
}

fn derive_tool_call_id(id: &str, attempt: usize) -> String {
    let normalized: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    if attempt == 0 && normalized.len() == TOOL_CALL_ID_LENGTH {
        return normalized;
    }
    let base = if normalized.is_empty() {
        id
    } else {
        &normalized
    };
    let seed = if attempt == 0 {
        base.to_string()
    } else {
        format!("{base}:{attempt}")
    };
    short_hash(&seed)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(TOOL_CALL_ID_LENGTH)
        .collect()
}

fn normalize_messages(model: &Model, messages: &[Message]) -> Vec<Message> {
    // Preserve the spec's per-request collision map through the `Fn` normalizer.
    let ids = std::cell::RefCell::new(HashMap::<String, String>::new());
    let reverse = std::cell::RefCell::new(HashMap::<String, String>::new());
    let normalize = |id: &str, _model: &Model, _source: &AssistantMessage| {
        if let Some(value) = ids.borrow().get(id) {
            return value.clone();
        }
        let mut attempt = 0;
        loop {
            let candidate = derive_tool_call_id(id, attempt);
            if reverse
                .borrow()
                .get(&candidate)
                .is_none_or(|owner| owner == id)
            {
                ids.borrow_mut().insert(id.to_string(), candidate.clone());
                reverse
                    .borrow_mut()
                    .insert(candidate.clone(), id.to_string());
                return candidate;
            }
            attempt += 1;
        }
    };
    transform_messages(messages, model, Some(&normalize))
}

fn tool_result_text(message: &pi_rs_ai_types::ToolResultMessage, supports_images: bool) -> String {
    let text = message
        .content
        .iter()
        .filter_map(|part| match part {
            pi_rs_ai_types::TextOrImageContent::Text(text) => Some(sanitize_surrogates(&text.text)),
            pi_rs_ai_types::TextOrImageContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let has_images = message
        .content
        .iter()
        .any(|part| matches!(part, pi_rs_ai_types::TextOrImageContent::Image(_)));
    let prefix = if message.is_error {
        "[tool error] "
    } else {
        ""
    };
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        let suffix = if has_images && !supports_images {
            "\n[tool image omitted: model does not support images]"
        } else {
            ""
        };
        return format!("{prefix}{trimmed}{suffix}");
    }
    if has_images {
        if supports_images {
            return format!("{prefix}(see attached image)");
        }
        return format!("{prefix}(image omitted: model does not support images)");
    }
    format!("{prefix}(no tool output)")
}

fn convert_messages(model: &Model, context: &Context) -> Vec<Value> {
    let supports_images = model.input.contains(&pi_rs_ai_types::Modality::Image);
    let mut result = Vec::new();
    for message in normalize_messages(model, &context.messages) {
        match message {
            Message::User(user) => match user.content {
                UserContent::Text(text) => {
                    result.push(json!({"role":"user","content":sanitize_surrogates(&text)}))
                }
                UserContent::Blocks(parts) => {
                    let had_images = parts
                        .iter()
                        .any(|part| matches!(part, pi_rs_ai_types::TextOrImageContent::Image(_)));
                    let content = parts.into_iter().filter_map(|part| match part {
                        pi_rs_ai_types::TextOrImageContent::Text(text) => Some(json!({"type":"text","text":sanitize_surrogates(&text.text)})),
                        pi_rs_ai_types::TextOrImageContent::Image(image) if supports_images => Some(json!({"type":"image_url","image_url":format!("data:{};base64,{}", image.mime_type, image.data)})),
                        pi_rs_ai_types::TextOrImageContent::Image(_) => None,
                    }).collect::<Vec<_>>();
                    if !content.is_empty() {
                        result.push(json!({"role":"user","content":content}));
                    } else if had_images && !supports_images {
                        result.push(json!({"role":"user","content":"(image omitted: model does not support images)"}));
                    }
                }
            },
            Message::Assistant(assistant) => {
                let mut content = Vec::new();
                let mut calls = Vec::new();
                for block in assistant.content {
                    match block {
                        AssistantContent::Text(text) if !text.text.trim().is_empty() => content.push(json!({"type":"text","text":sanitize_surrogates(&text.text)})),
                        AssistantContent::Thinking(thinking) if !thinking.thinking.trim().is_empty() => content.push(json!({"type":"thinking","thinking":[{"type":"text","text":sanitize_surrogates(&thinking.thinking)}]})),
                        AssistantContent::ToolCall(call) => calls.push(json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":Value::Object(call.arguments).to_string()},"index":calls.len()})),
                        _ => {}
                    }
                }
                if !content.is_empty() || !calls.is_empty() {
                    let mut value = Map::new();
                    value.insert("role".into(), json!("assistant"));
                    if !content.is_empty() {
                        value.insert("content".into(), Value::Array(content));
                    }
                    if !calls.is_empty() {
                        value.insert("tool_calls".into(), Value::Array(calls));
                    }
                    value.insert("prefix".into(), json!(false));
                    result.push(Value::Object(value));
                }
            }
            Message::ToolResult(tool) => {
                let mut content =
                    vec![json!({"type":"text","text":tool_result_text(&tool, supports_images)})];
                if supports_images {
                    for part in &tool.content {
                        if let pi_rs_ai_types::TextOrImageContent::Image(image) = part {
                            content.push(json!({"type":"image_url","image_url":format!("data:{};base64,{}", image.mime_type, image.data)}));
                        }
                    }
                }
                result.push(json!({"role":"tool","content":content,"tool_call_id":tool.tool_call_id,"name":tool.tool_name}));
            }
        }
    }
    result
}

fn build_params(model: &Model, context: &Context, options: &MistralOptions) -> Value {
    let mut messages = convert_messages(model, context);
    if let Some(prompt) = &context.system_prompt {
        messages.insert(
            0,
            json!({"role":"system","content":sanitize_surrogates(prompt)}),
        );
    }
    let mut value = Map::new();
    value.insert("model".into(), json!(model.id));
    if let Some(temperature) = options.base.temperature {
        value.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_tokens) = options.base.max_tokens {
        value.insert("max_tokens".into(), json!(max_tokens));
    }
    value.insert("stream".into(), json!(true));
    value.insert("messages".into(), Value::Array(messages));
    if let Some(tools) = &context.tools
        && !tools.is_empty()
    {
        value.insert("tools".into(), Value::Array(tools.iter().map(|tool| json!({"type":"function","function":{"name":tool.name,"description":tool.description,"strict":false,"parameters":tool.parameters}})).collect()));
    }
    if let Some(choice) = &options.tool_choice {
        value.insert("tool_choice".into(), choice.value());
    }
    if options.prompt_mode_reasoning {
        value.insert("prompt_mode".into(), json!("reasoning"));
    }
    if let Some(effort) = options.reasoning_effort {
        value.insert("reasoning_effort".into(), json!(effort.as_str()));
    }
    Value::Object(value)
}

fn request_headers(
    model: &Model,
    options: &MistralOptions,
    api_key: &str,
) -> Result<HeaderMap, ProtocolError> {
    let mut values = vec![
        ("content-type".into(), "application/json".into()),
        ("accept".into(), "text/event-stream".into()),
        ("authorization".into(), format!("Bearer {api_key}")),
        ("cookie".into(), String::new()),
    ];
    merge_header_map(&mut values, model.headers.as_ref());
    merge_header_map(&mut values, options.base.headers.as_ref());
    if let Some(session) = options.base.session_id.as_deref()
        && !values
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("x-affinity"))
    {
        values.push(("x-affinity".into(), session.into()));
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

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "length" | "model_length" => StopReason::Length,
        "tool_calls" => StopReason::ToolUse,
        "error" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

struct ToolMeta {
    index: usize,
    partial: String,
}

fn close_current(
    stream: &AssistantMessageEventStream,
    output: &AssistantMessage,
    current: Option<usize>,
) {
    let Some(index) = current else {
        return;
    };
    match &output.content[index] {
        AssistantContent::Text(text) => stream.push(AssistantMessageEvent::TextEnd {
            content_index: index,
            content: text.text.clone(),
            partial: output.clone(),
        }),
        AssistantContent::Thinking(thinking) => stream.push(AssistantMessageEvent::ThinkingEnd {
            content_index: index,
            content: thinking.thinking.clone(),
            partial: output.clone(),
        }),
        AssistantContent::ToolCall(_) => {}
    }
}

fn process_chunk(
    model: &Model,
    chunk: &Value,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
    current: &mut Option<usize>,
    tools: &mut HashMap<String, ToolMeta>,
) -> Result<(), ProtocolError> {
    if output.response_id.is_none() {
        output.response_id = chunk.get("id").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(usage) = chunk.get("usage") {
        output.usage.input = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        output.usage.output = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        output.usage.total_tokens = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(output.usage.input + output.usage.output);
        calculate_cost(model, &mut output.usage);
    }
    let Some(choice) = chunk
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    else {
        return Ok(());
    };
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        output.stop_reason = map_stop_reason(reason);
    }
    let Some(delta) = choice.get("delta") else {
        return Ok(());
    };
    if let Some(content) = delta.get("content").filter(|value| !value.is_null()) {
        let items = content
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![content.clone()]);
        for item in items {
            let (kind, text) = if let Some(text) = item.as_str() {
                ("text", text.to_string())
            } else if item.get("type").and_then(Value::as_str) == Some("thinking") {
                (
                    "thinking",
                    item.get("thinking")
                        .and_then(Value::as_array)
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|part| part.get("text").and_then(Value::as_str))
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            } else if item.get("type").and_then(Value::as_str) == Some("text") {
                (
                    "text",
                    item.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let same = current.is_some_and(|index| {
                matches!(
                    (&output.content[index], kind),
                    (AssistantContent::Text(_), "text")
                        | (AssistantContent::Thinking(_), "thinking")
                )
            });
            if !same {
                close_current(stream, output, *current);
                if kind == "text" {
                    output
                        .content
                        .push(AssistantContent::Text(TextContent::new("")));
                    *current = Some(output.content.len() - 1);
                    stream.push(AssistantMessageEvent::TextStart {
                        content_index: current.unwrap_or(0),
                        partial: output.clone(),
                    });
                } else {
                    output
                        .content
                        .push(AssistantContent::Thinking(ThinkingContent {
                            r#type: ThinkingType::Thinking,
                            thinking: String::new(),
                            thinking_signature: None,
                            redacted: None,
                        }));
                    *current = Some(output.content.len() - 1);
                    stream.push(AssistantMessageEvent::ThinkingStart {
                        content_index: current.unwrap_or(0),
                        partial: output.clone(),
                    });
                }
            }
            let index = current.unwrap_or(0);
            if let Some(AssistantContent::Text(block)) = output.content.get_mut(index) {
                block.text.push_str(&text);
                stream.push(AssistantMessageEvent::TextDelta {
                    content_index: index,
                    delta: text,
                    partial: output.clone(),
                });
            } else if let Some(AssistantContent::Thinking(block)) = output.content.get_mut(index) {
                block.thinking.push_str(&text);
                stream.push(AssistantMessageEvent::ThinkingDelta {
                    content_index: index,
                    delta: text,
                    partial: output.clone(),
                });
            }
        }
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            close_current(stream, output, current.take());
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| *id != "null")
                .map(str::to_string)
                .unwrap_or_else(|| {
                    derive_tool_call_id(
                        &format!(
                            "toolcall:{}",
                            call.get("index").and_then(Value::as_u64).unwrap_or(0)
                        ),
                        0,
                    )
                });
            let stream_index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
            let key = format!("{call_id}:{stream_index}");
            if !tools.contains_key(&key) {
                output.content.push(AssistantContent::ToolCall(ToolCall {
                    r#type: ToolCallType::ToolCall,
                    id: call_id,
                    name: call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments: Map::new(),
                    thought_signature: None,
                }));
                let index = output.content.len() - 1;
                tools.insert(
                    key.clone(),
                    ToolMeta {
                        index,
                        partial: String::new(),
                    },
                );
                stream.push(AssistantMessageEvent::ToolCallStart {
                    content_index: index,
                    partial: output.clone(),
                });
            }
            let meta = tools
                .get_mut(&key)
                .ok_or_else(|| ProtocolError("missing tool stream state".into()))?;
            let argument = call
                .pointer("/function/arguments")
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string())
                })
                .unwrap_or_default();
            meta.partial.push_str(&argument);
            if let Some(AssistantContent::ToolCall(block)) = output.content.get_mut(meta.index) {
                block.arguments = match parse_streaming_json(&meta.partial) {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
            }
            stream.push(AssistantMessageEvent::ToolCallDelta {
                content_index: meta.index,
                delta: argument,
                partial: output.clone(),
            });
        }
    }
    Ok(())
}

fn format_http_error(status: u16, body: &str) -> String {
    let trimmed = body.trim();
    let body = if trimmed.chars().count() > MAX_ERROR_BODY_CHARS {
        let prefix: String = trimmed.chars().take(MAX_ERROR_BODY_CHARS).collect();
        format!(
            "{prefix}... [truncated {} chars]",
            trimmed.chars().count() - MAX_ERROR_BODY_CHARS
        )
    } else {
        trimmed.to_string()
    };
    if body.is_empty() {
        format!("Mistral API error ({status}): HTTP status {status}")
    } else {
        format!("Mistral API error ({status}): {body}")
    }
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &MistralOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let api_key = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let mut payload = build_params(model, context, options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(payload.clone(), model.clone()).await
    {
        payload = next;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            options.base.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
        ))
        .build()
        .map_err(|error| ProtocolError(error.to_string()))?;
    let response = client
        .post(format!(
            "{}/v1/chat/completions",
            model.base_url.trim_end_matches('/')
        ))
        .headers(request_headers(model, options, api_key)?)
        .body(payload.to_string())
        .send()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(ProtocolError(format_http_error(status, &body)));
    }
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
    let mut reader = response_sse_reader(response, options.base.signal.clone());
    let mut current = None;
    let mut tools = HashMap::new();
    while let Some(event) = reader
        .next()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?
    {
        if event.data == "[DONE]" {
            break;
        }
        let chunk: Value =
            serde_json::from_str(&event.data).map_err(|error| ProtocolError(error.to_string()))?;
        process_chunk(model, &chunk, stream, output, &mut current, &mut tools)?;
    }
    close_current(stream, output, current);
    let mut ordered = tools.into_values().collect::<Vec<_>>();
    ordered.sort_by_key(|meta| meta.index);
    for meta in ordered {
        if let Some(AssistantContent::ToolCall(block)) = output.content.get_mut(meta.index) {
            block.arguments = match parse_streaming_json(&meta.partial) {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            stream.push(AssistantMessageEvent::ToolCallEnd {
                content_index: meta.index,
                tool_call: block.clone(),
                partial: output.clone(),
            });
        }
    }
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request was aborted".into()));
    }
    if matches!(output.stop_reason, StopReason::Aborted | StopReason::Error) {
        return Err(ProtocolError("An unknown error occurred".into()));
    }
    Ok(())
}

pub fn stream_mistral(
    model: &Model,
    context: &Context,
    options: Option<MistralOptions>,
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
                    .is_some_and(AbortSignal::is_aborted)
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

fn uses_reasoning_effort(model: &Model) -> bool {
    matches!(
        model.id.as_str(),
        "mistral-small-2603" | "mistral-small-latest" | "mistral-medium-3.5"
    )
}

fn mapped_effort(model: &Model, level: ThinkingLevel) -> MistralReasoningEffort {
    let key = ModelThinkingLevel::from(level);
    let mapped = model
        .thinking_level_map
        .as_ref()
        .and_then(|map| map.get(&key))
        .and_then(Clone::clone)
        .unwrap_or_else(|| "high".into());
    if mapped == "none" {
        MistralReasoningEffort::None
    } else {
        MistralReasoningEffort::High
    }
}

pub fn stream_simple_mistral(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let api_key = options
        .as_ref()
        .and_then(|value| value.base.api_key.as_deref())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?
        .to_string();
    let base = build_base_options(model, options.as_ref(), Some(&api_key));
    let reasoning = options
        .as_ref()
        .and_then(|value| value.reasoning)
        .and_then(|level| {
            let clamped = clamp_thinking_level(model, ModelThinkingLevel::from(level));
            match clamped {
                ModelThinkingLevel::Off => None,
                ModelThinkingLevel::Minimal => Some(ThinkingLevel::Minimal),
                ModelThinkingLevel::Low => Some(ThinkingLevel::Low),
                ModelThinkingLevel::Medium => Some(ThinkingLevel::Medium),
                ModelThinkingLevel::High | ModelThinkingLevel::XHigh | ModelThinkingLevel::Max => {
                    Some(ThinkingLevel::High)
                }
            }
        });
    Ok(stream_mistral(
        model,
        context,
        Some(MistralOptions {
            base,
            prompt_mode_reasoning: model.reasoning
                && reasoning.is_some()
                && !uses_reasoning_effort(model),
            reasoning_effort: reasoning
                .filter(|_| uses_reasoning_effort(model))
                .map(|level| mapped_effort(model, level)),
            tool_choice: None,
        }),
    ))
}
