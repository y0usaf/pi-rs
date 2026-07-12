//! Port of `providers/openai-responses.ts` and its shared Responses API
//! message/stream mapping. HTTP/SSE and retries reuse the common transport.

use std::collections::{BTreeMap, HashSet};

use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, CacheRetention,
    Context, Message, Modality, Model, ModelThinkingLevel, OpenAIResponsesCompat, ProviderResponse,
    StopReason, TextContent, TextOrImageContent, TextSignaturePhase, TextSignatureV1,
    ThinkingContent, ThinkingLevel, ThinkingType, Tool, ToolCall, ToolCallType, Usage, UserContent,
    calculate_cost, clamp_thinking_level, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::cloudflare::{is_cloudflare_provider, resolve_cloudflare_base_url};
use super::copilot_headers::{build_copilot_dynamic_headers, has_copilot_vision_input};
use super::openai_prompt_cache::clamp_openai_prompt_cache_key;
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::build_base_options;
use super::transform_messages::transform_messages;
use super::{ProtocolError, merge_header, merge_header_map, resolve_cache_retention};
use crate::transport::{
    AbortSignal, AssistantMessageEventStream, RetryOptions, RetryPolicy, TransportError,
    create_assistant_message_event_stream, post_with_retry, response_sse_reader,
};
use crate::util::{headers_to_record, parse_streaming_json, sanitize_surrogates};

const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 600_000;
const OPENAI_TOOL_CALL_PROVIDERS: &[&str] = &["openai", "openai-codex", "opencode"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningSummary {
    Auto,
    Detailed,
    Concise,
}

impl ReasoningSummary {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Detailed => "detailed",
            Self::Concise => "concise",
        }
    }
}

/// Provider-specific options accepted by `streamOpenAIResponses`.
#[derive(Clone, Default)]
pub struct OpenAIResponsesOptions {
    pub base: StreamOptions,
    pub reasoning_effort: Option<ThinkingLevel>,
    pub reasoning_summary: Option<ReasoningSummary>,
    pub service_tier: Option<String>,
}

#[derive(Clone, Copy)]
struct Compat {
    supports_developer_role: bool,
    send_session_id_header: bool,
    supports_long_cache_retention: bool,
}

fn get_compat(model: &Model) -> Compat {
    let compat = model
        .compat::<OpenAIResponsesCompat>()
        .ok()
        .flatten()
        .unwrap_or_default();
    Compat {
        supports_developer_role: compat.supports_developer_role.unwrap_or(true),
        send_session_id_header: compat.send_session_id_header.unwrap_or(true),
        supports_long_cache_retention: compat.supports_long_cache_retention.unwrap_or(true),
    }
}

fn base36(mut value: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize] as char);
        value /= 36;
    }
    out.iter().rev().collect()
}

/// Spec: `shortHash`, including JavaScript UTF-16 iteration and `Math.imul`.
fn short_hash(text: &str) -> String {
    let mut h1 = 0xdead_beefu32;
    let mut h2 = 0x41c6_ce57u32;
    for ch in text.encode_utf16() {
        h1 = (h1 ^ u32::from(ch)).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ u32::from(ch)).wrapping_mul(1_597_334_677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(3_266_489_909);
    format!("{}{}", base36(h2), base36(h1))
}

fn normalize_id_part(part: &str) -> String {
    let sanitized: String = part
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    sanitized.trim_end_matches('_').to_string()
}

fn parse_text_signature(signature: Option<&str>) -> Option<(String, Option<TextSignaturePhase>)> {
    let signature = signature?;
    if signature.starts_with('{')
        && let Ok(parsed) = serde_json::from_str::<TextSignatureV1>(signature)
        && parsed.v == 1
    {
        return Some((parsed.id, parsed.phase));
    }
    Some((signature.to_string(), None))
}

fn phase_value(phase: TextSignaturePhase) -> &'static str {
    match phase {
        TextSignaturePhase::Commentary => "commentary",
        TextSignaturePhase::FinalAnswer => "final_answer",
    }
}

