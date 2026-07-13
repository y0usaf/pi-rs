//! Port of `providers/openai-completions.ts` — the OpenAI Chat
//! Completions protocol (and every OpenAI-compatible endpoint pi routes
//! through it: openrouter, deepseek, z.ai, together, moonshot, xai,
//! cerebras, nvidia, cloudflare, vercel gateway, …).
//!
//! The spec drives the OpenAI SDK; the SDK's contribution is reproduced
//! explicitly and pinned by the replay/request tests in
//! `tests/openai_completions.rs`:
//!
//! - `client.chat.completions.create(params).withResponse()` →
//!   [`crate::transport`] `post_with_retry` against
//!   `{baseUrl}/chat/completions` with `Authorization: Bearer` auth;
//!   `defaultHeaders` merge over the SDK defaults, and a `null`-valued
//!   `Authorization` default header (the cloudflare-ai-gateway branch)
//!   suppresses the Bearer header, as in the SDK;
//! - `maxRetries`/`timeout` request options → [`RetryOptions`] (the
//!   SDK's default timeout is 10 minutes,
//!   [`DEFAULT_OPENAI_COMPLETIONS_TIMEOUT_MS`]);
//! - the SDK's SSE chunk iterator → the transport [`SseReader`]
//!   (`data:`-only events); the `[DONE]` sentinel ends the stream and a
//!   chunk that fails to parse as JSON is an error, as in the SDK.
//!
//! Divergences (mechanism only):
//! - the spec's catch reads `error.error.metadata.raw` off SDK
//!   `APIError` objects (OpenRouter detail); here the raw metadata is
//!   extracted from the HTTP error body at the transport seam and
//!   appended to the message the same way;
//! - `toolCall.thoughtSignature` is `JSON.stringify(detail)`; serde
//!   serialization may order keys differently than V8, but the value is
//!   opaque and is only ever `JSON.parse`d back (semantic parity);
//! - `streamSimpleOpenAICompletions` reads a `toolChoice` the
//!   `SimpleStreamOptions` type does not declare (a TS cast); callers
//!   with a tool choice use [`stream_openai_completions`] directly.
//!
//! [`SseReader`]: crate::transport::SseReader

use std::collections::BTreeMap;

use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, CacheControlFormat,
    CacheRetention, Context, MaxTokensField, Message, Model, ModelThinkingLevel, ProviderResponse,
    StopReason, TextContent, ThinkingContent, ThinkingFormat, ThinkingLevel, ThinkingType, Tool,
    ToolCall, ToolCallType, ToolResultMessage, Usage, UserContent, calculate_cost,
    clamp_thinking_level, now_ms,
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
    AbortSignal, AssistantMessageEventStream, RetryOptions, TransportError,
    create_assistant_message_event_stream, post_with_retry, response_sse_reader,
};
use crate::util::{headers_to_record, parse_streaming_json, sanitize_surrogates};

/// Spec: SDK request-option `timeout` default (OpenAI SDK: 10 min).
pub const DEFAULT_OPENAI_COMPLETIONS_TIMEOUT_MS: u64 = 600_000;

/// Spec: `OpenAICompletionsOptions["toolChoice"]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenAIToolChoice {
    Auto,
    None,
    Required,
    Function { name: String },
}

impl OpenAIToolChoice {
    fn to_value(&self) -> Value {
        match self {
            Self::Auto => json!("auto"),
            Self::None => json!("none"),
            Self::Required => json!("required"),
            Self::Function { name } => {
                json!({ "type": "function", "function": { "name": name } })
            }
        }
    }
}

/// Spec: `OpenAICompletionsOptions` (`StreamOptions` + tool choice +
/// reasoning effort).
#[derive(Clone, Default)]
pub struct OpenAICompletionsOptions {
    pub base: StreamOptions,
    pub tool_choice: Option<OpenAIToolChoice>,
    /// `"minimal" | "low" | "medium" | "high" | "xhigh"`.
    pub reasoning_effort: Option<ThinkingLevel>,
}

/// Spec: `hasToolHistory` — Anthropic (via proxy) requires the tools
/// param when messages include tool calls or tool results.
fn has_tool_history(messages: &[Message]) -> bool {
    messages.iter().any(|msg| match msg {
        Message::ToolResult(_) => true,
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .any(|block| matches!(block, AssistantContent::ToolCall(_))),
        Message::User(_) => false,
    })
}

// ---------------------------------------------------------------------
// Compat detection
// ---------------------------------------------------------------------

/// Spec: `ResolvedOpenAICompletionsCompat` (the routing preferences are
/// read off the raw `model.compat` in `buildParams`, as in the spec).
struct ResolvedCompat {
    supports_store: bool,
    supports_developer_role: bool,
    supports_reasoning_effort: bool,
    supports_usage_in_streaming: bool,
    max_tokens_field: MaxTokensField,
    requires_tool_result_name: bool,
    requires_assistant_after_tool_result: bool,
    requires_thinking_as_text: bool,
    requires_reasoning_content_on_assistant_messages: bool,
    thinking_format: ThinkingFormat,
    zai_tool_stream: bool,
    supports_strict_mode: bool,
    cache_control_format: Option<CacheControlFormat>,
    send_session_affinity_headers: bool,
    supports_long_cache_retention: bool,
}

