//! Port of `providers/amazon-bedrock.ts` (`bedrock-converse-stream`).
//! Bedrock request mapping, SigV4/bearer auth, and AWS event-stream decoding
//! reuse the crate's HTTP/cancellation primitives.

use std::collections::{BTreeMap, HashMap};

use futures_util::StreamExt;
use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, AssistantRole, CacheRetention,
    Context, Message, Model, ProviderResponse, StopReason, TextContent, ThinkingBudgets,
    ThinkingContent, ThinkingLevel, ThinkingType, ToolCall, ToolCallType, Usage, UserContent,
    calculate_cost, now_ms,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::options::{SimpleStreamOptions, StreamOptions};
use super::simple_options::{adjust_max_tokens_for_thinking, build_base_options};
use super::transform_messages::transform_messages;
use super::{ProtocolError, merge_header, merge_header_map};
use crate::transport::{
    AbortSignal, AssistantMessageEventStream, create_assistant_message_event_stream,
};
use crate::util::{headers_to_record, parse_streaming_json, sanitize_surrogates};

const DEFAULT_TIMEOUT_MS: u64 = 600_000;
const EMPTY_TEXT: &str = "<empty>";
const CLOUD_CACHE_ENV: &str = "PI_CACHE_RETENTION";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BedrockToolChoice {
    Auto,
    Any,
    None,
    Tool { name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BedrockThinkingDisplay {
    Summarized,
    Omitted,
}

impl BedrockThinkingDisplay {
    fn as_str(self) -> &'static str {
        match self {
            Self::Summarized => "summarized",
            Self::Omitted => "omitted",
        }
    }
}

#[derive(Clone, Default)]
pub struct BedrockOptions {
    pub base: StreamOptions,
    pub region: Option<String>,
    pub profile: Option<String>,
    pub tool_choice: Option<BedrockToolChoice>,
    pub reasoning: Option<ThinkingLevel>,
    pub thinking_budgets: Option<ThinkingBudgets>,
    pub interleaved_thinking: Option<bool>,
    pub thinking_display: Option<BedrockThinkingDisplay>,
    pub request_metadata: Option<BTreeMap<String, String>>,
    pub bearer_token: Option<String>,
}

fn candidates(model: &Model) -> Vec<String> {
    [&model.id, &model.name]
        .into_iter()
        .flat_map(|value| {
            let lower = value.to_ascii_lowercase();
            [lower.clone(), lower.replace([' ', '_', '.', ':'], "-")]
        })
        .collect()
}
fn is_claude(model: &Model) -> bool {
    candidates(model)
        .iter()
        .any(|value| value.contains("claude"))
}
fn supports_adaptive(model: &Model) -> bool {
    candidates(model).iter().any(|value| {
        value.contains("opus-4-6")
            || value.contains("opus-4-7")
            || value.contains("opus-4-8")
            || value.contains("sonnet-4-6")
    })
}
fn supports_xhigh(model: &Model) -> bool {
    candidates(model)
        .iter()
        .any(|value| value.contains("opus-4-7") || value.contains("opus-4-8"))
}
fn supports_cache(model: &Model) -> bool {
    let values = candidates(model);
    if !values.iter().any(|value| value.contains("claude")) {
        return std::env::var("AWS_BEDROCK_FORCE_CACHE").as_deref() == Ok("1");
    }
    values.iter().any(|value| {
        value.contains("-4-")
            || value.contains("claude-3-7-sonnet")
            || value.contains("claude-3-5-haiku")
    })
}
fn cache_retention(options: &BedrockOptions) -> CacheRetention {
    options.base.cache_retention.unwrap_or_else(|| {
        if std::env::var(CLOUD_CACHE_ENV).as_deref() == Ok("long") {
            CacheRetention::Long
        } else {
            CacheRetention::Short
        }
    })
}
fn cache_point(retention: CacheRetention) -> Value {
    if retention == CacheRetention::Long {
        json!({"cachePoint":{"type":"default","ttl":"1h"}})
    } else {
        json!({"cachePoint":{"type":"default"}})
    }
}
fn non_blank(text: &str) -> Option<Value> {
    let text = sanitize_surrogates(text);
    (!text.trim().is_empty()).then(|| json!({"text":text}))
}
fn required_text(text: &str) -> Value {
    non_blank(text).unwrap_or_else(|| json!({"text":EMPTY_TEXT}))
}
fn image(image: &pi_rs_ai_types::ImageContent) -> Result<Value, ProtocolError> {
    let format = match image.mime_type.as_str() {
        "image/jpeg" | "image/jpg" => "jpeg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        value => return Err(ProtocolError(format!("Unknown image type: {value}"))),
    };
    Ok(json!({"image":{"format":format,"source":{"bytes":image.data}}}))
}
fn normalize_tool_id(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

fn tool_result_content(
    message: &pi_rs_ai_types::ToolResultMessage,
) -> Result<Vec<Value>, ProtocolError> {
    let mut result = Vec::new();
    for content in &message.content {
        match content {
            pi_rs_ai_types::TextOrImageContent::Text(text) => {
                if let Some(value) = non_blank(&text.text) {
                    result.push(value);
                }
            }
            pi_rs_ai_types::TextOrImageContent::Image(value) => result.push(image(value)?),
        }
    }
    if result.is_empty() {
        result.push(json!({"text":EMPTY_TEXT}));
    }
    Ok(result)
}

fn convert_messages(
    model: &Model,
    context: &Context,
    retention: CacheRetention,
) -> Result<Vec<Value>, ProtocolError> {
    let normalize = |id: &str, _model: &Model, _source: &AssistantMessage| normalize_tool_id(id);
    let messages = transform_messages(&context.messages, model, Some(&normalize));
    let mut result = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        match &messages[index] {
            Message::User(user) => {
                let mut content = Vec::new();
                match &user.content {
                    UserContent::Text(text) => content.push(required_text(text)),
                    UserContent::Blocks(parts) => {
                        for part in parts {
                            match part {
                                pi_rs_ai_types::TextOrImageContent::Text(text) => {
                                    if let Some(value) = non_blank(&text.text) {
                                        content.push(value);
                                    }
                                }
                                pi_rs_ai_types::TextOrImageContent::Image(value) => {
                                    content.push(image(value)?)
                                }
                            }
                        }
                        if content.is_empty() {
                            content.push(json!({"text":EMPTY_TEXT}));
                        }
                    }
                }
                result.push(json!({"role":"user","content":content}));
            }
            Message::Assistant(assistant) => {
                let mut content = Vec::new();
                for block in &assistant.content {
                    match block {
                        AssistantContent::Text(text) => if let Some(value) = non_blank(&text.text) { content.push(value); },
                        AssistantContent::ToolCall(call) => content.push(json!({"toolUse":{"toolUseId":call.id,"name":call.name,"input":call.arguments}})),
                        AssistantContent::Thinking(thinking) if !thinking.thinking.trim().is_empty() => {
                            let text = sanitize_surrogates(&thinking.thinking);
                            if is_claude(model) {
                                if let Some(signature) = thinking.thinking_signature.as_deref().filter(|value| !value.trim().is_empty()) { content.push(json!({"reasoningContent":{"reasoningText":{"text":text,"signature":signature}}})); }
                                else { content.push(json!({"text":text})); }
                            } else { content.push(json!({"reasoningContent":{"reasoningText":{"text":text}}})); }
                        }
                        _ => {}
                    }
                }
                if !content.is_empty() {
                    result.push(json!({"role":"assistant","content":content}));
                }
            }
            Message::ToolResult(_) => {
                let mut content = Vec::new();
                let mut next = index;
                while next < messages.len() {
                    let Message::ToolResult(tool) = &messages[next] else {
                        break;
                    };
                    content.push(json!({"toolResult":{"toolUseId":tool.tool_call_id,"content":tool_result_content(tool)?,"status":if tool.is_error {"error"} else {"success"}}}));
                    next += 1;
                }
                index = next - 1;
                result.push(json!({"role":"user","content":content}));
            }
        }
        index += 1;
    }
    if retention != CacheRetention::None
        && supports_cache(model)
        && let Some(last) = result.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
        && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
    {
        content.push(cache_point(retention));
    }
    Ok(result)
}

fn system_prompt(
    model: &Model,
    context: &Context,
    retention: CacheRetention,
) -> Option<Vec<Value>> {
    let prompt = context.system_prompt.as_deref()?;
    if prompt.is_empty() {
        return None;
    }
    let mut blocks = vec![json!({"text":sanitize_surrogates(prompt)})];
    if retention != CacheRetention::None && supports_cache(model) {
        blocks.push(cache_point(retention));
    }
    Some(blocks)
}
fn tool_config(context: &Context, choice: Option<&BedrockToolChoice>) -> Option<Value> {
    let tools = context.tools.as_ref()?;
    if tools.is_empty() || matches!(choice, Some(BedrockToolChoice::None)) {
        return None;
    }
    let tools = tools.iter().map(|tool| json!({"toolSpec":{"name":tool.name,"inputSchema":{"json":tool.parameters},"description":tool.description}})).collect::<Vec<_>>();
    let choice = match choice {
        Some(BedrockToolChoice::Auto) => Some(json!({"auto":{}})),
        Some(BedrockToolChoice::Any) => Some(json!({"any":{}})),
        Some(BedrockToolChoice::Tool { name }) => Some(json!({"tool":{"name":name}})),
        _ => None,
    };
    let mut value = json!({"tools":tools});
    if let Some(choice) = choice {
        value["toolChoice"] = choice;
    }
    Some(value)
}
fn effort(model: &Model, level: ThinkingLevel) -> &'static str {
    if level == ThinkingLevel::XHigh && supports_xhigh(model) {
        return "xhigh";
    }
    match level {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        _ => "high",
    }
}
fn is_gov(model: &Model, options: &BedrockOptions) -> bool {
    options
        .region
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("us-gov-"))
        || std::env::var("AWS_REGION")
            .is_ok_and(|value| value.to_ascii_lowercase().starts_with("us-gov-"))
        || model.id.to_ascii_lowercase().starts_with("us-gov.")
        || model.id.to_ascii_lowercase().starts_with("arn:aws-us-gov:")
}
fn additional_fields(model: &Model, options: &BedrockOptions) -> Option<Value> {
    let reasoning = options.reasoning?;
    if !model.reasoning || !is_claude(model) {
        return None;
    }
    let display = (!is_gov(model, options)).then(|| {
        options
            .thinking_display
            .unwrap_or(BedrockThinkingDisplay::Summarized)
            .as_str()
    });
    if supports_adaptive(model) {
        let mut thinking = json!({"type":"adaptive"});
        if let Some(display) = display {
            thinking["display"] = json!(display);
        }
        return Some(
            json!({"thinking":thinking,"output_config":{"effort":effort(model, reasoning)}}),
        );
    }
    let level = if reasoning == ThinkingLevel::XHigh {
        ThinkingLevel::High
    } else {
        reasoning
    };
    let budget = options
        .thinking_budgets
        .as_ref()
        .and_then(|budgets| match level {
            ThinkingLevel::Minimal => budgets.minimal,
            ThinkingLevel::Low => budgets.low,
            ThinkingLevel::Medium => budgets.medium,
            _ => budgets.high,
        })
        .unwrap_or(match reasoning {
            ThinkingLevel::Minimal => 1024,
            ThinkingLevel::Low => 2048,
            ThinkingLevel::Medium => 8192,
            _ => 16384,
        });
    let mut thinking = json!({"type":"enabled","budget_tokens":budget});
    if let Some(display) = display {
        thinking["display"] = json!(display);
    }
    let mut result = json!({"thinking":thinking});
    if options.interleaved_thinking.unwrap_or(true) {
        result["anthropic_beta"] = json!(["interleaved-thinking-2025-05-14"]);
    }
    Some(result)
}
fn build_params(
    model: &Model,
    context: &Context,
    options: &BedrockOptions,
) -> Result<Value, ProtocolError> {
    let retention = cache_retention(options);
    let mut value = Map::new();
    value.insert(
        "messages".into(),
        Value::Array(convert_messages(model, context, retention)?),
    );
    if let Some(system) = system_prompt(model, context, retention) {
        value.insert("system".into(), Value::Array(system));
    }
    let mut inference = Map::new();
    if let Some(max) = options
        .base
        .max_tokens
        .or_else(|| is_claude(model).then_some(model.max_tokens))
    {
        inference.insert("maxTokens".into(), json!(max));
    }
    if let Some(temperature) = options.base.temperature {
        inference.insert("temperature".into(), json!(temperature));
    }
    value.insert("inferenceConfig".into(), Value::Object(inference));
    if let Some(tools) = tool_config(context, options.tool_choice.as_ref()) {
        value.insert("toolConfig".into(), tools);
    }
    if let Some(fields) = additional_fields(model, options) {
        value.insert("additionalModelRequestFields".into(), fields);
    }
    if let Some(metadata) = &options.request_metadata {
        value.insert("requestMetadata".into(), json!(metadata));
    }
    Ok(Value::Object(value))
}

