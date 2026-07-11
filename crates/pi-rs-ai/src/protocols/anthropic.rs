//! Port of `providers/anthropic.ts` — the anthropic-messages protocol.
//!
//! The spec drives the Anthropic SDK; here the SDK's contribution is
//! reproduced explicitly and pinned by the differential oracle in
//! `tests/anthropic-parity/` (Pi's real provider run against a scripted
//! stub; replayed by `tests/anthropic_parity.rs`):
//!
//! - `client.messages.create(..).asResponse()` → [`crate::transport`]
//!   `post_with_retry` against `{baseUrl}/v1/messages` with the
//!   `anthropic-version` header the SDK injects;
//! - `maxRetries`/`timeout` request options → [`RetryOptions`] with
//!   [`RetryPolicy::AnthropicSdk`] (the SDK's retry loop; its default
//!   timeout is 10 minutes, [`DEFAULT_ANTHROPIC_TIMEOUT_MS`]);
//! - SDK error shaping: `APIError.makeMessage` ([`sdk_status_error_message`]),
//!   `APIUserAbortError` ("Request was aborted."), and undici's abort
//!   `DOMException` for reads cancelled mid-stream
//!   (`TransportError::BodyAborted`);
//! - the spec's hand-rolled `iterateSseMessages` (its own line-based SSE
//!   decoder) → the transport [`SseReader`]; the anthropic-specific
//!   filtering (`error` events, `ANTHROPIC_MESSAGE_EVENTS`, repair-parse,
//!   message_start/stop bookkeeping) stays here, as in the spec.
//!
//! Divergences (mechanism only):
//! - the SSE-parse failure message reuses the event data for the spec's
//!   `raw=` segment (the transport reader does not retain raw lines);
//! - `options.client` (pre-built SDK client injection, used for Vertex)
//!   has no analogue yet — it arrives with the google-vertex protocol;
//! - `effort` is an open string (the spec casts through its
//!   `AnthropicEffort` union from catalog data, so any string passes).

use std::collections::BTreeMap;

use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, CacheRetention,
    Context, Message, Model, ModelThinkingLevel, ProviderResponse, StopReason, TextContent,
    ThinkingContent, ThinkingLevel, ThinkingType, Tool, ToolCall, ToolCallType, ToolResultMessage,
    Usage, UserContent, calculate_cost, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::cloudflare::resolve_cloudflare_base_url;
use super::copilot_headers::{build_copilot_dynamic_headers, has_copilot_vision_input};
use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::{adjust_max_tokens_for_thinking, build_base_options};
use super::transform_messages::transform_messages;
use super::{ProtocolError, merge_header, merge_header_map, resolve_cache_retention};
use crate::transport::{
    AbortSignal, AssistantMessageEventStream, RetryOptions, RetryPolicy,
    create_assistant_message_event_stream, post_with_retry, response_sse_reader,
};
use crate::util::{
    headers_to_record, parse_json_with_repair, parse_streaming_json, sanitize_surrogates,
};

/// Spec: SDK request-option `timeout` default (Anthropic SDK: 10 min).
pub const DEFAULT_ANTHROPIC_TIMEOUT_MS: u64 = 600_000;

/// The `anthropic-version` header the SDK injects on every request.
const ANTHROPIC_VERSION: &str = "2023-06-01";

const FINE_GRAINED_TOOL_STREAMING_BETA: &str = "fine-grained-tool-streaming-2025-05-14";
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";

/// Spec: stealth mode — mimic Claude Code's tool naming exactly.
const CLAUDE_CODE_VERSION: &str = "2.1.75";

/// Claude Code 2.x tool names (canonical casing).
const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

const ANTHROPIC_MESSAGE_EVENTS: &[&str] = &[
    "message_start",
    "message_delta",
    "message_stop",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
];

/// Spec: `AnthropicThinkingDisplay`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnthropicThinkingDisplay {
    #[default]
    Summarized,
    Omitted,
}

impl AnthropicThinkingDisplay {
    fn as_str(self) -> &'static str {
        match self {
            Self::Summarized => "summarized",
            Self::Omitted => "omitted",
        }
    }
}

/// Spec: `AnthropicOptions["toolChoice"]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnthropicToolChoice {
    Auto,
    Any,
    None,
    Tool { name: String },
}

impl AnthropicToolChoice {
    fn to_value(&self) -> Value {
        match self {
            Self::Auto => json!({ "type": "auto" }),
            Self::Any => json!({ "type": "any" }),
            Self::None => json!({ "type": "none" }),
            Self::Tool { name } => json!({ "type": "tool", "name": name }),
        }
    }
}

