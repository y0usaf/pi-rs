use serde_json::{Map, Value, json};

use crate::simple_options::resolve_sampling_params;
use crate::thinking::level_name;
use crate::transform::{ToolCallIdNormalizer, transform_messages};
use crate::types::{
    AssistantBlock, AssistantMessage, CacheRetention, ImageContent, Message, Model,
    ModelThinkingLevel, Payload, SystemMessage, Tool, TranscriptContext, UserBlock, UserContent,
};
use crate::utils::text::{system_content_text, system_message_text, utf16_len};
use crate::utils::transcript::{ToolSupport, resolve_transcript_tools};

use super::codex::DEFAULT_INSTRUCTIONS;

const TOOL_CALL_PROVIDERS: [&str; 3] = ["openai", "openai-codex", "opencode"];
const MIN_OUTPUT_TOKENS: u64 = 16;
const ITEM_ID_MAX_CHARS: usize = 64;

pub struct Compat {
    developer_role: bool,
    pub mid_convo_system: bool,
    openrouter_session: bool,
    long_cache: bool,
    strict: bool,
    max_output: bool,
    explicit_cache: bool,
    pub codex: bool,
    additional_tools: bool,
    tool_search: bool,
}

impl Compat {
    pub fn of(model: &Model) -> Self {
        let flag = |key: &str, default: bool| {
            model
                .compat
                .as_ref()
                .and_then(|compat| compat.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(default)
        };
        let openrouter = model.provider == "openrouter" || model.base_url.contains("openrouter.ai");
        let format = model
            .compat
            .as_ref()
            .and_then(|compat| compat.get("sessionAffinityFormat"))
            .and_then(Value::as_str);
        Self {
            developer_role: flag("supportsDeveloperRole", true),
            mid_convo_system: flag("supportsMidConvoSystemMessages", false),
            openrouter_session: format.map_or(openrouter, |format| format == "openrouter"),
            long_cache: flag("supportsLongCacheRetention", true),
            strict: flag("supportsStrictMode", false),
            max_output: flag("supportsMaxOutputTokens", true),
            explicit_cache: flag("supportsExplicitPromptCacheMode", false),
            codex: model.api == "openai-codex-responses",
            additional_tools: flag("supportsAdditionalTools", false),
            tool_search: flag("supportsToolSearch", false),
        }
    }

    pub fn tool_support(&self) -> ToolSupport {
        if self.mid_convo_system && self.additional_tools {
            return ToolSupport::Changes;
        }
        if self.tool_search {
            return ToolSupport::Additions;
        }
        ToolSupport::None
    }

    pub fn openrouter_session(&self) -> bool {
        self.openrouter_session
    }
}

pub struct Turn {
    pub request_model: Option<String>,
    pub cache_key: Option<String>,
    pub retention: CacheRetention,
    pub max_tokens: u64,
    pub effort: Option<ModelThinkingLevel>,
    pub chatgpt_sign_in: bool,
}

pub fn build(
    model: &Model,
    context: &TranscriptContext,
    compat: &Compat,
    turn: &Turn,
) -> Result<Payload, String> {
    let plain = !turn.chatgpt_sign_in && !compat.codex;
    let support = compat.tool_support();
    let transcript_tools = resolve_transcript_tools(&context.messages, support);
    let model_id = json!(turn.request_model.as_deref().unwrap_or(&model.id));
    let input = input(
        model,
        context,
        compat,
        support,
        transcript_tools.folded_system_messages,
    )?;
    let cache_key = turn.cache_key.as_ref().map(|key| json!(key));
    let mut payload = Payload::new(Map::new());
    if compat.codex {
        payload.body.insert("model".to_owned(), model_id);
        payload.body.insert("store".to_owned(), json!(false));
        payload.body.insert("stream".to_owned(), json!(true));
        payload
            .body
            .insert("instructions".to_owned(), json!(instructions(context)));
        payload.body.insert("input".to_owned(), Value::Array(input));
        payload
            .body
            .insert("text".to_owned(), json!({"verbosity": "low"}));
        payload
            .body
            .insert("include".to_owned(), json!(["reasoning.encrypted_content"]));
        payload.declare(&["prompt_cache_key"], cache_key);
        payload.body.insert("tool_choice".to_owned(), json!("auto"));
        payload
            .body
            .insert("parallel_tool_calls".to_owned(), json!(true));
    } else {
        payload.body.insert("model".to_owned(), model_id);
        payload.body.insert("input".to_owned(), Value::Array(input));
        payload.body.insert("stream".to_owned(), json!(true));
        payload.declare(&["prompt_cache_key"], cache_key);
        let retention = (plain
            && turn.retention == CacheRetention::Long
            && compat.long_cache
            && !compat.explicit_cache)
            .then(|| json!("24h"));
        payload.declare(&["prompt_cache_retention"], retention);
        let cache_options = match turn.retention {
            _ if !plain || !compat.explicit_cache => None,
            CacheRetention::None => Some(json!({"mode": "explicit"})),
            CacheRetention::Long if compat.long_cache => Some(json!({"ttl": "30m"})),
            _ => None,
        };
        payload.declare(&["prompt_cache_options"], cache_options);
        payload.body.insert("store".to_owned(), json!(false));
        if plain && compat.max_output && turn.max_tokens > 0 {
            payload.body.insert(
                "max_output_tokens".to_owned(),
                json!(turn.max_tokens.max(MIN_OUTPUT_TOKENS)),
            );
        }
    }
    let params = &mut payload.body;
    if !transcript_tools.request_tools.is_empty() {
        params.insert(
            "tools".to_owned(),
            Value::Array(tool_items(&transcript_tools.request_tools, compat, false)),
        );
    }
    if model.reasoning {
        reasoning(model, turn.effort, params);
    }
    if let Some(sampling) =
        resolve_sampling_params(model, turn.effort.unwrap_or(ModelThinkingLevel::Off), None)
    {
        params.extend(sampling);
    }
    Ok(payload)
}

fn instructions(context: &TranscriptContext) -> String {
    let text = match context.messages.first() {
        Some(Message::System(system)) => system_message_text(system),
        _ => String::new(),
    };
    if text.is_empty() {
        DEFAULT_INSTRUCTIONS.to_owned()
    } else {
        text
    }
}

fn reasoning(model: &Model, effort: Option<ModelThinkingLevel>, params: &mut Map<String, Value>) {
    let map = model.thinking_level_map.as_ref();
    let mapped = |name: &str| {
        map.and_then(|map| map.get(name))
            .filter(|value| !value.is_null())
            .cloned()
    };
    match effort {
        Some(level) => {
            let name = level_name(level);
            let effort = mapped(name).unwrap_or_else(|| json!(name));
            params.insert(
                "reasoning".to_owned(),
                json!({"effort": effort, "summary": "auto"}),
            );
            params.insert("include".to_owned(), json!(["reasoning.encrypted_content"]));
        }
        None if model.provider != "github-copilot"
            && map.and_then(|map| map.get("off")) != Some(&Value::Null) =>
        {
            let effort = mapped("off").unwrap_or_else(|| json!("none"));
            params.insert("reasoning".to_owned(), json!({"effort": effort}));
        }
        None => {}
    }
    if model.provider == "xai" {
        params.insert("include".to_owned(), json!(["reasoning.encrypted_content"]));
    }
}

fn tool_items(tools: &[Tool], compat: &Compat, deferred: bool) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            let mut item = json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            });
            if compat.strict {
                item["strict"] = json!(false);
            }
            if deferred {
                item["defer_loading"] = json!(true);
            }
            item
        })
        .collect()
}