fn region(model: &Model, options: &BedrockOptions) -> String {
    if let Some(value) = options
        .region
        .clone()
        .or_else(|| std::env::var("AWS_REGION").ok())
        .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
    {
        return value;
    }
    let host = url::Url::parse(&model.base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    host.strip_prefix("bedrock-runtime.")
        .and_then(|rest| rest.split(".amazonaws.com").next())
        .filter(|value| !value.is_empty())
        .unwrap_or("us-east-1")
        .to_string()
}
fn request_url(model: &Model) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(model.id.as_bytes()).collect();
    format!(
        "{}/model/{encoded}/converse-stream",
        model.base_url.trim_end_matches('/')
    )
}
fn sha256(value: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, value)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn hmac(key: &[u8], value: &str) -> Vec<u8> {
    ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
        value.as_bytes(),
    )
    .as_ref()
    .to_vec()
}
fn aws_date() -> Result<(String, String), ProtocolError> {
    let value = httpdate::fmt_http_date(std::time::SystemTime::now());
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 6 {
        return Err(ProtocolError("Unable to format AWS request date".into()));
    }
    let month = match parts[2] {
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
    let date = format!("{}{}{}", parts[3], month, parts[1]);
    Ok((
        date.clone(),
        format!("{date}T{}Z", parts[4].replace(':', "")),
    ))
}
fn profile_credentials(profile: &str) -> Option<(String, String, Option<String>)> {
    let home = std::env::var("HOME").ok()?;
    let raw = std::fs::read_to_string(std::path::Path::new(&home).join(".aws/credentials")).ok()?;
    let mut active = false;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            active = line.trim_matches(['[', ']']) == profile;
            continue;
        }
        if active && let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim(), value.trim().to_string());
        }
    }
    Some((
        values.get("aws_access_key_id")?.clone(),
        values.get("aws_secret_access_key")?.clone(),
        values.get("aws_session_token").cloned(),
    ))
}
fn credentials(
    options: &BedrockOptions,
) -> Result<(String, String, Option<String>), ProtocolError> {
    if std::env::var("AWS_BEDROCK_SKIP_AUTH").as_deref() == Ok("1") {
        return Ok(("dummy-access-key".into(), "dummy-secret-key".into(), None));
    }
    if let (Ok(access), Ok(secret)) = (
        std::env::var("AWS_ACCESS_KEY_ID"),
        std::env::var("AWS_SECRET_ACCESS_KEY"),
    ) {
        return Ok((access, secret, std::env::var("AWS_SESSION_TOKEN").ok()));
    }
    let profile = options
        .profile
        .clone()
        .or_else(|| std::env::var("AWS_PROFILE").ok());
    if let Some(profile) = profile
        && let Some(value) = profile_credentials(&profile)
    {
        return Ok(value);
    }
    Err(ProtocolError(
        "Could not load credentials from any providers in the chain".into(),
    ))
}
fn signed_headers(
    url: &str,
    body: &str,
    region: &str,
    credentials: &(String, String, Option<String>),
) -> Result<Vec<(String, String)>, ProtocolError> {
    let parsed = url::Url::parse(url).map_err(|error| ProtocolError(error.to_string()))?;
    let host = parsed
        .host_str()
        .map(|host| {
            parsed
                .port()
                .map_or_else(|| host.to_string(), |port| format!("{host}:{port}"))
        })
        .ok_or_else(|| ProtocolError("invalid Bedrock endpoint".into()))?;
    let (date, amz) = aws_date()?;
    let mut canonical = BTreeMap::new();
    canonical.insert("content-type", "application/json".to_string());
    canonical.insert("host", host);
    canonical.insert("x-amz-date", amz.clone());
    if let Some(token) = &credentials.2 {
        canonical.insert("x-amz-security-token", token.clone());
    }
    let names = canonical.keys().copied().collect::<Vec<_>>().join(";");
    let lines = canonical
        .iter()
        .map(|(key, value)| format!("{key}:{value}\n"))
        .collect::<String>();
    let request = format!(
        "POST\n{}\n{}\n{lines}\n{names}\n{}",
        parsed.path(),
        parsed.query().unwrap_or_default(),
        sha256(body.as_bytes())
    );
    let scope = format!("{date}/{region}/bedrock/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz}\n{scope}\n{}",
        sha256(request.as_bytes())
    );
    let kd = hmac(format!("AWS4{}", credentials.1).as_bytes(), &date);
    let kr = hmac(&kd, region);
    let ks = hmac(&kr, "bedrock");
    let key = hmac(&ks, "aws4_request");
    let signature = hmac(&key, &to_sign)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut result = canonical
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect::<Vec<_>>();
    result.push((
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={names}, Signature={signature}",
            credentials.0
        ),
    ));
    Ok(result)
}
fn request_headers(
    model: &Model,
    options: &BedrockOptions,
    url: &str,
    body: &str,
) -> Result<HeaderMap, ProtocolError> {
    let bearer = options
        .bearer_token
        .clone()
        .or_else(|| std::env::var("AWS_BEARER_TOKEN_BEDROCK").ok())
        .filter(|_| std::env::var("AWS_BEDROCK_SKIP_AUTH").as_deref() != Ok("1"));
    let mut values = if let Some(token) = bearer {
        vec![
            ("content-type".into(), "application/json".into()),
            ("authorization".into(), format!("Bearer {token}")),
        ]
    } else {
        signed_headers(url, body, &region(model, options), &credentials(options)?)?
    };
    merge_header_map(&mut values, model.headers.as_ref());
    if let Some(headers) = &options.base.headers {
        for (key, value) in headers {
            let lower = key.to_ascii_lowercase();
            if lower != "authorization" && lower != "host" && !lower.starts_with("x-amz-") {
                merge_header(&mut values, key, value);
            }
        }
    }
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

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn event_type(headers: &[u8]) -> Result<Option<String>, ProtocolError> {
    let mut position = 0;
    while position < headers.len() {
        let length = usize::from(headers[position]);
        position += 1;
        if position + length + 1 > headers.len() {
            return Err(ProtocolError("Invalid AWS event-stream header".into()));
        }
        let name = std::str::from_utf8(&headers[position..position + length])
            .map_err(|error| ProtocolError(error.to_string()))?;
        position += length;
        let kind = headers[position];
        position += 1;
        if kind != 7 || position + 2 > headers.len() {
            return Err(ProtocolError("Unsupported AWS event-stream header".into()));
        }
        let size = u16::from_be_bytes([headers[position], headers[position + 1]]) as usize;
        position += 2;
        if position + size > headers.len() {
            return Err(ProtocolError("Invalid AWS event-stream header".into()));
        }
        let value = std::str::from_utf8(&headers[position..position + size])
            .map_err(|error| ProtocolError(error.to_string()))?;
        position += size;
        if matches!(name, ":event-type" | ":exception-type") {
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}
fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<(String, Value)>, ProtocolError> {
    if buffer.len() < 12 {
        return Ok(None);
    }
    let total = u32::from_be_bytes(
        buffer[0..4]
            .try_into()
            .map_err(|_| ProtocolError("Invalid AWS event-stream prelude".into()))?,
    ) as usize;
    let header_len = u32::from_be_bytes(
        buffer[4..8]
            .try_into()
            .map_err(|_| ProtocolError("Invalid AWS event-stream prelude".into()))?,
    ) as usize;
    let prelude_crc = u32::from_be_bytes(
        buffer[8..12]
            .try_into()
            .map_err(|_| ProtocolError("Invalid AWS event-stream prelude".into()))?,
    );
    if prelude_crc != crc32(&buffer[..8]) {
        return Err(ProtocolError("Invalid AWS event-stream prelude CRC".into()));
    }
    if total < 16 + header_len {
        return Err(ProtocolError("Invalid AWS event-stream length".into()));
    }
    if buffer.len() < total {
        return Ok(None);
    }
    let raw = buffer.drain(..total).collect::<Vec<_>>();
    let message_crc = u32::from_be_bytes(
        raw[total - 4..]
            .try_into()
            .map_err(|_| ProtocolError("Invalid AWS event-stream message CRC".into()))?,
    );
    if message_crc != crc32(&raw[..total - 4]) {
        return Err(ProtocolError("Invalid AWS event-stream message CRC".into()));
    }
    let kind = event_type(&raw[12..12 + header_len])?
        .ok_or_else(|| ProtocolError("AWS event-stream frame has no event type".into()))?;
    let body = &raw[12 + header_len..total - 4];
    let value = serde_json::from_slice(body).map_err(|error| ProtocolError(error.to_string()))?;
    Ok(Some((kind, value)))
}

struct StreamBlock {
    index: usize,
    partial: String,
}
fn map_stop(value: &str) -> StopReason {
    match value {
        "end_turn" | "stop_sequence" => StopReason::Stop,
        "max_tokens" | "model_context_window_exceeded" => StopReason::Length,
        "tool_use" => StopReason::ToolUse,
        _ => StopReason::Error,
    }
}
fn process_event(
    kind: &str,
    value: &Value,
    model: &Model,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
    blocks: &mut HashMap<u64, StreamBlock>,
) -> Result<(), ProtocolError> {
    match kind {
        "messageStart" => {
            if value.get("role").and_then(Value::as_str) != Some("assistant") {
                return Err(ProtocolError(
                    "Unexpected assistant message start but got user message start instead".into(),
                ));
            }
            stream.push(AssistantMessageEvent::Start {
                partial: output.clone(),
            });
        }
        "contentBlockStart" => {
            if let Some(tool) = value.get("start").and_then(|start| start.get("toolUse")) {
                let wire = value
                    .get("contentBlockIndex")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                output.content.push(AssistantContent::ToolCall(ToolCall {
                    r#type: ToolCallType::ToolCall,
                    id: tool
                        .get("toolUseId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    name: tool
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments: Map::new(),
                    thought_signature: None,
                }));
                let index = output.content.len() - 1;
                blocks.insert(
                    wire,
                    StreamBlock {
                        index,
                        partial: String::new(),
                    },
                );
                stream.push(AssistantMessageEvent::ToolCallStart {
                    content_index: index,
                    partial: output.clone(),
                });
            }
        }
        "contentBlockDelta" => {
            let wire = value
                .get("contentBlockIndex")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let delta = &value["delta"];
            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                let index = if let Some(block) = blocks.get(&wire) {
                    block.index
                } else {
                    output
                        .content
                        .push(AssistantContent::Text(TextContent::new("")));
                    let index = output.content.len() - 1;
                    blocks.insert(
                        wire,
                        StreamBlock {
                            index,
                            partial: String::new(),
                        },
                    );
                    stream.push(AssistantMessageEvent::TextStart {
                        content_index: index,
                        partial: output.clone(),
                    });
                    index
                };
                if let Some(AssistantContent::Text(block)) = output.content.get_mut(index) {
                    block.text.push_str(text);
                }
                stream.push(AssistantMessageEvent::TextDelta {
                    content_index: index,
                    delta: text.to_string(),
                    partial: output.clone(),
                });
            } else if let Some(tool) = delta.get("toolUse") {
                if let Some(block) = blocks.get_mut(&wire) {
                    let text = tool.get("input").and_then(Value::as_str).unwrap_or("");
                    block.partial.push_str(text);
                    if let Some(AssistantContent::ToolCall(call)) =
                        output.content.get_mut(block.index)
                    {
                        call.arguments = match parse_streaming_json(&block.partial) {
                            Value::Object(map) => map,
                            _ => Map::new(),
                        };
                    }
                    stream.push(AssistantMessageEvent::ToolCallDelta {
                        content_index: block.index,
                        delta: text.to_string(),
                        partial: output.clone(),
                    });
                }
            } else if let Some(reasoning) = delta.get("reasoningContent") {
                let index = if let Some(block) = blocks.get(&wire) {
                    block.index
                } else {
                    output
                        .content
                        .push(AssistantContent::Thinking(ThinkingContent {
                            r#type: ThinkingType::Thinking,
                            thinking: String::new(),
                            thinking_signature: Some(String::new()),
                            redacted: None,
                        }));
                    let index = output.content.len() - 1;
                    blocks.insert(
                        wire,
                        StreamBlock {
                            index,
                            partial: String::new(),
                        },
                    );
                    stream.push(AssistantMessageEvent::ThinkingStart {
                        content_index: index,
                        partial: output.clone(),
                    });
                    index
                };
                let text = reasoning
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty());
                if let Some(AssistantContent::Thinking(block)) = output.content.get_mut(index) {
                    if let Some(text) = text {
                        block.thinking.push_str(text);
                    }
                    if let Some(signature) = reasoning.get("signature").and_then(Value::as_str) {
                        block
                            .thinking_signature
                            .get_or_insert_default()
                            .push_str(signature);
                    }
                }
                if let Some(text) = text {
                    stream.push(AssistantMessageEvent::ThinkingDelta {
                        content_index: index,
                        delta: text.to_string(),
                        partial: output.clone(),
                    });
                }
            }
        }
        "contentBlockStop" => {
            let wire = value
                .get("contentBlockIndex")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if let Some(block) = blocks.get(&wire) {
                match &mut output.content[block.index] {
                    AssistantContent::Text(text) => stream.push(AssistantMessageEvent::TextEnd {
                        content_index: block.index,
                        content: text.text.clone(),
                        partial: output.clone(),
                    }),
                    AssistantContent::Thinking(thinking) => {
                        stream.push(AssistantMessageEvent::ThinkingEnd {
                            content_index: block.index,
                            content: thinking.thinking.clone(),
                            partial: output.clone(),
                        })
                    }
                    AssistantContent::ToolCall(call) => {
                        call.arguments = match parse_streaming_json(&block.partial) {
                            Value::Object(map) => map,
                            _ => Map::new(),
                        };
                        stream.push(AssistantMessageEvent::ToolCallEnd {
                            content_index: block.index,
                            tool_call: call.clone(),
                            partial: output.clone(),
                        });
                    }
                }
            }
        }
        "messageStop" => {
            output.stop_reason = map_stop(
                value
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
        }
        "metadata" => {
            if let Some(usage) = value.get("usage") {
                output.usage.input = usage
                    .get("inputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                output.usage.output = usage
                    .get("outputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                output.usage.cache_read = usage
                    .get("cacheReadInputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                output.usage.cache_write = usage
                    .get("cacheWriteInputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                output.usage.total_tokens = usage
                    .get("totalTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(output.usage.input + output.usage.output);
                calculate_cost(model, &mut output.usage);
            }
        }
        "internalServerException"
        | "modelStreamErrorException"
        | "validationException"
        | "throttlingException"
        | "serviceUnavailableException" => {
            return Err(ProtocolError(value.to_string()));
        }
        _ => {}
    }
    Ok(())
}

async fn drive(
    model: &Model,
    context: &Context,
    options: &BedrockOptions,
    stream: &AssistantMessageEventStream,
    output: &mut AssistantMessage,
) -> Result<(), ProtocolError> {
    let mut params = build_params(model, context, options)?;
    if let Some(hook) = &options.base.on_payload
        && let Some(next) = hook(params.clone(), model.clone()).await
    {
        params = next;
    }
    let body = params.to_string();
    let url = request_url(model);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            options.base.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
        ))
        .build()
        .map_err(|error| ProtocolError(error.to_string()))?;
    let response = client
        .post(&url)
        .headers(request_headers(model, options, &url, &body)?)
        .body(body)
        .send()
        .await
        .map_err(|error| ProtocolError(error.to_string()))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or(body);
        return Err(ProtocolError(format!("Unknown: {message}")));
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
    let mut bytes = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut blocks = HashMap::new();
    while let Some(chunk) = bytes.next().await {
        buffer.extend(chunk.map_err(|error| ProtocolError(error.to_string()))?);
        while let Some((kind, value)) = take_frame(&mut buffer)? {
            process_event(&kind, &value, model, stream, output, &mut blocks)?;
        }
    }
    if !buffer.is_empty() {
        return Err(ProtocolError("Truncated AWS event-stream frame".into()));
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

pub fn stream_bedrock(
    model: &Model,
    context: &Context,
    options: Option<BedrockOptions>,
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

pub fn stream_simple_bedrock(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    let mut base = build_base_options(model, options.as_ref(), None);
    let reasoning = options.as_ref().and_then(|value| value.reasoning);
    let mut budgets = options.as_ref().and_then(|value| value.thinking_budgets);
    if is_claude(model) && reasoning.is_some() && !supports_adaptive(model) {
        let adjusted = adjust_max_tokens_for_thinking(
            base.max_tokens,
            model.max_tokens,
            reasoning.unwrap_or(ThinkingLevel::High),
            budgets.as_ref(),
        );
        base.max_tokens = Some(adjusted.max_tokens);
        let mut value = budgets.unwrap_or_default();
        match reasoning.unwrap_or(ThinkingLevel::High) {
            ThinkingLevel::Minimal => value.minimal = Some(adjusted.thinking_budget),
            ThinkingLevel::Low => value.low = Some(adjusted.thinking_budget),
            ThinkingLevel::Medium => value.medium = Some(adjusted.thinking_budget),
            _ => value.high = Some(adjusted.thinking_budget),
        }
        budgets = Some(value);
    }
    Ok(stream_bedrock(
        model,
        context,
        Some(BedrockOptions {
            base,
            reasoning,
            thinking_budgets: budgets,
            ..BedrockOptions::default()
        }),
    ))
}