/// Spec: `AnthropicOptions` (`StreamOptions` + anthropic knobs).
#[derive(Clone, Default)]
pub struct AnthropicOptions {
    pub base: StreamOptions,
    /// Enable extended thinking.
    pub thinking_enabled: Option<bool>,
    /// Token budget for extended thinking (older models only; default
    /// 1024 when thinking is enabled without a budget).
    pub thinking_budget_tokens: Option<u64>,
    /// Effort level for adaptive-thinking models
    /// ("low"|"medium"|"high"|"xhigh"|"max"; open string, see module docs).
    pub effort: Option<String>,
    /// How thinking content is returned (default "summarized").
    pub thinking_display: Option<AnthropicThinkingDisplay>,
    /// Request the interleaved-thinking beta for non-adaptive models
    /// (default true; adaptive models skip the header regardless).
    pub interleaved_thinking: Option<bool>,
    pub tool_choice: Option<AnthropicToolChoice>,
}

/// Spec: `getCacheControl` — the `cache_control` block, with a 1h TTL for
/// long retention on models that support it.
fn get_cache_control(
    compat: &ResolvedCompat,
    cache_retention: Option<CacheRetention>,
) -> (CacheRetention, Option<Value>) {
    let retention = resolve_cache_retention(cache_retention);
    if retention == CacheRetention::None {
        return (retention, None);
    }
    let mut cache_control = json!({ "type": "ephemeral" });
    if retention == CacheRetention::Long && compat.supports_long_cache_retention {
        cache_control["ttl"] = json!("1h");
    }
    (retention, Some(cache_control))
}

/// Spec: `getAnthropicCompat` — compat with provider-based auto-detection
/// (fireworks, cloudflare-ai-gateway → anthropic passthrough).
struct ResolvedCompat {
    supports_eager_tool_input_streaming: bool,
    supports_long_cache_retention: bool,
    send_session_affinity_headers: bool,
    supports_cache_control_on_tools: bool,
    supports_temperature: bool,
    allow_empty_signature: bool,
    /// Spec: `model.compat?.forceAdaptiveThinking === true` (not part of
    /// the `Required<Omit<…>>` result, read alongside it everywhere).
    force_adaptive_thinking: bool,
}

fn get_anthropic_compat(model: &Model) -> ResolvedCompat {
    let compat: pi_rs_ai_types::AnthropicMessagesCompat =
        model.compat().ok().flatten().unwrap_or_default();
    let is_fireworks = model.provider == "fireworks";
    let is_cloudflare_ai_gateway_anthropic =
        model.provider == "cloudflare-ai-gateway" && model.base_url.contains("anthropic");
    ResolvedCompat {
        supports_eager_tool_input_streaming: compat
            .supports_eager_tool_input_streaming
            .unwrap_or(!is_fireworks),
        supports_long_cache_retention: compat
            .supports_long_cache_retention
            .unwrap_or(!is_fireworks),
        send_session_affinity_headers: compat
            .send_session_affinity_headers
            .unwrap_or(is_fireworks || is_cloudflare_ai_gateway_anthropic),
        supports_cache_control_on_tools: compat
            .supports_cache_control_on_tools
            .unwrap_or(!is_fireworks),
        supports_temperature: compat.supports_temperature.unwrap_or(true),
        allow_empty_signature: compat.allow_empty_signature.unwrap_or(false),
        force_adaptive_thinking: compat.force_adaptive_thinking == Some(true),
    }
}

/// Spec: `toClaudeCodeName` — CC canonical casing when it matches
/// (case-insensitive).
fn to_claude_code_name(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|tool| tool.eq_ignore_ascii_case(name))
        .map_or_else(|| name.to_string(), |tool| (*tool).to_string())
}

/// Spec: `fromClaudeCodeName` — map back to the registered tool's name.
fn from_claude_code_name(name: &str, tools: Option<&[Tool]>) -> String {
    if let Some(tools) = tools
        && !tools.is_empty()
        && let Some(tool) = tools
            .iter()
            .find(|tool| tool.name.eq_ignore_ascii_case(name))
    {
        return tool.name.clone();
    }
    name.to_string()
}

/// Spec: `isOAuthToken`.
fn is_oauth_token(api_key: &str) -> bool {
    api_key.contains("sk-ant-oat")
}

/// Spec: `normalizeToolCallId` — Anthropic requires `^[a-zA-Z0-9_-]+$`,
/// max 64 chars.
fn normalize_tool_call_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

/// Spec: `convertContentBlocks` — tool-result content: text-only joins to
/// a string; with images, a block array (placeholder text when there is
/// no text at all).
fn convert_content_blocks(content: &[pi_rs_ai_types::TextOrImageContent]) -> Value {
    use pi_rs_ai_types::TextOrImageContent;

    let has_images = content
        .iter()
        .any(|block| matches!(block, TextOrImageContent::Image(_)));
    if !has_images {
        let joined = content
            .iter()
            .map(|block| match block {
                TextOrImageContent::Text(text) => text.text.as_str(),
                TextOrImageContent::Image(_) => "",
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Value::String(sanitize_surrogates(&joined).to_string());
    }

    let mut blocks: Vec<Value> = content
        .iter()
        .map(|block| match block {
            TextOrImageContent::Text(text) => {
                json!({ "type": "text", "text": sanitize_surrogates(&text.text) })
            }
            TextOrImageContent::Image(image) => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.mime_type,
                    "data": image.data,
                },
            }),
        })
        .collect();

    let has_text = blocks
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("text"));
    if !has_text {
        blocks.insert(0, json!({ "type": "text", "text": "(see attached image)" }));
    }

    Value::Array(blocks)
}