fn normalize_tool_call_id(
    model: &Model,
    id: &str,
    source: &AssistantMessage,
    allowed: &HashSet<&str>,
) -> String {
    if !allowed.contains(model.provider.as_str()) {
        return normalize_id_part(id);
    }
    let Some((call_id, item_id)) = id.split_once('|') else {
        return normalize_id_part(id);
    };
    let call_id = normalize_id_part(call_id);
    let foreign = source.provider != model.provider || source.api != model.api;
    let mut item_id = if foreign {
        format!("fc_{}", short_hash(item_id))
    } else {
        normalize_id_part(item_id)
    };
    if !item_id.starts_with("fc_") {
        item_id = normalize_id_part(&format!("fc_{item_id}"));
    }
    format!("{call_id}|{item_id}")
}

/// Spec: `convertResponsesMessages`, shared by OpenAI and Codex Responses.
pub(crate) fn convert_responses_messages(
    model: &Model,
    context: &Context,
    include_system_prompt: bool,
) -> Vec<Value> {
    let allowed: HashSet<&str> = OPENAI_TOOL_CALL_PROVIDERS.iter().copied().collect();
    let normalize = |id: &str, _model: &Model, source: &AssistantMessage| {
        normalize_tool_call_id(model, id, source, &allowed)
    };
    let transformed = transform_messages(&context.messages, model, Some(&normalize));
    let mut messages = Vec::new();
    if include_system_prompt && let Some(system_prompt) = &context.system_prompt {
        let compat = get_compat(model);
        let role = if model.reasoning && compat.supports_developer_role {
            "developer"
        } else {
            "system"
        };
        messages.push(json!({ "role": role, "content": sanitize_surrogates(system_prompt) }));
    }

    for (msg_index, message) in transformed.into_iter().enumerate() {
        match message {
            Message::User(user) => match user.content {
                UserContent::Text(text) => messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": sanitize_surrogates(&text) }]
                })),
                UserContent::Blocks(blocks) => {
                    let content: Vec<Value> = blocks
                        .into_iter()
                        .map(|block| match block {
                            TextOrImageContent::Text(text) => json!({
                                "type": "input_text", "text": sanitize_surrogates(&text.text)
                            }),
                            TextOrImageContent::Image(image) => json!({
                                "type": "input_image", "detail": "auto",
                                "image_url": format!("data:{};base64,{}", image.mime_type, image.data)
                            }),
                        })
                        .collect();
                    if !content.is_empty() {
                        messages.push(json!({ "role": "user", "content": content }));
                    }
                }
            },
            Message::Assistant(assistant) => {
                let different_model = assistant.model != model.id
                    && assistant.provider == model.provider
                    && assistant.api == model.api;
                let mut output = Vec::new();
                let mut text_index = 0usize;
                for block in assistant.content.clone() {
                    match block {
                        AssistantContent::Thinking(thinking) => {
                            if let Some(signature) = thinking.thinking_signature
                                && let Ok(item) = serde_json::from_str::<Value>(&signature)
                            {
                                output.push(item);
                            }
                        }
                        AssistantContent::Text(text) => {
                            let parsed = parse_text_signature(text.text_signature.as_deref());
                            let fallback = if text_index == 0 {
                                format!("msg_pi_{msg_index}")
                            } else {
                                format!("msg_pi_{msg_index}_{text_index}")
                            };
                            text_index += 1;
                            let (mut id, phase) = parsed.unwrap_or((fallback, None));
                            if id.chars().count() > 64 {
                                id = format!("msg_{}", short_hash(&id));
                            }
                            let mut item = json!({
                                "type": "message", "role": "assistant",
                                "content": [{ "type": "output_text", "text": sanitize_surrogates(&text.text), "annotations": [] }],
                                "status": "completed", "id": id
                            });
                            if let Some(phase) = phase {
                                item["phase"] = json!(phase_value(phase));
                            }
                            output.push(item);
                        }
                        AssistantContent::ToolCall(tool_call) => {
                            let (call_id, raw_item_id) = tool_call
                                .id
                                .split_once('|')
                                .map_or((tool_call.id.as_str(), None), |(call, item)| {
                                    (call, Some(item))
                                });
                            let item_id = raw_item_id
                                .filter(|item| !(different_model && item.starts_with("fc_")));
                            let arguments = Value::Object(tool_call.arguments).to_string();
                            let item = if let Some(item_id) = item_id {
                                json!({
                                    "type": "function_call", "id": item_id,
                                    "call_id": call_id, "name": tool_call.name,
                                    "arguments": arguments
                                })
                            } else {
                                json!({
                                    "type": "function_call", "call_id": call_id,
                                    "name": tool_call.name, "arguments": arguments
                                })
                            };
                            output.push(item);
                        }
                    }
                }
                messages.extend(output);
            }
            Message::ToolResult(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        TextOrImageContent::Text(text) => Some(text.text.as_str()),
                        TextOrImageContent::Image(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let has_images = result
                    .content
                    .iter()
                    .any(|block| matches!(block, TextOrImageContent::Image(_)));
                let output = if has_images && model.input.contains(&Modality::Image) {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(
                            json!({ "type": "input_text", "text": sanitize_surrogates(&text) }),
                        );
                    }
                    for block in result.content {
                        if let TextOrImageContent::Image(image) = block {
                            parts.push(json!({
                                "type": "input_image", "detail": "auto",
                                "image_url": format!("data:{};base64,{}", image.mime_type, image.data)
                            }));
                        }
                    }
                    Value::Array(parts)
                } else {
                    json!(sanitize_surrogates(if text.is_empty() {
                        "(see attached image)"
                    } else {
                        &text
                    }))
                };
                let call_id = result.tool_call_id.split('|').next().unwrap_or("");
                messages.push(json!({
                    "type": "function_call_output", "call_id": call_id, "output": output
                }));
            }
        }
    }
    messages
}