fn input(
    model: &Model,
    context: &TranscriptContext,
    compat: &Compat,
    support: ToolSupport,
    folded_system_messages: usize,
) -> Result<Vec<Value>, String> {
    let normalize =
        |id: &str, _target: &Model, source: &AssistantMessage| normalize_call_id(model, id, source);
    let normalizer: &ToolCallIdNormalizer = &normalize;
    let messages = transform_messages(&context.messages, model, Some(normalizer));
    let instruction_role = if model.reasoning && compat.developer_role {
        "developer"
    } else {
        "system"
    };
    let mut items = Vec::new();
    let mut msg_index = 0usize;
    let mut system_index = 0usize;
    let mut declared_additional_tools = false;
    let mut unavailable: Vec<String> = Vec::new();
    for (source_index, message) in messages.iter().enumerate() {
        let leading_system = source_index == 0 && matches!(message, Message::System(_));
        match message {
            Message::System(system) => {
                let anchors = system_index >= folded_system_messages;
                system_index += 1;
                if anchors {
                    append_tool_additions(
                        compat,
                        support,
                        system,
                        msg_index,
                        &mut items,
                        &mut declared_additional_tools,
                        &mut unavailable,
                    );
                }
                if leading_system && compat.codex {
                    continue;
                }
                let mut parts = vec![if leading_system {
                    system_message_text(system)
                } else {
                    render_update(system)
                }];
                if anchors {
                    let added: Vec<&str> = system
                        .tools_added
                        .iter()
                        .flatten()
                        .map(|tool| tool.name.as_str())
                        .collect();
                    let removed: Vec<&str> = system
                        .tools_removed
                        .iter()
                        .flatten()
                        .map(|tool| tool.name.as_str())
                        .filter(|name| !added.contains(name))
                        .collect();
                    if !removed.is_empty() {
                        parts.push(render_tool_removal_notice(&removed));
                    }
                    unavailable.extend(removed.into_iter().map(str::to_owned));
                }
                let text = parts
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if !text.is_empty() {
                    items.push(json!({"role": instruction_role, "content": text}));
                }
            }
            Message::User(user) => {
                let content: Vec<Value> = match &user.content {
                    UserContent::Text(text) => vec![json!({"type": "input_text", "text": text})],
                    UserContent::Blocks(blocks) => blocks.iter().map(user_part).collect(),
                };
                if content.is_empty() {
                    continue;
                }
                items.push(json!({"role": "user", "content": content}));
            }
            Message::Assistant(assistant) => {
                let output = assistant_items(model, assistant, msg_index)?;
                if output.is_empty() {
                    continue;
                }
                items.extend(output);
            }
            Message::ToolResult(result) => {
                let call_id = result.tool_call_id.split('|').next().unwrap_or("");
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": tool_output(model, &result.content),
                }));
            }
        }
        if !leading_system {
            msg_index += 1;
        }
    }
    Ok(items)
}