/// Spec: `shouldUseFineGrainedToolStreamingBeta`.
fn should_use_fine_grained_tool_streaming_beta(model: &Model, context: &Context) -> bool {
    context
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
        && !get_anthropic_compat(model).supports_eager_tool_input_streaming
}

/// Spec: `mapStopReason` — unknown reasons throw (API may add values).
fn map_stop_reason(reason: &str) -> Result<StopReason, ProtocolError> {
    match reason {
        "end_turn" => Ok(StopReason::Stop),
        "max_tokens" => Ok(StopReason::Length),
        "tool_use" => Ok(StopReason::ToolUse),
        "refusal" => Ok(StopReason::Error),
        // Stop is good enough -> resubmit.
        "pause_turn" => Ok(StopReason::Stop),
        // We don't supply stop sequences, so this should never happen.
        "stop_sequence" => Ok(StopReason::Stop),
        // Content flagged by safety filters.
        "sensitive" => Ok(StopReason::Error),
        _ => Err(ProtocolError(format!("Unhandled stop reason: {reason}"))),
    }
}

/// The spec's `createClient` — URL, auth headers and beta features per
/// provider branch (SDK defaults reproduced, see module docs).
struct PreparedRequest {
    url: String,
    headers: HeaderMap,
    is_oauth_token: bool,
}

#[allow(clippy::too_many_arguments)]
fn create_request(
    model: &Model,
    api_key: &str,
    interleaved_thinking: bool,
    use_fine_grained_tool_streaming_beta: bool,
    options_headers: Option<&BTreeMap<String, String>>,
    dynamic_headers: &[(String, String)],
    session_id: Option<&str>,
) -> Result<PreparedRequest, ProtocolError> {
    let compat = get_anthropic_compat(model);
    // Adaptive thinking models have interleaved thinking built in.
    let needs_interleaved_beta = interleaved_thinking && !compat.force_adaptive_thinking;
    let mut beta_features: Vec<&str> = Vec::new();
    if use_fine_grained_tool_streaming_beta {
        beta_features.push(FINE_GRAINED_TOOL_STREAMING_BETA);
    }
    if needs_interleaved_beta {
        beta_features.push(INTERLEAVED_THINKING_BETA);
    }

    let mut headers: Vec<(String, String)> = vec![
        ("content-type".to_string(), "application/json".to_string()),
        (
            "anthropic-version".to_string(),
            ANTHROPIC_VERSION.to_string(),
        ),
        ("accept".to_string(), "application/json".to_string()),
        (
            "anthropic-dangerous-direct-browser-access".to_string(),
            "true".to_string(),
        ),
    ];
    let mut base_url = model.base_url.clone();
    let mut oauth = false;

    if model.provider == "cloudflare-ai-gateway" {
        base_url = resolve_cloudflare_base_url(model)?;
        merge_header(
            &mut headers,
            "cf-aig-authorization",
            &format!("Bearer {api_key}"),
        );
        if !beta_features.is_empty() {
            merge_header(&mut headers, "anthropic-beta", &beta_features.join(","));
        }
        merge_header_map(&mut headers, model.headers.as_ref());
        merge_header_map(&mut headers, options_headers);
    } else if model.provider == "github-copilot" {
        // Copilot: Bearer auth, selective betas.
        merge_header(&mut headers, "authorization", &format!("Bearer {api_key}"));
        if !beta_features.is_empty() {
            merge_header(&mut headers, "anthropic-beta", &beta_features.join(","));
        }
        merge_header_map(&mut headers, model.headers.as_ref());
        for (key, value) in dynamic_headers {
            merge_header(&mut headers, key, value);
        }
        merge_header_map(&mut headers, options_headers);
    } else if is_oauth_token(api_key) {
        // OAuth: Bearer auth, Claude Code identity headers.
        oauth = true;
        merge_header(&mut headers, "authorization", &format!("Bearer {api_key}"));
        let betas: Vec<&str> = ["claude-code-20250219", "oauth-2025-04-20"]
            .into_iter()
            .chain(beta_features.iter().copied())
            .collect();
        merge_header(&mut headers, "anthropic-beta", &betas.join(","));
        merge_header(
            &mut headers,
            "user-agent",
            &format!("claude-cli/{CLAUDE_CODE_VERSION}"),
        );
        merge_header(&mut headers, "x-app", "cli");
        merge_header_map(&mut headers, model.headers.as_ref());
        merge_header_map(&mut headers, options_headers);
    } else {
        // API key auth.
        merge_header(&mut headers, "x-api-key", api_key);
        if !beta_features.is_empty() {
            merge_header(&mut headers, "anthropic-beta", &beta_features.join(","));
        }
        if let Some(session_id) = session_id
            && compat.send_session_affinity_headers
        {
            merge_header(&mut headers, "x-session-affinity", session_id);
        }
        merge_header_map(&mut headers, model.headers.as_ref());
        merge_header_map(&mut headers, options_headers);
    }

    let mut header_map = HeaderMap::new();
    for (key, value) in &headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|error| ProtocolError(error.to_string()))?;
        let value =
            HeaderValue::from_str(value).map_err(|error| ProtocolError(error.to_string()))?;
        header_map.insert(name, value);
    }

    Ok(PreparedRequest {
        url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        headers: header_map,
        is_oauth_token: oauth,
    })
}