/// Spec: `detectCompat` — provider takes precedence over URL detection.
#[allow(clippy::struct_excessive_bools)]
fn detect_compat(model: &Model) -> ResolvedCompat {
    let provider = model.provider.as_str();
    let base_url = model.base_url.as_str();

    let is_zai = provider == "zai"
        || provider == "zai-coding-cn"
        || base_url.contains("api.z.ai")
        || base_url.contains("open.bigmodel.cn");
    let is_together = provider == "together"
        || base_url.contains("api.together.ai")
        || base_url.contains("api.together.xyz");
    let is_moonshot = provider == "moonshotai"
        || provider == "moonshotai-cn"
        || base_url.contains("api.moonshot.");
    let is_openrouter = provider == "openrouter" || base_url.contains("openrouter.ai");
    let is_cloudflare_workers_ai =
        provider == "cloudflare-workers-ai" || base_url.contains("api.cloudflare.com");
    let is_cloudflare_ai_gateway =
        provider == "cloudflare-ai-gateway" || base_url.contains("gateway.ai.cloudflare.com");
    let is_nvidia = provider == "nvidia" || base_url.contains("integrate.api.nvidia.com");
    let is_ant_ling = provider == "ant-ling" || base_url.contains("api.ant-ling.com");

    let is_non_standard = is_nvidia
        || provider == "cerebras"
        || base_url.contains("cerebras.ai")
        || provider == "xai"
        || base_url.contains("api.x.ai")
        || is_together
        || base_url.contains("chutes.ai")
        || base_url.contains("deepseek.com")
        || is_zai
        || is_moonshot
        || provider == "opencode"
        || base_url.contains("opencode.ai")
        || is_cloudflare_workers_ai
        || is_cloudflare_ai_gateway
        || is_ant_ling;

    let use_max_tokens = base_url.contains("chutes.ai")
        || is_moonshot
        || is_cloudflare_ai_gateway
        || is_together
        || is_nvidia
        || is_ant_ling;

    let is_grok = provider == "xai" || base_url.contains("api.x.ai");
    let is_deepseek = provider == "deepseek" || base_url.contains("deepseek.com");
    let is_openrouter_developer_role_model =
        is_openrouter && (model.id.starts_with("anthropic/") || model.id.starts_with("openai/"));
    let cache_control_format = if provider == "openrouter" && model.id.starts_with("anthropic/") {
        Some(CacheControlFormat::Anthropic)
    } else {
        None
    };

    ResolvedCompat {
        supports_store: !is_non_standard,
        supports_developer_role: is_openrouter_developer_role_model
            || (!is_non_standard && !is_openrouter),
        supports_reasoning_effort: !is_grok
            && !is_zai
            && !is_moonshot
            && !is_together
            && !is_cloudflare_ai_gateway
            && !is_nvidia
            && !is_ant_ling,
        supports_usage_in_streaming: true,
        max_tokens_field: if use_max_tokens {
            MaxTokensField::MaxTokens
        } else {
            MaxTokensField::MaxCompletionTokens
        },
        requires_tool_result_name: false,
        requires_assistant_after_tool_result: false,
        requires_thinking_as_text: false,
        requires_reasoning_content_on_assistant_messages: is_deepseek,
        thinking_format: if is_deepseek {
            ThinkingFormat::Deepseek
        } else if is_zai {
            ThinkingFormat::Zai
        } else if is_together {
            ThinkingFormat::Together
        } else if is_ant_ling {
            ThinkingFormat::AntLing
        } else if is_openrouter {
            ThinkingFormat::Openrouter
        } else {
            ThinkingFormat::Openai
        },
        zai_tool_stream: false,
        supports_strict_mode: !is_moonshot
            && !is_together
            && !is_cloudflare_ai_gateway
            && !is_nvidia,
        cache_control_format,
        send_session_affinity_headers: false,
        supports_long_cache_retention: !(is_together
            || is_cloudflare_workers_ai
            || is_cloudflare_ai_gateway
            || is_nvidia
            || is_ant_ling),
    }
}

/// Spec: `getCompat` — explicit `model.compat` fields win over detection.
fn get_compat(model: &Model) -> ResolvedCompat {
    let detected = detect_compat(model);
    let Ok(Some(compat)) = model.compat::<pi_rs_ai_types::OpenAICompletionsCompat>() else {
        return detected;
    };
    ResolvedCompat {
        supports_store: compat.supports_store.unwrap_or(detected.supports_store),
        supports_developer_role: compat
            .supports_developer_role
            .unwrap_or(detected.supports_developer_role),
        supports_reasoning_effort: compat
            .supports_reasoning_effort
            .unwrap_or(detected.supports_reasoning_effort),
        supports_usage_in_streaming: compat
            .supports_usage_in_streaming
            .unwrap_or(detected.supports_usage_in_streaming),
        max_tokens_field: compat.max_tokens_field.unwrap_or(detected.max_tokens_field),
        requires_tool_result_name: compat
            .requires_tool_result_name
            .unwrap_or(detected.requires_tool_result_name),
        requires_assistant_after_tool_result: compat
            .requires_assistant_after_tool_result
            .unwrap_or(detected.requires_assistant_after_tool_result),
        requires_thinking_as_text: compat
            .requires_thinking_as_text
            .unwrap_or(detected.requires_thinking_as_text),
        requires_reasoning_content_on_assistant_messages: compat
            .requires_reasoning_content_on_assistant_messages
            .unwrap_or(detected.requires_reasoning_content_on_assistant_messages),
        thinking_format: compat.thinking_format.unwrap_or(detected.thinking_format),
        zai_tool_stream: compat.zai_tool_stream.unwrap_or(detected.zai_tool_stream),
        supports_strict_mode: compat
            .supports_strict_mode
            .unwrap_or(detected.supports_strict_mode),
        cache_control_format: compat
            .cache_control_format
            .or(detected.cache_control_format),
        send_session_affinity_headers: compat
            .send_session_affinity_headers
            .unwrap_or(detected.send_session_affinity_headers),
        supports_long_cache_retention: compat
            .supports_long_cache_retention
            .unwrap_or(detected.supports_long_cache_retention),
    }
}

// ---------------------------------------------------------------------
// Request assembly
// ---------------------------------------------------------------------

/// Spec: the `normalizeToolCallId` closure in `convertMessages` — pipe
/// IDs from the Responses API are split, sanitized and clamped to 40;
/// the openai provider clamps to 40 unconditionally.
fn normalize_tool_call_id(model: &Model, id: &str) -> String {
    if id.contains('|') {
        let call_id = id.split('|').next().unwrap_or_default();
        return call_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect();
    }
    if model.provider == "openai" && id.chars().count() > 40 {
        return id.chars().take(40).collect();
    }
    id.to_string()
}