fn convert_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function", "name": tool.name, "description": tool.description,
                "parameters": tool.parameters, "strict": false
            })
        })
        .collect()
}

fn mapped_effort(model: &Model, effort: ThinkingLevel) -> String {
    let level = ModelThinkingLevel::from(effort);
    model
        .thinking_level_map
        .as_ref()
        .and_then(|map| map.get(&level))
        .and_then(Clone::clone)
        .unwrap_or_else(|| {
            match effort {
                ThinkingLevel::Minimal => "minimal",
                ThinkingLevel::Low => "low",
                ThinkingLevel::Medium => "medium",
                ThinkingLevel::High => "high",
                ThinkingLevel::XHigh => "xhigh",
                ThinkingLevel::Max => "max",
            }
            .to_string()
        })
}

fn build_params(model: &Model, context: &Context, options: &OpenAIResponsesOptions) -> Value {
    let cache_retention = resolve_cache_retention(options.base.cache_retention);
    let compat = get_compat(model);
    let mut object = Map::new();
    object.insert("model".to_string(), json!(model.id));
    object.insert(
        "input".to_string(),
        json!(convert_responses_messages(model, context, true)),
    );
    object.insert("stream".to_string(), json!(true));
    object.insert("store".to_string(), json!(false));
    if cache_retention != CacheRetention::None
        && let Some(key) = clamp_openai_prompt_cache_key(options.base.session_id.as_deref())
    {
        object.insert("prompt_cache_key".to_string(), json!(key));
    }
    if cache_retention == CacheRetention::Long && compat.supports_long_cache_retention {
        object.insert("prompt_cache_retention".to_string(), json!("24h"));
    }
    if let Some(max_tokens) = options.base.max_tokens {
        object.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = options.base.temperature {
        object.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(service_tier) = &options.service_tier {
        object.insert("service_tier".to_string(), json!(service_tier));
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
            if model.provider != "github-copilot" && !matches!(explicit_off, Some(None)) {
                let effort = explicit_off
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| "none".to_string());
                object.insert("reasoning".to_string(), json!({ "effort": effort }));
            }
        }
    }
    Value::Object(object)
}

struct PreparedRequest {
    url: String,
    headers: HeaderMap,
}