/// Spec: `buildParams` — the `MessageCreateParamsStreaming` payload.
fn build_params(
    model: &Model,
    context: &Context,
    is_oauth: bool,
    options: &AnthropicOptions,
) -> Value {
    let compat = get_anthropic_compat(model);
    let (_retention, cache_control) = get_cache_control(&compat, options.base.cache_retention);

    let mut params = Map::new();
    params.insert("model".to_string(), json!(model.id));
    params.insert(
        "messages".to_string(),
        Value::Array(convert_messages(
            &context.messages,
            model,
            is_oauth,
            cache_control.as_ref(),
            compat.allow_empty_signature,
        )),
    );
    params.insert(
        "max_tokens".to_string(),
        json!(options.base.max_tokens.unwrap_or(model.max_tokens)),
    );
    params.insert("stream".to_string(), json!(true));

    let system_prompt = context
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.is_empty());
    if is_oauth {
        // For OAuth tokens, we MUST include Claude Code identity.
        let mut system = Vec::new();
        let mut identity = json!({
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude.",
        });
        if let Some(cache_control) = &cache_control {
            identity["cache_control"] = cache_control.clone();
        }
        system.push(identity);
        if let Some(prompt) = system_prompt {
            let mut block = json!({ "type": "text", "text": sanitize_surrogates(prompt) });
            if let Some(cache_control) = &cache_control {
                block["cache_control"] = cache_control.clone();
            }
            system.push(block);
        }
        params.insert("system".to_string(), Value::Array(system));
    } else if let Some(prompt) = system_prompt {
        let mut block = json!({ "type": "text", "text": sanitize_surrogates(prompt) });
        if let Some(cache_control) = &cache_control {
            block["cache_control"] = cache_control.clone();
        }
        params.insert("system".to_string(), Value::Array(vec![block]));
    }

    // Temperature is incompatible with extended thinking and unsupported
    // on some models.
    if let Some(temperature) = options.base.temperature
        && !options.thinking_enabled.unwrap_or(false)
        && compat.supports_temperature
    {
        params.insert("temperature".to_string(), json!(temperature));
    }

    if let Some(tools) = &context.tools
        && !tools.is_empty()
    {
        params.insert(
            "tools".to_string(),
            convert_tools(
                tools,
                is_oauth,
                compat.supports_eager_tool_input_streaming,
                if compat.supports_cache_control_on_tools {
                    cache_control.as_ref()
                } else {
                    None
                },
            ),
        );
    }

    // Thinking mode: adaptive, budget-based, or explicitly disabled.
    if model.reasoning {
        match options.thinking_enabled {
            Some(true) => {
                let display = options.thinking_display.unwrap_or_default().as_str();
                if compat.force_adaptive_thinking {
                    // Adaptive thinking: Claude decides when/how much.
                    params.insert(
                        "thinking".to_string(),
                        json!({ "type": "adaptive", "display": display }),
                    );
                    if let Some(effort) = &options.effort {
                        params.insert("output_config".to_string(), json!({ "effort": effort }));
                    }
                } else {
                    // Budget-based thinking for older models (spec:
                    // `thinkingBudgetTokens || 1024`, so 0 → 1024).
                    let budget = match options.thinking_budget_tokens {
                        Some(budget) if budget > 0 => budget,
                        _ => 1024,
                    };
                    params.insert(
                        "thinking".to_string(),
                        json!({ "type": "enabled", "budget_tokens": budget, "display": display }),
                    );
                }
            }
            Some(false) => {
                params.insert("thinking".to_string(), json!({ "type": "disabled" }));
            }
            None => {}
        }
    }

    if let Some(metadata) = &options.base.metadata
        && let Some(Value::String(user_id)) = metadata.get("user_id")
    {
        params.insert("metadata".to_string(), json!({ "user_id": user_id }));
    }

    if let Some(tool_choice) = &options.tool_choice {
        params.insert("tool_choice".to_string(), tool_choice.to_value());
    }

    Value::Object(params)
}