/// The spec's `createClient` — URL and headers per provider branch (SDK
/// defaults reproduced, see module docs).
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
    compat: &ResolvedCompat,
) -> Result<PreparedRequest, ProtocolError> {
    // Spec: headers = { ...model.headers } + copilot + affinity + options.
    let mut default_headers: Vec<(String, String)> = Vec::new();
    merge_header_map(&mut default_headers, model.headers.as_ref());
    if model.provider == "github-copilot" {
        let has_images = has_copilot_vision_input(&context.messages);
        for (key, value) in build_copilot_dynamic_headers(&context.messages, has_images) {
            merge_header(&mut default_headers, &key, &value);
        }
    }
    if let Some(session_id) = session_id
        && compat.send_session_affinity_headers
    {
        merge_header(&mut default_headers, "session_id", session_id);
        merge_header(&mut default_headers, "x-client-request-id", session_id);
        merge_header(&mut default_headers, "x-session-affinity", session_id);
    }
    // Merge options headers last so they can override defaults.
    merge_header_map(&mut default_headers, options_headers);

    // SDK defaults, then the default headers merged over them.
    let mut headers: Vec<(String, String)> = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("accept".to_string(), "application/json".to_string()),
        ("authorization".to_string(), format!("Bearer {api_key}")),
    ];
    if model.provider == "cloudflare-ai-gateway" {
        // Spec: Authorization: headers.Authorization ?? null — a missing
        // model Authorization suppresses the SDK Bearer default.
        if !default_headers.iter().any(|(k, _)| k == "authorization") {
            headers.retain(|(k, _)| k != "authorization");
        }
        merge_header(
            &mut headers,
            "cf-aig-authorization",
            &format!("Bearer {api_key}"),
        );
    }
    for (key, value) in &default_headers {
        merge_header(&mut headers, key, value);
    }

    let base_url = if is_cloudflare_provider(&model.provider) {
        resolve_cloudflare_base_url(model)?
    } else {
        model.base_url.clone()
    };

    let mut header_map = HeaderMap::new();
    for (key, value) in &headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|error| ProtocolError(error.to_string()))?;
        let value =
            HeaderValue::from_str(value).map_err(|error| ProtocolError(error.to_string()))?;
        header_map.insert(name, value);
    }

    Ok(PreparedRequest {
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        headers: header_map,
    })
}

/// Spec: `getCompatCacheControl`.
fn get_compat_cache_control(
    compat: &ResolvedCompat,
    cache_retention: CacheRetention,
) -> Option<Value> {
    if compat.cache_control_format != Some(CacheControlFormat::Anthropic)
        || cache_retention == CacheRetention::None
    {
        return None;
    }
    let mut cache_control = json!({ "type": "ephemeral" });
    if cache_retention == CacheRetention::Long && compat.supports_long_cache_retention {
        cache_control["ttl"] = json!("1h");
    }
    Some(cache_control)
}