/// Declare a system message's tool additions the way the transport can: in place for
/// [ToolSupport::Changes], as a synthetic tool search for [ToolSupport::Additions].
fn append_tool_additions(
    compat: &Compat,
    support: ToolSupport,
    system: &SystemMessage,
    msg_index: usize,
    items: &mut Vec<Value>,
    declared_additional_tools: &mut bool,
    unavailable: &mut Vec<String>,
) {
    let tools = system.tools_added.as_deref().unwrap_or_default();
    unavailable.retain(|name| !tools.iter().any(|tool| tool.name == *name));
    if tools.is_empty() {
        return;
    }
    match support {
        ToolSupport::Changes => {
            if *declared_additional_tools {
                items.push(json!({
                    "role": "developer",
                    "content": render_incremental_tools_notice(unavailable),
                }));
            }
            *declared_additional_tools = true;
            items.push(json!({
                "type": "additional_tools",
                "role": "developer",
                "tools": tool_items(tools, compat, false),
            }));
        }
        ToolSupport::Additions => {
            let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
            let call_id = format!(
                "pi_tool_load_{}",
                short_hash(&format!("system:{msg_index}:{}", names.join(",")))
            );
            items.push(json!({
                "type": "tool_search_call",
                "call_id": call_id,
                "execution": "client",
                "status": "completed",
                "arguments": {"query": names.join(" "), "limit": names.len()},
            }));
            items.push(json!({
                "type": "tool_search_output",
                "call_id": call_id,
                "execution": "client",
                "status": "completed",
                "tools": tool_items(tools, compat, true),
            }));
        }
        ToolSupport::None => {}
    }
}