/// Spec: `convertMessages`.
fn convert_messages(
    messages: &[Message],
    model: &Model,
    is_oauth_token: bool,
    cache_control: Option<&Value>,
    allow_empty_signature: bool,
) -> Vec<Value> {
    use pi_rs_ai_types::TextOrImageContent;

    let normalize =
        |id: &str, _model: &Model, _source: &AssistantMessage| normalize_tool_call_id(id);
    let transformed = transform_messages(messages, model, Some(&normalize));

    let mut params: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < transformed.len() {
        match &transformed[i] {
            Message::User(user) => match &user.content {
                UserContent::Text(text) => {
                    if !text.trim().is_empty() {
                        params.push(json!({
                            "role": "user",
                            "content": sanitize_surrogates(text),
                        }));
                    }
                }
                UserContent::Blocks(blocks) => {
                    let filtered: Vec<Value> = blocks
                        .iter()
                        .filter_map(|item| match item {
                            TextOrImageContent::Text(text) => {
                                if text.text.trim().is_empty() {
                                    None
                                } else {
                                    Some(json!({
                                        "type": "text",
                                        "text": sanitize_surrogates(&text.text),
                                    }))
                                }
                            }
                            TextOrImageContent::Image(image) => Some(json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": image.mime_type,
                                    "data": image.data,
                                },
                            })),
                        })
                        .collect();
                    if !filtered.is_empty() {
                        params.push(json!({ "role": "user", "content": filtered }));
                    }
                }
            },
            Message::Assistant(assistant) => {
                let mut blocks: Vec<Value> = Vec::new();
                for block in &assistant.content {
                    match block {
                        AssistantContent::Text(text) => {
                            if text.text.trim().is_empty() {
                                continue;
                            }
                            blocks.push(json!({
                                "type": "text",
                                "text": sanitize_surrogates(&text.text),
                            }));
                        }
                        AssistantContent::Thinking(thinking) => {
                            // Redacted thinking: pass the opaque payload back.
                            if thinking.redacted.unwrap_or(false) {
                                blocks.push(json!({
                                    "type": "redacted_thinking",
                                    "data": thinking.thinking_signature.clone().unwrap_or_default(),
                                }));
                                continue;
                            }
                            if thinking.thinking.trim().is_empty() {
                                continue;
                            }
                            // Missing/empty signature (e.g. aborted stream):
                            // plain text, unless the model accepts empty
                            // signatures.
                            let signature = thinking.thinking_signature.as_deref().unwrap_or("");
                            if signature.trim().is_empty() {
                                if allow_empty_signature {
                                    blocks.push(json!({
                                        "type": "thinking",
                                        "thinking": sanitize_surrogates(&thinking.thinking),
                                        "signature": "",
                                    }));
                                } else {
                                    blocks.push(json!({
                                        "type": "text",
                                        "text": sanitize_surrogates(&thinking.thinking),
                                    }));
                                }
                            } else {
                                blocks.push(json!({
                                    "type": "thinking",
                                    "thinking": sanitize_surrogates(&thinking.thinking),
                                    "signature": signature,
                                }));
                            }
                        }
                        AssistantContent::ToolCall(tool_call) => {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": tool_call.id,
                                "name": if is_oauth_token {
                                    to_claude_code_name(&tool_call.name)
                                } else {
                                    tool_call.name.clone()
                                },
                                "input": tool_call.arguments,
                            }));
                        }
                    }
                }
                if !blocks.is_empty() {
                    params.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            Message::ToolResult(tool_result) => {
                // Collect all consecutive toolResult messages (needed for
                // z.ai's Anthropic endpoint).
                let mut tool_results = vec![tool_result_block(tool_result)];
                let mut j = i + 1;
                while j < transformed.len() {
                    let Message::ToolResult(next) = &transformed[j] else {
                        break;
                    };
                    tool_results.push(tool_result_block(next));
                    j += 1;
                }
                i = j - 1;
                params.push(json!({ "role": "user", "content": tool_results }));
            }
        }
        i += 1;
    }

    // Cache the conversation history: cache_control on the last user
    // message.
    if let Some(cache_control) = cache_control
        && let Some(last) = params.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
    {
        match last.get_mut("content") {
            Some(Value::Array(blocks)) => {
                if let Some(last_block) = blocks.last_mut()
                    && matches!(
                        last_block.get("type").and_then(Value::as_str),
                        Some("text" | "image" | "tool_result")
                    )
                {
                    last_block["cache_control"] = cache_control.clone();
                }
            }
            Some(content @ Value::String(_)) => {
                let text = content.clone();
                *content = json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": cache_control,
                }]);
            }
            _ => {}
        }
    }

    params
}

fn tool_result_block(message: &ToolResultMessage) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": message.tool_call_id,
        "content": convert_content_blocks(&message.content),
        "is_error": message.is_error,
    })
}

/// Spec: `convertTools`.
fn convert_tools(
    tools: &[Tool],
    is_oauth_token: bool,
    supports_eager_tool_input_streaming: bool,
    cache_control: Option<&Value>,
) -> Value {
    Value::Array(
        tools
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                let schema = tool.parameters.as_object();
                let properties = schema
                    .and_then(|s| s.get("properties"))
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let required = schema
                    .and_then(|s| s.get("required"))
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let mut value = json!({
                    "name": if is_oauth_token {
                        to_claude_code_name(&tool.name)
                    } else {
                        tool.name.clone()
                    },
                    "description": tool.description,
                    "input_schema": {
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    },
                });
                if supports_eager_tool_input_streaming {
                    value["eager_input_streaming"] = json!(true);
                }
                if let Some(cache_control) = cache_control
                    && index == tools.len() - 1
                {
                    value["cache_control"] = cache_control.clone();
                }
                value
            })
            .collect(),
    )
}