fn create_request(
    model: &Model,
    context: &Context,
    api_key: &str,
    options_headers: Option<&BTreeMap<String, String>>,
    session_id: Option<&str>,
) -> Result<PreparedRequest, ProtocolError> {
    let compat = get_compat(model);
    let mut defaults = Vec::new();
    merge_header_map(&mut defaults, model.headers.as_ref());
    if model.provider == "github-copilot" {
        let has_images = has_copilot_vision_input(&context.messages);
        for (key, value) in build_copilot_dynamic_headers(&context.messages, has_images) {
            merge_header(&mut defaults, &key, &value);
        }
    }
    if let Some(session_id) = session_id {
        if compat.send_session_id_header {
            merge_header(&mut defaults, "session_id", session_id);
        }
        merge_header(&mut defaults, "x-client-request-id", session_id);
    }
    merge_header_map(&mut defaults, options_headers);

    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("accept".to_string(), "application/json".to_string()),
        ("authorization".to_string(), format!("Bearer {api_key}")),
    ];
    if model.provider == "cloudflare-ai-gateway" {
        if !defaults.iter().any(|(key, _)| key == "authorization") {
            headers.retain(|(key, _)| key != "authorization");
        }
        merge_header(
            &mut headers,
            "cf-aig-authorization",
            &format!("Bearer {api_key}"),
        );
    }
    for (key, value) in defaults {
        merge_header(&mut headers, &key, &value);
    }
    let mut header_map = HeaderMap::new();
    for (key, value) in headers {
        header_map.insert(
            HeaderName::from_bytes(key.as_bytes())
                .map_err(|error| ProtocolError(error.to_string()))?,
            HeaderValue::from_str(&value).map_err(|error| ProtocolError(error.to_string()))?,
        );
    }
    let base_url = if is_cloudflare_provider(&model.provider) {
        resolve_cloudflare_base_url(model)?
    } else {
        model.base_url.clone()
    };
    Ok(PreparedRequest {
        url: format!("{}/responses", base_url.trim_end_matches('/')),
        headers: header_map,
    })
}