fn render_incremental_tools_notice(unavailable: &[String]) -> String {
    let notice = "This is an incremental tools update. Previously declared tools remain available unless explicitly marked unavailable. A tool declared again here is available again, and its latest definition replaces the earlier one.";
    if unavailable.is_empty() {
        notice.to_owned()
    } else {
        format!("{notice}\nStill unavailable: {}.", unavailable.join(", "))
    }
}

fn render_tool_removal_notice(names: &[&str]) -> String {
    format!(
        "The following tools are no longer available. Do not call them:\n{}",
        names
            .iter()
            .map(|name| format!("- {name}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub(crate) fn render_update(system: &SystemMessage) -> String {
    let mut parts = Vec::new();
    let text = system_content_text(&system.content);
    if !text.is_empty() {
        parts.push(text);
    }
    for (name, value) in system
        .sections
        .iter()
        .flat_map(|sections| sections.0.iter())
    {
        parts.push(match value {
            None => format!("Removed system prompt section \"{name}\"."),
            Some(value) => format!("Updated system prompt section \"{name}\":\n\n{value}"),
        });
    }
    parts.join("\n\n")
}

fn user_part(block: &UserBlock) -> Value {
    match block {
        UserBlock::Text(text) => json!({"type": "input_text", "text": text.text}),
        UserBlock::Image(image) => image_part(image),
    }
}

fn image_part(image: &ImageContent) -> Value {
    json!({
        "type": "input_image",
        "detail": "auto",
        "image_url": format!("data:{};base64,{}", image.mime_type, image.data),
    })
}

fn tool_output(model: &Model, content: &[UserBlock]) -> Value {
    let text = content
        .iter()
        .filter_map(|block| match block {
            UserBlock::Text(text) => Some(text.text.as_str()),
            UserBlock::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let images: Vec<&ImageContent> = content
        .iter()
        .filter_map(|block| match block {
            UserBlock::Image(image) => Some(image),
            UserBlock::Text(_) => None,
        })
        .collect();
    let has_text = !text.is_empty();
    let accepts_images = model.input.iter().any(|kind| kind == "image");
    if images.is_empty() || !accepts_images {
        let output = if has_text {
            text
        } else if !images.is_empty() {
            "(see attached image)".to_owned()
        } else {
            "(no tool output)".to_owned()
        };
        return json!(output);
    }
    let mut parts = Vec::new();
    if has_text {
        parts.push(json!({"type": "input_text", "text": text}));
    }
    parts.extend(images.into_iter().map(image_part));
    Value::Array(parts)
}

fn assistant_items(
    model: &Model,
    message: &AssistantMessage,
    msg_index: usize,
) -> Result<Vec<Value>, String> {
    let same_api = message.provider == model.provider && message.api == model.api;
    let same_model = same_api && message.model == model.id;
    let different_model = same_api && message.model != model.id;
    let mut items = Vec::new();
    let mut text_index = 0usize;
    for block in &message.content {
        match block {
            AssistantBlock::Thinking(thinking) => {
                if let Some(signature) = thinking
                    .thinking_signature
                    .as_deref()
                    .filter(|signature| !signature.is_empty())
                {
                    let item: Value = serde_json::from_str(signature)
                        .map_err(|error| format!("invalid reasoning signature: {error}"))?;
                    items.push(item);
                }
            }
            AssistantBlock::Text(text) => {
                let parsed = text_signature(text.text_signature.as_deref());
                let fallback = if text_index == 0 {
                    format!("msg_pi_{msg_index}")
                } else {
                    format!("msg_pi_{msg_index}_{text_index}")
                };
                text_index += 1;
                let phase = parsed.as_ref().and_then(|(_, phase)| phase.clone());
                let mut id = parsed
                    .map(|(id, _)| id)
                    .filter(|id| !id.is_empty())
                    .unwrap_or(fallback);
                if utf16_len(&id) > ITEM_ID_MAX_CHARS {
                    id = format!("msg_{}", short_hash(&id));
                }
                let mut item = json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text.text, "annotations": []}],
                    "status": "completed",
                    "id": id,
                });
                if let Some(phase) = phase {
                    item["phase"] = json!(phase);
                }
                items.push(item);
            }
            AssistantBlock::ToolCall(call) => {
                let mut parts = call.id.split('|');
                let call_id = parts.next().unwrap_or("");
                let item_id = parts
                    .next()
                    .filter(|item_id| !different_model && item_id.starts_with("fc_"));
                let arguments = Value::Object(call.arguments.clone()).to_string();
                let mut item = json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": call.name,
                    "arguments": arguments,
                });
                if let Some(item_id) = item_id {
                    item["id"] = json!(item_id);
                }
                if same_model && let Some(namespace) = &call.namespace {
                    item["namespace"] = json!(namespace);
                }
                items.push(item);
            }
        }
    }
    Ok(items)
}