/// Spec: `streamAnthropic` — the `StreamFunction<"anthropic-messages">`.
/// Every failure folds into an `error` event; the returned stream never
/// hangs. Must be called within a tokio runtime.
pub fn stream_anthropic(
    model: &Model,
    context: &Context,
    options: Option<AnthropicOptions>,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let task_stream = stream.clone();
    let model = model.clone();
    let context = context.clone();
    tokio::spawn(async move {
        run_anthropic(&model, &context, &options.unwrap_or_default(), &task_stream).await;
    });
    stream
}

async fn run_anthropic(
    model: &Model,
    context: &Context,
    options: &AnthropicOptions,
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

/// Bookkeeping for streaming content blocks — the spec's transient
/// `index`/`partialJson` properties, kept off the message.
struct BlockMeta {
    index: u64,
    content_index: usize,
    partial_json: String,
}

fn arguments_from(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// SDK spec (`@anthropic-ai/sdk` 0.91.1 `APIError.makeMessage` with
/// `errJSON`/`errText` from the client): a truthy `errJSON.message`
/// wins (stringified when not a string), then the stringified JSON
/// body, then the raw body text; an empty message renders the
/// "status code (no body)" form. Pinned by the anthropic-parity
/// oracle's `http-*`/`retry-*` cases.
fn sdk_status_error_message(status: u16, body: &str) -> String {
    let msg = match serde_json::from_str::<Value>(body).ok() {
        Some(json) => {
            let message = json.get("message").cloned().unwrap_or(Value::Null);
            let message_truthy = match &message {
                Value::String(text) => !text.is_empty(),
                Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
                Value::Bool(flag) => *flag,
                Value::Null => false,
                _ => true,
            };
            if message_truthy {
                match message {
                    Value::String(text) => text,
                    other => other.to_string(),
                }
            } else {
                // Falsy whole-body JSON (`null`, `0`, `false`, `""`)
                // falls back to the raw text in the SDK; its compact
                // stringification coincides with that text.
                json.to_string()
            }
        }
        None => body.to_string(),
    };
    if msg.is_empty() {
        format!("{status} status code (no body)")
    } else {
        format!("{status} {msg}")
    }
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &AnthropicOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let api_key = options
        .base
        .api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ProtocolError(format!("No API key for provider: {}", model.provider)))?;

    let dynamic_headers = if model.provider == "github-copilot" {
        let has_images = has_copilot_vision_input(&context.messages);
        build_copilot_dynamic_headers(&context.messages, has_images)
    } else {
        Vec::new()
    };

    let cache_retention = resolve_cache_retention(options.base.cache_retention);
    let cache_session_id = if cache_retention == CacheRetention::None {
        None
    } else {
        options.base.session_id.as_deref()
    };

    let request = create_request(
        model,
        api_key,
        options.interleaved_thinking.unwrap_or(true),
        should_use_fine_grained_tool_streaming_beta(model, context),
        options.base.headers.as_ref(),
        &dynamic_headers,
        cache_session_id,
    )?;
    let is_oauth = request.is_oauth_token;

    let mut params = build_params(model, context, is_oauth, options);
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model)
    {
        params = next;
    }
    let body = params.to_string();

    let retry = RetryOptions {
        max_retries: options.base.max_retries.unwrap_or(0),
        // The spec passes only `signal`/`timeout`/`maxRetries` to the
        // SDK; `maxRetryDelayMs` has no SDK analogue.
        max_retry_delay_ms: None,
        header_timeout_ms: options
            .base
            .timeout_ms
            .unwrap_or(DEFAULT_ANTHROPIC_TIMEOUT_MS),
        policy: RetryPolicy::AnthropicSdk,
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
    .map_err(|error| match error {
        // SDK: `APIError.makeMessage(status, errJSON, errText)`.
        crate::transport::TransportError::Status { status, body, .. } => {
            ProtocolError(sdk_status_error_message(status, &body))
        }
        // SDK: `APIUserAbortError` ("Request was aborted.").
        crate::transport::TransportError::Aborted => {
            ProtocolError("Request was aborted.".to_string())
        }
        other => ProtocolError(other.to_string()),
    })?;

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
    let mut metas: Vec<BlockMeta> = Vec::new();
    let mut saw_message_start = false;
    let mut saw_message_stop = false;

    loop {
        let Some(sse) = reader
            .next()
            .await
            .map_err(|error| ProtocolError(error.to_string()))?
        else {
            break;
        };
        if sse.event.as_deref() == Some("error") {
            return Err(ProtocolError(sse.data));
        }
        let event_name = sse.event.clone().unwrap_or_default();
        if !ANTHROPIC_MESSAGE_EVENTS.contains(&event_name.as_str()) {
            continue;
        }
        let event = parse_json_with_repair(&sse.data).map_err(|error| {
            ProtocolError(format!(
                "Could not parse Anthropic SSE event {event_name}: {error}; data={}; raw={}",
                sse.data, sse.data
            ))
        })?;

        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "message_start" => {
                saw_message_start = true;
                let message = event.get("message").cloned().unwrap_or(Value::Null);
                if let Some(id) = message.get("id").and_then(Value::as_str) {
                    output.response_id = Some(id.to_string());
                }
                // Capture initial usage so input counts survive an early
                // abort. Anthropic has no total_tokens; compute it.
                let usage = message.get("usage").cloned().unwrap_or(Value::Null);
                output.usage.input = u64_or_zero(usage.get("input_tokens"));
                output.usage.output = u64_or_zero(usage.get("output_tokens"));
                output.usage.cache_read = u64_or_zero(usage.get("cache_read_input_tokens"));
                output.usage.cache_write = u64_or_zero(usage.get("cache_creation_input_tokens"));
                finalize_usage(model, output);
            }
            "content_block_start" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
                let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                match block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "text" => {
                        output
                            .content
                            .push(AssistantContent::Text(TextContent::new("")));
                        metas.push(BlockMeta {
                            index,
                            content_index: output.content.len() - 1,
                            partial_json: String::new(),
                        });
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::TextStart {
                            content_index: partial.content.len() - 1,
                            partial,
                        });
                    }
                    "thinking" => {
                        output
                            .content
                            .push(AssistantContent::Thinking(ThinkingContent {
                                r#type: ThinkingType::Thinking,
                                thinking: String::new(),
                                thinking_signature: Some(String::new()),
                                redacted: None,
                            }));
                        metas.push(BlockMeta {
                            index,
                            content_index: output.content.len() - 1,
                            partial_json: String::new(),
                        });
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::ThinkingStart {
                            content_index: partial.content.len() - 1,
                            partial,
                        });
                    }
                    "redacted_thinking" => {
                        let data = block
                            .get("data")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        output
                            .content
                            .push(AssistantContent::Thinking(ThinkingContent {
                                r#type: ThinkingType::Thinking,
                                thinking: "[Reasoning redacted]".to_string(),
                                thinking_signature: Some(data),
                                redacted: Some(true),
                            }));
                        metas.push(BlockMeta {
                            index,
                            content_index: output.content.len() - 1,
                            partial_json: String::new(),
                        });
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::ThinkingStart {
                            content_index: partial.content.len() - 1,
                            partial,
                        });
                    }
                    "tool_use" => {
                        let raw_name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let name = if is_oauth {
                            from_claude_code_name(raw_name, context.tools.as_deref())
                        } else {
                            raw_name.to_string()
                        };
                        output.content.push(AssistantContent::ToolCall(ToolCall {
                            r#type: ToolCallType::ToolCall,
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            name,
                            arguments: block
                                .get("input")
                                .and_then(Value::as_object)
                                .cloned()
                                .unwrap_or_default(),
                            thought_signature: None,
                        }));
                        metas.push(BlockMeta {
                            index,
                            content_index: output.content.len() - 1,
                            partial_json: String::new(),
                        });
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::ToolCallStart {
                            content_index: partial.content.len() - 1,
                            partial,
                        });
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                let Some(meta_pos) = metas.iter().position(|meta| meta.index == index) else {
                    continue;
                };
                let content_index = metas[meta_pos].content_index;
                match delta
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "text_delta" => {
                        let text = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(AssistantContent::Text(block)) =
                            output.content.get_mut(content_index)
                        {
                            block.text.push_str(text);
                            let partial = output.clone();
                            stream.push(AssistantMessageEvent::TextDelta {
                                content_index,
                                delta: text.to_string(),
                                partial,
                            });
                        }
                    }
                    "thinking_delta" => {
                        let text = delta
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(AssistantContent::Thinking(block)) =
                            output.content.get_mut(content_index)
                        {
                            block.thinking.push_str(text);
                            let partial = output.clone();
                            stream.push(AssistantMessageEvent::ThinkingDelta {
                                content_index,
                                delta: text.to_string(),
                                partial,
                            });
                        }
                    }
                    "input_json_delta" => {
                        let text = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let meta = &mut metas[meta_pos];
                        meta.partial_json.push_str(text);
                        let arguments = arguments_from(parse_streaming_json(&meta.partial_json));
                        if let Some(AssistantContent::ToolCall(block)) =
                            output.content.get_mut(content_index)
                        {
                            block.arguments = arguments;
                            let partial = output.clone();
                            stream.push(AssistantMessageEvent::ToolCallDelta {
                                content_index,
                                delta: text.to_string(),
                                partial,
                            });
                        }
                    }
                    "signature_delta" => {
                        let signature = delta
                            .get("signature")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(AssistantContent::Thinking(block)) =
                            output.content.get_mut(content_index)
                        {
                            block
                                .thinking_signature
                                .get_or_insert_with(String::new)
                                .push_str(signature);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
                let Some(meta_pos) = metas.iter().position(|meta| meta.index == index) else {
                    continue;
                };
                // The spec deletes the transient `index`; the block stops
                // being addressable by later events.
                let meta = metas.remove(meta_pos);
                let content_index = meta.content_index;
                match output.content.get_mut(content_index) {
                    Some(AssistantContent::Text(block)) => {
                        let content = block.text.clone();
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::TextEnd {
                            content_index,
                            content,
                            partial,
                        });
                    }
                    Some(AssistantContent::Thinking(block)) => {
                        let content = block.thinking.clone();
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::ThinkingEnd {
                            content_index,
                            content,
                            partial,
                        });
                    }
                    Some(AssistantContent::ToolCall(block)) => {
                        // Finalize; the scratch buffer never persists.
                        block.arguments = arguments_from(parse_streaming_json(&meta.partial_json));
                        let tool_call = block.clone();
                        let partial = output.clone();
                        stream.push(AssistantMessageEvent::ToolCallEnd {
                            content_index,
                            tool_call,
                            partial,
                        });
                    }
                    None => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = event
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.is_empty())
                {
                    output.stop_reason = map_stop_reason(reason)?;
                }
                // Only update usage fields that are present (not null):
                // proxies may omit input_tokens in message_delta.
                let usage = event.get("usage").cloned().unwrap_or(Value::Null);
                if let Some(input) = usage.get("input_tokens").and_then(Value::as_u64) {
                    output.usage.input = input;
                }
                if let Some(out) = usage.get("output_tokens").and_then(Value::as_u64) {
                    output.usage.output = out;
                }
                if let Some(cache_read) =
                    usage.get("cache_read_input_tokens").and_then(Value::as_u64)
                {
                    output.usage.cache_read = cache_read;
                }
                if let Some(cache_write) = usage
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                {
                    output.usage.cache_write = cache_write;
                }
                finalize_usage(model, output);
            }
            "message_stop" => {
                saw_message_stop = true;
            }
            _ => {}
        }
    }

    if saw_message_start && !saw_message_stop {
        return Err(ProtocolError(
            "Anthropic stream ended before message_stop".to_string(),
        ));
    }
    if signal.as_ref().is_some_and(AbortSignal::is_aborted) {
        return Err(ProtocolError("Request was aborted".to_string()));
    }
    if matches!(output.stop_reason, StopReason::Aborted | StopReason::Error) {
        return Err(ProtocolError("An unknown error occurred".to_string()));
    }
    Ok(())
}