/// Spec: `addCacheControlToTextContent` — string content becomes a
/// single cached text part; array content caches its last text part.
fn add_cache_control_to_text_content(message: &mut Value, cache_control: &Value) -> bool {
    match message.get_mut("content") {
        Some(content @ Value::String(_)) => {
            let Some(text) = content.as_str() else {
                return false;
            };
            if text.is_empty() {
                return false;
            }
            let text = text.to_string();
            *content = json!([{
                "type": "text",
                "text": text,
                "cache_control": cache_control,
            }]);
            true
        }
        Some(Value::Array(parts)) => {
            for part in parts.iter_mut().rev() {
                if part.get("type").and_then(Value::as_str) == Some("text") {
                    part["cache_control"] = cache_control.clone();
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// Spec: `applyAnthropicCacheControl` — system prompt, last tool, last
/// user/assistant message.
fn apply_anthropic_cache_control(
    messages: &mut [Value],
    tools: Option<&mut Value>,
    cache_control: &Value,
) {
    // addCacheControlToSystemPrompt
    for message in messages.iter_mut() {
        if matches!(
            message.get("role").and_then(Value::as_str),
            Some("system" | "developer")
        ) {
            add_cache_control_to_text_content(message, cache_control);
            break;
        }
    }
    // addCacheControlToLastTool
    if let Some(Value::Array(tools)) = tools
        && let Some(last) = tools.last_mut()
    {
        last["cache_control"] = cache_control.clone();
    }
    // addCacheControlToLastConversationMessage
    for message in messages.iter_mut().rev() {
        if matches!(
            message.get("role").and_then(Value::as_str),
            Some("user" | "assistant")
        ) && add_cache_control_to_text_content(message, cache_control)
        {
            return;
        }
    }
}

/// Spec: `convertMessages`.
fn convert_messages(model: &Model, context: &Context, compat: &ResolvedCompat) -> Vec<Value> {
    use pi_rs_ai_types::TextOrImageContent;

    let normalize =
        |id: &str, model: &Model, _source: &AssistantMessage| normalize_tool_call_id(model, id);
    let transformed = transform_messages(&context.messages, model, Some(&normalize));

    let mut params: Vec<Value> = Vec::new();

    if let Some(system_prompt) = &context.system_prompt {
        let use_developer_role = model.reasoning && compat.supports_developer_role;
        let role = if use_developer_role {
            "developer"
        } else {
            "system"
        };
        params.push(json!({ "role": role, "content": sanitize_surrogates(system_prompt) }));
    }

    let mut last_role: Option<&str> = None;

    let mut i = 0;
    while i < transformed.len() {
        let msg = &transformed[i];
        // Some providers don't allow user messages directly after tool
        // results; bridge with a synthetic assistant message.
        if compat.requires_assistant_after_tool_result
            && last_role == Some("toolResult")
            && matches!(msg, Message::User(_))
        {
            params.push(json!({
                "role": "assistant",
                "content": "I have processed the tool results.",
            }));
        }

        match msg {
            Message::User(user) => match &user.content {
                UserContent::Text(text) => {
                    params.push(json!({
                        "role": "user",
                        "content": sanitize_surrogates(text),
                    }));
                }
                UserContent::Blocks(blocks) => {
                    let content: Vec<Value> = blocks
                        .iter()
                        .map(|item| match item {
                            TextOrImageContent::Text(text) => json!({
                                "type": "text",
                                "text": sanitize_surrogates(&text.text),
                            }),
                            TextOrImageContent::Image(image) => json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!(
                                        "data:{};base64,{}",
                                        image.mime_type, image.data
                                    ),
                                },
                            }),
                        })
                        .collect();
                    if content.is_empty() {
                        i += 1;
                        continue;
                    }
                    params.push(json!({ "role": "user", "content": content }));
                }
            },
            Message::Assistant(assistant) => {
                // Some providers don't accept null content; use "".
                let mut assistant_msg = Map::new();
                assistant_msg.insert("role".to_string(), json!("assistant"));
                assistant_msg.insert(
                    "content".to_string(),
                    if compat.requires_assistant_after_tool_result {
                        json!("")
                    } else {
                        Value::Null
                    },
                );

                let assistant_text_parts: Vec<Value> = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AssistantContent::Text(text) if !text.text.trim().is_empty() => {
                            Some(json!({
                                "type": "text",
                                "text": sanitize_surrogates(&text.text),
                            }))
                        }
                        _ => None,
                    })
                    .collect();
                let assistant_text: String = assistant_text_parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect();

                let non_empty_thinking: Vec<&ThinkingContent> = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AssistantContent::Thinking(thinking)
                            if !thinking.thinking.trim().is_empty() =>
                        {
                            Some(thinking)
                        }
                        _ => None,
                    })
                    .collect();
                if !non_empty_thinking.is_empty() {
                    if compat.requires_thinking_as_text {
                        // Thinking as plain text (no tags, to avoid the
                        // model mimicking them).
                        let thinking_text = non_empty_thinking
                            .iter()
                            .map(|block| sanitize_surrogates(&block.thinking).to_string())
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        let mut content = vec![json!({ "type": "text", "text": thinking_text })];
                        content.extend(assistant_text_parts.iter().cloned());
                        assistant_msg.insert("content".to_string(), Value::Array(content));
                    } else {
                        // Assistant content stays a plain string (the
                        // Chat Completions standard; array form makes
                        // some models mirror the block structure).
                        if !assistant_text.is_empty() {
                            assistant_msg.insert(
                                "content".to_string(),
                                Value::String(assistant_text.clone()),
                            );
                        }
                        // First thinking block's signature (llama.cpp
                        // server + gpt-oss).
                        let mut signature = non_empty_thinking[0]
                            .thinking_signature
                            .clone()
                            .unwrap_or_default();
                        if model.provider == "opencode-go" && signature == "reasoning" {
                            signature = "reasoning_content".to_string();
                        }
                        if !signature.is_empty() {
                            let joined = non_empty_thinking
                                .iter()
                                .map(|block| block.thinking.as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            assistant_msg.insert(signature, Value::String(joined));
                        }
                    }
                } else if !assistant_text.is_empty() {
                    assistant_msg
                        .insert("content".to_string(), Value::String(assistant_text.clone()));
                }

                let tool_calls: Vec<&ToolCall> = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AssistantContent::ToolCall(tool_call) => Some(tool_call),
                        _ => None,
                    })
                    .collect();
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": Value::Object(tc.arguments.clone()).to_string(),
                                },
                            })
                        })
                        .collect();
                    assistant_msg.insert("tool_calls".to_string(), Value::Array(calls));
                    let reasoning_details: Vec<Value> = tool_calls
                        .iter()
                        .filter_map(|tc| tc.thought_signature.as_deref())
                        .filter_map(|signature| serde_json::from_str::<Value>(signature).ok())
                        .filter(|value| !value.is_null())
                        .collect();
                    if !reasoning_details.is_empty() {
                        assistant_msg.insert(
                            "reasoning_details".to_string(),
                            Value::Array(reasoning_details),
                        );
                    }
                }
                if compat.requires_reasoning_content_on_assistant_messages
                    && model.reasoning
                    && !assistant_msg.contains_key("reasoning_content")
                {
                    assistant_msg.insert("reasoning_content".to_string(), json!(""));
                }
                // Skip assistant messages with no content and no tool
                // calls (aborted responses that got no content).
                let has_content = match assistant_msg.get("content") {
                    Some(Value::String(text)) => !text.is_empty(),
                    Some(Value::Array(parts)) => !parts.is_empty(),
                    _ => false,
                };
                if !has_content && !assistant_msg.contains_key("tool_calls") {
                    i += 1;
                    continue;
                }
                params.push(Value::Object(assistant_msg));
            }
            Message::ToolResult(_) => {
                let mut image_blocks: Vec<Value> = Vec::new();
                let mut j = i;

                while j < transformed.len() {
                    let Message::ToolResult(tool_msg) = &transformed[j] else {
                        break;
                    };
                    let text_result = tool_result_text(tool_msg);
                    let has_images = tool_msg
                        .content
                        .iter()
                        .any(|block| matches!(block, TextOrImageContent::Image(_)));
                    let has_text = !text_result.is_empty();
                    let mut tool_result_msg = Map::new();
                    tool_result_msg.insert("role".to_string(), json!("tool"));
                    tool_result_msg.insert(
                        "content".to_string(),
                        json!(sanitize_surrogates(if has_text {
                            &text_result
                        } else {
                            "(see attached image)"
                        })),
                    );
                    tool_result_msg
                        .insert("tool_call_id".to_string(), json!(tool_msg.tool_call_id));
                    // Some providers require the 'name' field.
                    if compat.requires_tool_result_name && !tool_msg.tool_name.is_empty() {
                        tool_result_msg.insert("name".to_string(), json!(tool_msg.tool_name));
                    }
                    params.push(Value::Object(tool_result_msg));

                    if has_images && model.input.contains(&pi_rs_ai_types::Modality::Image) {
                        for block in &tool_msg.content {
                            if let TextOrImageContent::Image(image) = block {
                                image_blocks.push(json!({
                                    "type": "image_url",
                                    "image_url": {
                                        "url": format!(
                                            "data:{};base64,{}",
                                            image.mime_type, image.data
                                        ),
                                    },
                                }));
                            }
                        }
                    }
                    j += 1;
                }

                i = j - 1;

                if image_blocks.is_empty() {
                    last_role = Some("toolResult");
                } else {
                    if compat.requires_assistant_after_tool_result {
                        params.push(json!({
                            "role": "assistant",
                            "content": "I have processed the tool results.",
                        }));
                    }
                    let mut content = vec![
                        json!({ "type": "text", "text": "Attached image(s) from tool result:" }),
                    ];
                    content.append(&mut image_blocks);
                    params.push(json!({ "role": "user", "content": content }));
                    last_role = Some("user");
                }
                i += 1;
                continue;
            }
        }

        last_role = Some(match msg {
            Message::User(_) => "user",
            Message::Assistant(_) => "assistant",
            Message::ToolResult(_) => "toolResult",
        });
        i += 1;
    }

    params
}