fn normalize_call_id(model: &Model, id: &str, source: &AssistantMessage) -> String {
    let mut parts = id.split('|');
    let call_id = parts.next().unwrap_or("");
    let item_id = parts.next();
    let Some(item_id) = item_id.filter(|_| TOOL_CALL_PROVIDERS.contains(&model.provider.as_str()))
    else {
        return normalize_part(id);
    };
    let foreign = source.provider != model.provider || source.api != model.api;
    let mut item_id = if foreign {
        truncate(&format!("fc_{}", short_hash(item_id)))
    } else {
        normalize_part(item_id)
    };
    if !item_id.starts_with("fc_") {
        item_id = normalize_part(&format!("fc_{item_id}"));
    }
    format!("{}|{item_id}", normalize_part(call_id))
}

fn normalize_part(part: &str) -> String {
    let sanitized: String = part
        .encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' => {
                char::from(byte)
            }
            _ => '_',
        })
        .collect();
    truncate(&sanitized).trim_end_matches('_').to_owned()
}

fn truncate(text: &str) -> String {
    text.chars().take(ITEM_ID_MAX_CHARS).collect()
}

pub fn text_signature(signature: Option<&str>) -> Option<(String, Option<String>)> {
    let signature = signature.filter(|signature| !signature.is_empty())?;
    if signature.starts_with('{')
        && let Ok(parsed) = serde_json::from_str::<Value>(signature)
        && parsed["v"].as_u64() == Some(1)
        && let Some(id) = parsed["id"].as_str()
    {
        let phase = match parsed["phase"].as_str() {
            Some(phase @ ("commentary" | "final_answer")) => Some(phase.to_owned()),
            _ => None,
        };
        return Some((id.to_owned(), phase));
    }
    Some((signature.to_owned(), None))
}

pub fn encode_text_signature(id: &str, phase: Option<&str>) -> String {
    let mut payload = json!({"v": 1, "id": id});
    if let Some(phase) = phase {
        payload["phase"] = json!(phase);
    }
    payload.to_string()
}

pub fn short_hash(text: &str) -> String {
    let mut h1: u32 = 0xdeadbeef;
    let mut h2: u32 = 0x41c6ce57;
    for unit in text.encode_utf16() {
        h1 = (h1 ^ u32::from(unit)).wrapping_mul(2654435761);
        h2 = (h2 ^ u32::from(unit)).wrapping_mul(1597334677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507) ^ (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507) ^ (h1 ^ (h1 >> 13)).wrapping_mul(3266489909);
    format!("{}{}", base36(h2), base36(h1))
}

fn base36(mut value: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.iter().rev().map(|&byte| char::from(byte)).collect()
}