fn u64_or_zero(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn finalize_usage(model: &Model, output: &mut AssistantMessage) {
    output.usage.total_tokens = output.usage.input
        + output.usage.output
        + output.usage.cache_read
        + output.usage.cache_write;
    calculate_cost(model, &mut output.usage);
}

/// Spec: `mapThinkingLevelToEffort` — model `thinkingLevelMap` first,
/// then the built-in mapping (default "high").
fn map_thinking_level_to_effort(model: &Model, level: Option<ThinkingLevel>) -> String {
    if let Some(level) = level
        && let Some(Some(mapped)) = model
            .thinking_level_map
            .as_ref()
            .and_then(|map| map.get(&ModelThinkingLevel::from(level)).cloned())
    {
        return mapped;
    }
    match level {
        Some(ThinkingLevel::Minimal | ThinkingLevel::Low) => "low",
        Some(ThinkingLevel::Medium) => "medium",
        Some(ThinkingLevel::High) => "high",
        Some(ThinkingLevel::Max) => "max",
        _ => "high",
    }
    .to_string()
}

/// Spec: `streamSimpleAnthropic` — maps `SimpleStreamOptions` reasoning
/// levels onto adaptive effort or budget-based thinking. The spec throws
/// synchronously when no API key is provided; here that is the `Err`.
pub fn stream_simple_anthropic(
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
    let reasoning = options.as_ref().and_then(|o| o.reasoning);

    let Some(reasoning) = reasoning else {
        return Ok(stream_anthropic(
            model,
            context,
            Some(AnthropicOptions {
                base,
                thinking_enabled: Some(false),
                ..Default::default()
            }),
        ));
    };

    // Adaptive thinking models use an effort level; older models use
    // budget-based thinking.
    if get_anthropic_compat(model).force_adaptive_thinking {
        let effort = map_thinking_level_to_effort(model, Some(reasoning));
        return Ok(stream_anthropic(
            model,
            context,
            Some(AnthropicOptions {
                base,
                thinking_enabled: Some(true),
                effort: Some(effort),
                ..Default::default()
            }),
        ));
    }

    // None = no explicit caller cap; the helper uses the model cap.
    let thinking_budgets = options.as_ref().and_then(|o| o.thinking_budgets);
    let adjusted = adjust_max_tokens_for_thinking(
        base.max_tokens,
        model.max_tokens,
        reasoning,
        thinking_budgets.as_ref(),
    );

    let mut base = base;
    base.max_tokens = Some(adjusted.max_tokens);
    Ok(stream_anthropic(
        model,
        context,
        Some(AnthropicOptions {
            base,
            thinking_enabled: Some(true),
            thinking_budget_tokens: Some(adjusted.thinking_budget),
            ..Default::default()
        }),
    ))
}