fn arguments_from(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn encode_text_signature(id: &str, phase: Option<&str>) -> String {
    let mut value = json!({ "v": 1, "id": id });
    if matches!(phase, Some("commentary" | "final_answer")) {
        value["phase"] = json!(phase);
    }
    value.to_string()
}

fn service_tier_multiplier(model: &Model, tier: Option<&str>) -> f64 {
    match tier {
        Some("flex") => 0.5,
        Some("priority") if model.id == "gpt-5.5" => 2.5,
        Some("priority") => 2.0,
        _ => 1.0,
    }
}

fn apply_service_tier_pricing(usage: &mut Usage, model: &Model, tier: Option<&str>) {
    let multiplier = service_tier_multiplier(model, tier);
    usage.cost.input *= multiplier;
    usage.cost.output *= multiplier;
    usage.cost.cache_read *= multiplier;
    usage.cost.cache_write *= multiplier;
    usage.cost.total =
        usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;
}

#[derive(Clone, Copy)]
enum CurrentBlock {
    Thinking(usize),
    Text(usize),
    Tool(usize),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ResponsesFlavor {
    OpenAi,
    Codex,
}

pub(crate) async fn process_responses_stream(
    response: reqwest::Response,
    signal: Option<AbortSignal>,
    output: &mut AssistantMessage,
    stream: &AssistantMessageEventStream,
    model: &Model,
    service_tier: Option<&str>,
    flavor: ResponsesFlavor,
) -> Result<(), ProtocolError> {
    let mut reader = response_sse_reader(response, signal);
    let mut current_item = Value::Null;
    let mut current_block = None;
    let mut partial_json = String::new();
    while let Some(sse) = reader
        .next()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?
    {
        if sse.data.is_empty() || sse.data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(&sse.data).map_err(|error| {
            ProtocolError(format!(
                "Could not parse OpenAI SSE chunk: {error}; data={}",
                sse.data
            ))
        })?;
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "response.created" => {
                output.response_id = event
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "response.output_item.added" => {
                current_item = event.get("item").cloned().unwrap_or(Value::Null);
                match current_item.get("type").and_then(Value::as_str) {
                    Some("reasoning") => {
                        output
                            .content
                            .push(AssistantContent::Thinking(ThinkingContent {
                                r#type: ThinkingType::Thinking,
                                thinking: String::new(),
                                thinking_signature: None,
                                redacted: None,
                            }));
                        let index = output.content.len() - 1;
                        current_block = Some(CurrentBlock::Thinking(index));
                        stream.push(AssistantMessageEvent::ThinkingStart {
                            content_index: index,
                            partial: output.clone(),
                        });
                    }
                    Some("message") => {
                        output
                            .content
                            .push(AssistantContent::Text(TextContent::new("")));
                        let index = output.content.len() - 1;
                        current_block = Some(CurrentBlock::Text(index));
                        stream.push(AssistantMessageEvent::TextStart {
                            content_index: index,
                            partial: output.clone(),
                        });
                    }
                    Some("function_call") => {
                        partial_json = current_item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        output.content.push(AssistantContent::ToolCall(ToolCall {
                            r#type: ToolCallType::ToolCall,
                            id: format!(
                                "{}|{}",
                                current_item
                                    .get("call_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or(""),
                                current_item.get("id").and_then(Value::as_str).unwrap_or("")
                            ),
                            name: current_item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            arguments: Map::new(),
                            thought_signature: None,
                        }));
                        let index = output.content.len() - 1;
                        current_block = Some(CurrentBlock::Tool(index));
                        stream.push(AssistantMessageEvent::ToolCallStart {
                            content_index: index,
                            partial: output.clone(),
                        });
                    }
                    _ => {}
                }
            }
            "response.reasoning_summary_part.added" => {
                if current_item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    if let Some(summary) = current_item["summary"].as_array_mut() {
                        summary.push(event["part"].clone());
                    } else {
                        current_item["summary"] = json!([event["part"].clone()]);
                    }
                }
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(CurrentBlock::Thinking(index)) = current_block
                    && let Some(delta) = event.get("delta").and_then(Value::as_str)
                    && let Some(last) = current_item["summary"]
                        .as_array_mut()
                        .and_then(|summary| summary.last_mut())
                {
                    if let Some(AssistantContent::Thinking(block)) = output.content.get_mut(index) {
                        block.thinking.push_str(delta);
                    }
                    let text = last
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                        + delta;
                    last["text"] = json!(text);
                    stream.push(AssistantMessageEvent::ThinkingDelta {
                        content_index: index,
                        delta: delta.to_string(),
                        partial: output.clone(),
                    });
                }
            }
            "response.reasoning_summary_part.done" => {
                if let Some(CurrentBlock::Thinking(index)) = current_block
                    && let Some(last) = current_item["summary"]
                        .as_array_mut()
                        .and_then(|summary| summary.last_mut())
                {
                    if let Some(AssistantContent::Thinking(block)) = output.content.get_mut(index) {
                        block.thinking.push_str("\n\n");
                    }
                    let text = last
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                        + "\n\n";
                    last["text"] = json!(text);
                    stream.push(AssistantMessageEvent::ThinkingDelta {
                        content_index: index,
                        delta: "\n\n".to_string(),
                        partial: output.clone(),
                    });
                }
            }
            "response.reasoning_text.delta" => {
                if let Some(CurrentBlock::Thinking(index)) = current_block
                    && let Some(delta) = event.get("delta").and_then(Value::as_str)
                {
                    if let Some(AssistantContent::Thinking(block)) = output.content.get_mut(index) {
                        block.thinking.push_str(delta);
                    }
                    stream.push(AssistantMessageEvent::ThinkingDelta {
                        content_index: index,
                        delta: delta.to_string(),
                        partial: output.clone(),
                    });
                }
            }
            "response.content_part.added" => {
                if current_item.get("type").and_then(Value::as_str) == Some("message")
                    && matches!(
                        event.pointer("/part/type").and_then(Value::as_str),
                        Some("output_text" | "refusal")
                    )
                {
                    if !current_item.get("content").is_some_and(Value::is_array) {
                        current_item["content"] = json!([]);
                    }
                    if let Some(content) = current_item["content"].as_array_mut() {
                        content.push(event["part"].clone());
                    }
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(CurrentBlock::Text(index)) = current_block
                    && let Some(delta) = event.get("delta").and_then(Value::as_str)
                    && let Some(last) = current_item["content"]
                        .as_array_mut()
                        .and_then(|content| content.last_mut())
                {
                    let expected = if event_type == "response.output_text.delta" {
                        "output_text"
                    } else {
                        "refusal"
                    };
                    if last.get("type").and_then(Value::as_str) == Some(expected) {
                        if let Some(AssistantContent::Text(block)) = output.content.get_mut(index) {
                            block.text.push_str(delta);
                        }
                        let field = if expected == "output_text" {
                            "text"
                        } else {
                            "refusal"
                        };
                        let text = last
                            .get(field)
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                            + delta;
                        last[field] = json!(text);
                        stream.push(AssistantMessageEvent::TextDelta {
                            content_index: index,
                            delta: delta.to_string(),
                            partial: output.clone(),
                        });
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(CurrentBlock::Tool(index)) = current_block
                    && let Some(delta) = event.get("delta").and_then(Value::as_str)
                {
                    partial_json.push_str(delta);
                    if let Some(AssistantContent::ToolCall(block)) = output.content.get_mut(index) {
                        block.arguments = arguments_from(parse_streaming_json(&partial_json));
                    }
                    stream.push(AssistantMessageEvent::ToolCallDelta {
                        content_index: index,
                        delta: delta.to_string(),
                        partial: output.clone(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(CurrentBlock::Tool(index)) = current_block
                    && let Some(arguments) = event.get("arguments").and_then(Value::as_str)
                {
                    let previous = partial_json.clone();
                    partial_json = arguments.to_string();
                    if let Some(AssistantContent::ToolCall(block)) = output.content.get_mut(index) {
                        block.arguments = arguments_from(parse_streaming_json(&partial_json));
                    }
                    if let Some(delta) = arguments.strip_prefix(&previous)
                        && !delta.is_empty()
                    {
                        stream.push(AssistantMessageEvent::ToolCallDelta {
                            content_index: index,
                            delta: delta.to_string(),
                            partial: output.clone(),
                        });
                    }
                }
            }
            "response.output_item.done" => {
                let item = &event["item"];
                match (item.get("type").and_then(Value::as_str), current_block) {
                    (Some("reasoning"), Some(CurrentBlock::Thinking(index))) => {
                        let joined = |field: &str| {
                            item.get(field)
                                .and_then(Value::as_array)
                                .map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                                        .collect::<Vec<_>>()
                                        .join("\n\n")
                                })
                                .unwrap_or_default()
                        };
                        if let Some(AssistantContent::Thinking(block)) =
                            output.content.get_mut(index)
                        {
                            let final_text = joined("summary");
                            let final_text = if final_text.is_empty() {
                                joined("content")
                            } else {
                                final_text
                            };
                            if !final_text.is_empty() {
                                block.thinking = final_text;
                            }
                            block.thinking_signature = Some(item.to_string());
                            let content = block.thinking.clone();
                            stream.push(AssistantMessageEvent::ThinkingEnd {
                                content_index: index,
                                content,
                                partial: output.clone(),
                            });
                        }
                        current_block = None;
                    }
                    (Some("message"), Some(CurrentBlock::Text(index))) => {
                        let text = item
                            .get("content")
                            .and_then(Value::as_array)
                            .map(|parts| {
                                parts
                                    .iter()
                                    .map(|part| {
                                        if part.get("type").and_then(Value::as_str)
                                            == Some("output_text")
                                        {
                                            part.get("text").and_then(Value::as_str).unwrap_or("")
                                        } else {
                                            part.get("refusal")
                                                .and_then(Value::as_str)
                                                .unwrap_or("")
                                        }
                                    })
                                    .collect::<String>()
                            })
                            .unwrap_or_default();
                        if let Some(AssistantContent::Text(block)) = output.content.get_mut(index) {
                            block.text = text.clone();
                            block.text_signature = Some(encode_text_signature(
                                item.get("id").and_then(Value::as_str).unwrap_or(""),
                                item.get("phase").and_then(Value::as_str),
                            ));
                        }
                        stream.push(AssistantMessageEvent::TextEnd {
                            content_index: index,
                            content: text,
                            partial: output.clone(),
                        });
                        current_block = None;
                    }
                    (Some("function_call"), _) => {
                        let index = match current_block {
                            Some(CurrentBlock::Tool(index)) => index,
                            _ => output.content.len(),
                        };
                        let args = if !partial_json.is_empty() {
                            parse_streaming_json(&partial_json)
                        } else {
                            parse_streaming_json(
                                item.get("arguments")
                                    .and_then(Value::as_str)
                                    .unwrap_or("{}"),
                            )
                        };
                        let tool_call = if let Some(AssistantContent::ToolCall(block)) =
                            output.content.get_mut(index)
                        {
                            block.arguments = arguments_from(args);
                            block.clone()
                        } else {
                            let call = ToolCall {
                                r#type: ToolCallType::ToolCall,
                                id: format!(
                                    "{}|{}",
                                    item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                                    item.get("id").and_then(Value::as_str).unwrap_or("")
                                ),
                                name: item
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                arguments: arguments_from(args),
                                thought_signature: None,
                            };
                            output
                                .content
                                .push(AssistantContent::ToolCall(call.clone()));
                            call
                        };
                        let actual_index = output.content.len().saturating_sub(1).min(index);
                        stream.push(AssistantMessageEvent::ToolCallEnd {
                            content_index: actual_index,
                            tool_call,
                            partial: output.clone(),
                        });
                        current_block = None;
                        partial_json.clear();
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.done" | "response.incomplete"
                if flavor == ResponsesFlavor::Codex || event_type == "response.completed" =>
            {
                let response = &event["response"];
                if let Some(id) = response.get("id").and_then(Value::as_str) {
                    output.response_id = Some(id.to_string());
                }
                if let Some(usage) = response.get("usage") {
                    let cached = usage
                        .pointer("/input_tokens_details/cached_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let input = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    output.usage = Usage {
                        input: input.saturating_sub(cached),
                        output: usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        cache_read: cached,
                        cache_write: 0,
                        total_tokens: usage
                            .get("total_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        cost: Default::default(),
                    };
                    calculate_cost(model, &mut output.usage);
                    let response_tier = response.get("service_tier").and_then(Value::as_str);
                    let tier = if flavor == ResponsesFlavor::Codex
                        && response_tier == Some("default")
                        && matches!(service_tier, Some("flex" | "priority"))
                    {
                        service_tier
                    } else {
                        response_tier.or(service_tier)
                    };
                    apply_service_tier_pricing(&mut output.usage, model, tier);
                }
                output.stop_reason = match response.get("status").and_then(Value::as_str) {
                    Some("incomplete") => StopReason::Length,
                    Some("failed" | "cancelled") => StopReason::Error,
                    _ => StopReason::Stop,
                };
                if output
                    .content
                    .iter()
                    .any(|block| matches!(block, AssistantContent::ToolCall(_)))
                    && output.stop_reason == StopReason::Stop
                {
                    output.stop_reason = StopReason::ToolUse;
                }
                if flavor == ResponsesFlavor::Codex {
                    break;
                }
            }
            "error" => {
                let code = event.get("code").and_then(Value::as_str).unwrap_or("");
                let message = event.get("message").and_then(Value::as_str).unwrap_or("");
                return Err(ProtocolError(if flavor == ResponsesFlavor::Codex {
                    let detail = if !message.is_empty() {
                        message.to_string()
                    } else if !code.is_empty() {
                        code.to_string()
                    } else {
                        event.to_string()
                    };
                    format!("Codex error: {detail}")
                } else {
                    format!("Error Code {code}: {message}")
                }));
            }
            "response.failed" => {
                let error = &event["response"]["error"];
                if flavor == ResponsesFlavor::Codex {
                    return Err(ProtocolError(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Codex response failed")
                            .to_string(),
                    ));
                }
                let message = if !error.is_null() {
                    format!(
                        "{}: {}",
                        error
                            .get("code")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("no message")
                    )
                } else if let Some(reason) = event
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                {
                    format!("incomplete: {reason}")
                } else {
                    "Unknown error (no error details in response)".to_string()
                };
                return Err(ProtocolError(message));
            }
            _ => {}
        }
    }
    Ok(())
}

fn format_transport_error(error: &TransportError) -> String {
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
            format!("OpenAI API error ({status}): {status} {message}")
        }
        _ => error.to_string(),
    }
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &OpenAIResponsesOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let api_key = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;
    let cache = resolve_cache_retention(options.base.cache_retention);
    let session_id = if cache == CacheRetention::None {
        None
    } else {
        options.base.session_id.as_deref()
    };
    let request = create_request(
        model,
        context,
        api_key,
        options.base.headers.as_ref(),
        session_id,
    )?;
    let mut params = build_params(model, context, options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model)
    {
        params = next;
    }
    let response = post_with_retry(
        &reqwest::Client::new(),
        &request.url,
        &request.headers,
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
    .map_err(|error| ProtocolError(format_transport_error(&error)))?;
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
        ResponsesFlavor::OpenAi,
    )
    .await?;
    if options
        .base
        .signal
        .as_ref()
        .is_some_and(AbortSignal::is_aborted)
    {
        return Err(ProtocolError("Request was aborted".to_string()));
    }
    if matches!(output.stop_reason, StopReason::Aborted | StopReason::Error) {
        return Err(ProtocolError("An unknown error occurred".to_string()));
    }
    Ok(())
}

pub fn stream_openai_responses(
    model: &Model,
    context: &Context,
    options: Option<OpenAIResponsesOptions>,
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
                    .is_some_and(AbortSignal::is_aborted)
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

pub fn stream_simple_openai_responses(
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
    Ok(stream_openai_responses(
        model,
        context,
        Some(OpenAIResponsesOptions {
            base,
            reasoning_effort,
            reasoning_summary: None,
            service_tier: None,
        }),
    ))
}