fn tool_result_text(tool_msg: &ToolResultMessage) -> String {
    tool_msg
        .content
        .iter()
        .filter_map(|block| match block {
            pi_rs_ai_types::TextOrImageContent::Text(text) => Some(text.text.as_str()),
            pi_rs_ai_types::TextOrImageContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Spec: `convertTools`.
fn convert_tools(tools: &[Tool], compat: &ResolvedCompat) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                let mut function = Map::new();
                function.insert("name".to_string(), json!(tool.name));
                function.insert("description".to_string(), json!(tool.description));
                function.insert("parameters".to_string(), tool.parameters.clone());
                // Only include strict if the provider supports it; some
                // reject unknown fields.
                if compat.supports_strict_mode {
                    function.insert("strict".to_string(), json!(false));
                }
                json!({ "type": "function", "function": function })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------
// Thinking-level helpers
// ---------------------------------------------------------------------

fn level_str(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
        ThinkingLevel::Max => "max",
    }
}

/// `model.thinkingLevelMap?.[level]` — `None` = key absent (undefined),
/// `Some(None)` = explicit null, `Some(Some(_))` = mapped string.
fn map_entry(model: &Model, level: ModelThinkingLevel) -> Option<Option<String>> {
    model
        .thinking_level_map
        .as_ref()
        .and_then(|map| map.get(&level).cloned())
}

/// Spec: `model.thinkingLevelMap?.[effort] ?? effort`.
fn mapped_effort(model: &Model, level: ThinkingLevel) -> String {
    map_entry(model, ModelThinkingLevel::from(level))
        .flatten()
        .unwrap_or_else(|| level_str(level).to_string())
}

// ---------------------------------------------------------------------
// Params
// ---------------------------------------------------------------------

/// Spec: `buildParams` — the `ChatCompletionCreateParamsStreaming`
/// payload.
fn build_params(
    model: &Model,
    context: &Context,
    options: &OpenAICompletionsOptions,
    compat: &ResolvedCompat,
    cache_retention: CacheRetention,
) -> Value {
    let mut messages = convert_messages(model, context, compat);
    let cache_control = get_compat_cache_control(compat, cache_retention);

    let mut params = Map::new();
    params.insert("model".to_string(), json!(model.id));
    params.insert("stream".to_string(), json!(true));

    if ((model.base_url.contains("api.openai.com") && cache_retention != CacheRetention::None)
        || (cache_retention == CacheRetention::Long && compat.supports_long_cache_retention))
        && let Some(key) = clamp_openai_prompt_cache_key(options.base.session_id.as_deref())
    {
        params.insert("prompt_cache_key".to_string(), json!(key));
    }
    if cache_retention == CacheRetention::Long && compat.supports_long_cache_retention {
        params.insert("prompt_cache_retention".to_string(), json!("24h"));
    }

    if compat.supports_usage_in_streaming {
        params.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }

    if compat.supports_store {
        params.insert("store".to_string(), json!(false));
    }

    if let Some(max_tokens) = options.base.max_tokens {
        let field = match compat.max_tokens_field {
            MaxTokensField::MaxTokens => "max_tokens",
            MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        };
        params.insert(field.to_string(), json!(max_tokens));
    }

    if let Some(temperature) = options.base.temperature {
        params.insert("temperature".to_string(), json!(temperature));
    }

    let mut tools: Option<Value> = None;
    if let Some(context_tools) = &context.tools
        && !context_tools.is_empty()
    {
        tools = Some(convert_tools(context_tools, compat));
        if compat.zai_tool_stream {
            params.insert("tool_stream".to_string(), json!(true));
        }
    } else if has_tool_history(&context.messages) {
        // Anthropic (via LiteLLM/proxy) requires the tools param when
        // the conversation has tool calls/results.
        tools = Some(json!([]));
    }

    if let Some(cache_control) = &cache_control {
        apply_anthropic_cache_control(&mut messages, tools.as_mut(), cache_control);
    }

    if let Some(tool_choice) = &options.tool_choice {
        params.insert("tool_choice".to_string(), tool_choice.to_value());
    }

    // Thinking format branches (a literal else-if chain, as in the spec:
    // an ant-ling model without effort can still reach the off-value
    // branch when its compat enables reasoningEffort).
    let effort = options.reasoning_effort;
    let off_entry = map_entry(model, ModelThinkingLevel::Off);
    let off_is_null = matches!(off_entry, Some(None));
    if compat.thinking_format == ThinkingFormat::Zai && model.reasoning {
        params.insert(
            "thinking".to_string(),
            json!({ "type": if effort.is_some() { "enabled" } else { "disabled" } }),
        );
    } else if compat.thinking_format == ThinkingFormat::Qwen && model.reasoning {
        params.insert("enable_thinking".to_string(), json!(effort.is_some()));
    } else if compat.thinking_format == ThinkingFormat::QwenChatTemplate && model.reasoning {
        params.insert(
            "chat_template_kwargs".to_string(),
            json!({ "enable_thinking": effort.is_some(), "preserve_thinking": true }),
        );
    } else if compat.thinking_format == ThinkingFormat::Deepseek && model.reasoning {
        params.insert(
            "thinking".to_string(),
            json!({ "type": if effort.is_some() { "enabled" } else { "disabled" } }),
        );
        if let Some(level) = effort
            && compat.supports_reasoning_effort
        {
            params.insert(
                "reasoning_effort".to_string(),
                json!(mapped_effort(model, level)),
            );
        }
    } else if compat.thinking_format == ThinkingFormat::Openrouter && model.reasoning {
        // OpenRouter normalizes reasoning via a nested object.
        if let Some(level) = effort {
            params.insert(
                "reasoning".to_string(),
                json!({ "effort": mapped_effort(model, level) }),
            );
        } else if !off_is_null {
            let effort = off_entry
                .clone()
                .flatten()
                .unwrap_or_else(|| "none".to_string());
            params.insert("reasoning".to_string(), json!({ "effort": effort }));
        }
    } else if compat.thinking_format == ThinkingFormat::AntLing
        && model.reasoning
        && effort.is_some()
    {
        if let Some(level) = effort
            && let Some(Some(mapped)) = map_entry(model, ModelThinkingLevel::from(level))
        {
            params.insert("reasoning".to_string(), json!({ "effort": mapped }));
        }
    } else if compat.thinking_format == ThinkingFormat::Together && model.reasoning {
        params.insert(
            "reasoning".to_string(),
            json!({ "enabled": effort.is_some() }),
        );
        if let Some(level) = effort
            && compat.supports_reasoning_effort
        {
            params.insert(
                "reasoning_effort".to_string(),
                json!(mapped_effort(model, level)),
            );
        }
    } else if compat.thinking_format == ThinkingFormat::StringThinking && model.reasoning {
        if let Some(level) = effort {
            params.insert("thinking".to_string(), json!(mapped_effort(model, level)));
        } else if !off_is_null {
            let value = off_entry
                .clone()
                .flatten()
                .unwrap_or_else(|| "none".to_string());
            params.insert("thinking".to_string(), json!(value));
        }
    } else if let Some(level) = effort
        && model.reasoning
        && compat.supports_reasoning_effort
    {
        // OpenAI-style reasoning_effort.
        params.insert(
            "reasoning_effort".to_string(),
            json!(mapped_effort(model, level)),
        );
    } else if effort.is_none()
        && model.reasoning
        && compat.supports_reasoning_effort
        && let Some(Some(off_value)) = off_entry
    {
        params.insert("reasoning_effort".to_string(), json!(off_value));
    }

    // OpenRouter provider routing preferences (raw model.compat, as in
    // the spec's `model.compat?.openRouterRouting` truthiness check).
    if let Some(compat_value) = &model.compat
        && let Some(routing) = compat_value.get("openRouterRouting")
        && !routing.is_null()
    {
        params.insert("provider".to_string(), routing.clone());
    }

    // Vercel AI Gateway provider routing preferences.
    if model.base_url.contains("ai-gateway.vercel.sh")
        && let Some(compat_value) = &model.compat
        && let Some(routing) = compat_value.get("vercelGatewayRouting")
        && !routing.is_null()
    {
        let only = routing.get("only").filter(|value| !value.is_null());
        let order = routing.get("order").filter(|value| !value.is_null());
        if only.is_some() || order.is_some() {
            let mut gateway = Map::new();
            if let Some(only) = only {
                gateway.insert("only".to_string(), only.clone());
            }
            if let Some(order) = order {
                gateway.insert("order".to_string(), order.clone());
            }
            params.insert("providerOptions".to_string(), json!({ "gateway": gateway }));
        }
    }

    params.insert("messages".to_string(), Value::Array(messages));
    if let Some(tools) = tools {
        params.insert("tools".to_string(), tools);
    }

    Value::Object(params)
}

// ---------------------------------------------------------------------
// Usage / stop reasons
// ---------------------------------------------------------------------

/// Spec: `parseChunkUsage` — cached_tokens is cache-read; writes are a
/// separate count, never subtracted from reads.
fn parse_chunk_usage(raw_usage: &Value, model: &Model) -> Usage {
    let prompt_tokens = raw_usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let details = raw_usage.get("prompt_tokens_details");
    let cache_read = details
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| {
            raw_usage
                .get("prompt_cache_hit_tokens")
                .and_then(Value::as_u64)
        })
        .unwrap_or(0);
    let cache_write = details
        .and_then(|d| d.get("cache_write_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input = prompt_tokens.saturating_sub(cache_read + cache_write);
    // completion_tokens already includes reasoning tokens.
    let output = raw_usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut usage = Usage {
        input,
        output,
        cache_read,
        cache_write,
        total_tokens: input + output + cache_read + cache_write,
        ..Usage::default()
    };
    calculate_cost(model, &mut usage);
    usage
}

/// Spec: `mapStopReason` (`null` never reaches here — the loop only maps
/// truthy finish reasons).
fn map_stop_reason(reason: &str) -> (StopReason, Option<String>) {
    match reason {
        "stop" | "end" => (StopReason::Stop, None),
        "length" => (StopReason::Length, None),
        "function_call" | "tool_calls" => (StopReason::ToolUse, None),
        _ => (
            StopReason::Error,
            Some(format!("Provider finish_reason: {reason}")),
        ),
    }
}

// ---------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------

/// Spec: `streamOpenAICompletions` — the
/// `StreamFunction<"openai-completions">`. Every failure folds into an
/// `error` event; the returned stream never hangs. Must be called within
/// a tokio runtime.
pub fn stream_openai_completions(
    model: &Model,
    context: &Context,
    options: Option<OpenAICompletionsOptions>,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let task_stream = stream.clone();
    let model = model.clone();
    let context = context.clone();
    tokio::spawn(async move {
        run_openai_completions(&model, &context, &options.unwrap_or_default(), &task_stream).await;
    });
    stream
}

async fn run_openai_completions(
    model: &Model,
    context: &Context,
    options: &OpenAICompletionsOptions,
    stream: &AssistantMessageEventStream,
) {
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

    match drive(model, context, options, stream, &mut output).await {
        Ok(()) => {
            stream.push(AssistantMessageEvent::Done {
                reason: output.stop_reason,
                message: output,
            });
            stream.end();
        }
        Err(ProtocolError(message)) => {
            let aborted = options
                .base
                .signal
                .as_ref()
                .is_some_and(AbortSignal::is_aborted);
            output.stop_reason = if aborted {
                StopReason::Aborted
            } else {
                StopReason::Error
            };
            output.error_message = Some(message);
            stream.push(AssistantMessageEvent::Error {
                reason: output.stop_reason,
                error: output,
            });
            stream.end();
        }
    }
}

/// Streaming scratch for a tool-call block — the spec's transient
/// `partialArgs`/`streamIndex` properties, kept off the message.
struct ToolMeta {
    content_index: usize,
    stream_index: Option<u64>,
    partial_args: String,
}

/// Spec: the OpenRouter `error.error.metadata.raw` append in the catch —
/// extracted from the HTTP error body here (see module docs).
fn shape_transport_error(error: &TransportError) -> String {
    let TransportError::Status { status, body, .. } = error else {
        return error.to_string();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return error.to_string();
    };
    let mut message = parsed
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map_or_else(|| error.to_string(), |value| format!("{status} {value}"));
    if let Some(raw) = parsed
        .pointer("/error/metadata/raw")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        message.push('\n');
        message.push_str(raw);
    }
    message
}

#[allow(clippy::too_many_lines)]
async fn drive(
    model: &Model,
    context: &Context,
    options: &OpenAICompletionsOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let api_key = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;

    let compat = get_compat(model);
    let cache_retention = resolve_cache_retention(options.base.cache_retention);
    let cache_session_id = if cache_retention == CacheRetention::None {
        None
    } else {
        options.base.session_id.as_deref()
    };

    let request = create_request(
        model,
        context,
        api_key,
        options.base.headers.as_ref(),
        cache_session_id,
        &compat,
    )?;

    let mut params = build_params(model, context, options, &compat, cache_retention);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model)
    {
        params = next;
    }
    let body = params.to_string();

    let retry = RetryOptions {
        max_retries: options.base.max_retries.unwrap_or(0),
        max_retry_delay_ms: options.base.max_retry_delay_ms,
        header_timeout_ms: options
            .base
            .timeout_ms
            .unwrap_or(DEFAULT_OPENAI_COMPLETIONS_TIMEOUT_MS),
        // The openai SDK's stainless retry loop matches the anthropic
        // one; pinning it arrives with the provider-breadth milestone
        // (PLAN item 8). Codex semantics until then.
        policy: crate::transport::RetryPolicy::default(),
    };
    let signal = options.base.signal.clone();
    let client = reqwest::Client::new();
    let response = post_with_retry(
        &client,
        &request.url,
        &request.headers,
        &body,
        &retry,
        signal.as_ref(),
    )
    .await
    .map_err(|error| ProtocolError(shape_transport_error(&error)))?;

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

    let mut reader = response_sse_reader(response, signal.clone());
    let mut text_index: Option<usize> = None;
    let mut thinking_index: Option<usize> = None;
    let mut has_finish_reason = false;
    let mut tool_metas: Vec<ToolMeta> = Vec::new();

    loop {
        let Some(sse) = reader
            .next()
            .await
            .map_err(|error| ProtocolError(error.to_string()))?
        else {
            break;
        };
        // SDK: the [DONE] sentinel ends the stream; empty keep-alives
        // are skipped.
        if sse.data == "[DONE]" {
            break;
        }
        if sse.data.is_empty() {
            continue;
        }
        let chunk: Value = serde_json::from_str(&sse.data).map_err(|error| {
            ProtocolError(format!(
                "Could not parse OpenAI SSE chunk: {error}; data={}",
                sse.data
            ))
        })?;
        if !chunk.is_object() {
            continue;
        }

        // Every chunk of a streamed completion carries the same id.
        if output.response_id.as_deref().unwrap_or("").is_empty()
            && let Some(id) = chunk.get("id").and_then(Value::as_str)
        {
            output.response_id = Some(id.to_string());
        }
        if output.response_model.is_none()
            && let Some(chunk_model) = chunk.get("model").and_then(Value::as_str)
            && !chunk_model.is_empty()
            && chunk_model != model.id
        {
            output.response_model = Some(chunk_model.to_string());
        }
        let chunk_usage = chunk.get("usage").filter(|value| !value.is_null());
        if let Some(usage) = chunk_usage {
            output.usage = parse_chunk_usage(usage, model);
        }

        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            continue;
        };

        // Fallback: some providers (e.g. Moonshot) put usage in
        // choice.usage instead of the standard chunk.usage.
        if chunk_usage.is_none()
            && let Some(usage) = choice.get("usage").filter(|value| !value.is_null())
        {
            output.usage = parse_chunk_usage(usage, model);
        }

        if let Some(reason) = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty())
        {
            let (stop_reason, error_message) = map_stop_reason(reason);
            output.stop_reason = stop_reason;
            if let Some(error_message) = error_message {
                output.error_message = Some(error_message);
            }
            has_finish_reason = true;
        }

        let Some(delta) = choice.get("delta").filter(|value| value.is_object()) else {
            continue;
        };

        if let Some(content) = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
        {
            let content_index = match text_index {
                Some(index) => index,
                None => {
                    output
                        .content
                        .push(AssistantContent::Text(TextContent::new("")));
                    let index = output.content.len() - 1;
                    text_index = Some(index);
                    let partial = output.clone();
                    stream.push(AssistantMessageEvent::TextStart {
                        content_index: index,
                        partial,
                    });
                    index
                }
            };
            if let Some(AssistantContent::Text(block)) = output.content.get_mut(content_index) {
                block.text.push_str(content);
            }
            let partial = output.clone();
            stream.push(AssistantMessageEvent::TextDelta {
                content_index,
                delta: content.to_string(),
                partial,
            });
        }

        // Reasoning may arrive as reasoning_content (llama.cpp),
        // reasoning, or reasoning_text; first non-empty wins to avoid
        // duplication (chutes.ai sends two with the same content).
        let reasoning_field = ["reasoning_content", "reasoning", "reasoning_text"]
            .into_iter()
            .find(|field| {
                delta
                    .get(*field)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty())
            });
        if let Some(field) = reasoning_field
            && let Some(text) = delta.get(field).and_then(Value::as_str)
            && !text.is_empty()
        {
            let thinking_signature = if model.provider == "opencode-go" && field == "reasoning" {
                "reasoning_content"
            } else {
                field
            };
            let content_index = match thinking_index {
                Some(index) => index,
                None => {
                    output
                        .content
                        .push(AssistantContent::Thinking(ThinkingContent {
                            r#type: ThinkingType::Thinking,
                            thinking: String::new(),
                            thinking_signature: Some(thinking_signature.to_string()),
                            redacted: None,
                        }));
                    let index = output.content.len() - 1;
                    thinking_index = Some(index);
                    let partial = output.clone();
                    stream.push(AssistantMessageEvent::ThinkingStart {
                        content_index: index,
                        partial,
                    });
                    index
                }
            };
            if let Some(AssistantContent::Thinking(block)) = output.content.get_mut(content_index) {
                block.thinking.push_str(text);
            }
            let partial = output.clone();
            stream.push(AssistantMessageEvent::ThinkingDelta {
                content_index,
                delta: text.to_string(),
                partial,
            });
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let meta_pos = ensure_tool_call_block(output, stream, &mut tool_metas, tool_call);
                let content_index = tool_metas[meta_pos].content_index;
                let delta_id = tool_call.get("id").and_then(Value::as_str).unwrap_or("");
                let delta_name = tool_call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if let Some(AssistantContent::ToolCall(block)) =
                    output.content.get_mut(content_index)
                {
                    if block.id.is_empty() && !delta_id.is_empty() {
                        block.id = delta_id.to_string();
                    }
                    if block.name.is_empty() && !delta_name.is_empty() {
                        block.name = delta_name.to_string();
                    }
                }
                let mut delta_args = "";
                if let Some(arguments) = tool_call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .filter(|arguments| !arguments.is_empty())
                {
                    delta_args = arguments;
                    let meta = &mut tool_metas[meta_pos];
                    meta.partial_args.push_str(arguments);
                    let parsed = arguments_from(parse_streaming_json(&meta.partial_args));
                    if let Some(AssistantContent::ToolCall(block)) =
                        output.content.get_mut(content_index)
                    {
                        block.arguments = parsed;
                    }
                }
                let partial = output.clone();
                stream.push(AssistantMessageEvent::ToolCallDelta {
                    content_index,
                    delta: delta_args.to_string(),
                    partial,
                });
            }
        }

        // OpenRouter encrypted reasoning details attach to tool calls.
        if let Some(details) = delta.get("reasoning_details").and_then(Value::as_array) {
            for detail in details {
                let is_encrypted =
                    detail.get("type").and_then(Value::as_str) == Some("reasoning.encrypted");
                let id = detail.get("id").and_then(Value::as_str).unwrap_or("");
                let has_data = detail
                    .get("data")
                    .and_then(Value::as_str)
                    .is_some_and(|data| !data.is_empty());
                if !is_encrypted || id.is_empty() || !has_data {
                    continue;
                }
                if let Some(AssistantContent::ToolCall(block)) = output
                    .content
                    .iter_mut()
                    .find(|block| matches!(block, AssistantContent::ToolCall(tc) if tc.id == id))
                {
                    block.thought_signature = Some(detail.to_string());
                }
            }
        }
    }

    // Finish blocks in content order.
    for content_index in 0..output.content.len() {
        match &output.content[content_index] {
            AssistantContent::Text(block) => {
                let content = block.text.clone();
                let partial = output.clone();
                stream.push(AssistantMessageEvent::TextEnd {
                    content_index,
                    content,
                    partial,
                });
            }
            AssistantContent::Thinking(block) => {
                let content = block.thinking.clone();
                let partial = output.clone();
                stream.push(AssistantMessageEvent::ThinkingEnd {
                    content_index,
                    content,
                    partial,
                });
            }
            AssistantContent::ToolCall(_) => {
                let partial_args = tool_metas
                    .iter()
                    .find(|meta| meta.content_index == content_index)
                    .map(|meta| meta.partial_args.clone())
                    .unwrap_or_default();
                if let Some(AssistantContent::ToolCall(block)) =
                    output.content.get_mut(content_index)
                {
                    // Finalize; the scratch buffer never persists.
                    block.arguments = arguments_from(parse_streaming_json(&partial_args));
                    let tool_call = block.clone();
                    let partial = output.clone();
                    stream.push(AssistantMessageEvent::ToolCallEnd {
                        content_index,
                        tool_call,
                        partial,
                    });
                }
            }
        }
    }

    if signal.as_ref().is_some_and(AbortSignal::is_aborted) {
        return Err(ProtocolError("Request was aborted".to_string()));
    }
    if output.stop_reason == StopReason::Aborted {
        return Err(ProtocolError("Request was aborted".to_string()));
    }
    if output.stop_reason == StopReason::Error {
        return Err(ProtocolError(output.error_message.clone().unwrap_or_else(
            || "Provider returned an error stop reason".to_string(),
        )));
    }
    if !has_finish_reason {
        return Err(ProtocolError(
            "Stream ended without finish_reason".to_string(),
        ));
    }
    Ok(())
}

/// Spec: `ensureToolCallBlock` — resolve by stream index, then by id;
/// create on miss. Returns the meta position.
fn ensure_tool_call_block(
    output: &mut AssistantMessage,
    stream: &AssistantMessageEventStream,
    tool_metas: &mut Vec<ToolMeta>,
    tool_call: &Value,
) -> usize {
    let stream_index = tool_call.get("index").and_then(Value::as_u64);
    let delta_id = tool_call.get("id").and_then(Value::as_str).unwrap_or("");

    let mut found = stream_index.and_then(|index| {
        tool_metas
            .iter()
            .position(|meta| meta.stream_index == Some(index))
    });
    if found.is_none() && !delta_id.is_empty() {
        found = tool_metas.iter().position(|meta| {
            matches!(
                output.content.get(meta.content_index),
                Some(AssistantContent::ToolCall(tc)) if tc.id == delta_id
            )
        });
    }

    let meta_pos = match found {
        Some(pos) => pos,
        None => {
            output.content.push(AssistantContent::ToolCall(ToolCall {
                r#type: ToolCallType::ToolCall,
                id: delta_id.to_string(),
                name: tool_call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                arguments: Map::new(),
                thought_signature: None,
            }));
            let content_index = output.content.len() - 1;
            tool_metas.push(ToolMeta {
                content_index,
                stream_index,
                partial_args: String::new(),
            });
            let partial = output.clone();
            stream.push(AssistantMessageEvent::ToolCallStart {
                content_index,
                partial,
            });
            tool_metas.len() - 1
        }
    };
    if let Some(index) = stream_index
        && tool_metas[meta_pos].stream_index.is_none()
    {
        tool_metas[meta_pos].stream_index = Some(index);
    }
    meta_pos
}

fn arguments_from(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

// ---------------------------------------------------------------------
// Simple options entry point
// ---------------------------------------------------------------------

/// Spec: `streamSimpleOpenAICompletions` — clamps the reasoning level to
/// the model's supported levels ("off" → no effort). The spec throws
/// synchronously when no API key is provided; here that is the `Err`.
pub fn stream_simple_openai_completions(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let api_key = options
        .as_ref()
        .and_then(|o| o.base.api_key.as_deref())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?
        .to_string();

    let base = build_base_options(model, options.as_ref(), Some(&api_key));
    let reasoning_effort = options
        .as_ref()
        .and_then(|o| o.reasoning)
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

    Ok(stream_openai_completions(
        model,
        context,
        Some(OpenAICompletionsOptions {
            base,
            tool_choice: None,
            reasoning_effort,
        }),
    ))
}
