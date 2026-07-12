//! Shared wire conversion for Google Generative AI and Vertex.
//! Port of `providers/google-shared.ts`.

use pi_rs_ai_types::{
    AssistantContent, Context, Message, Modality, Model, TextOrImageContent, Tool, UserContent,
};
use serde_json::{Map, Value, json};

use super::transform_messages::transform_messages;
use crate::util::sanitize_surrogates;

pub(crate) fn requires_tool_call_id(model_id: &str) -> bool {
    model_id.starts_with("claude-") || model_id.starts_with("gpt-oss-")
}

fn valid_signature(signature: Option<&str>) -> Option<&str> {
    let signature = signature.filter(|value| !value.is_empty() && value.len() % 4 == 0)?;
    let padding = signature
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    (padding <= 2
        && signature[..signature.len() - padding]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/')))
    .then_some(signature)
}

fn gemini_major(model_id: &str) -> Option<u32> {
    let lower = model_id.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("gemini-live-")
        .or_else(|| lower.strip_prefix("gemini-"))?;
    rest.split(|ch: char| !ch.is_ascii_digit())
        .next()
        .and_then(|value| value.parse().ok())
}

pub(crate) fn convert_messages(model: &Model, context: &Context) -> Vec<Value> {
    let normalize = |id: &str, _: &Model, _: &pi_rs_ai_types::AssistantMessage| {
        if !requires_tool_call_id(&model.id) {
            return id.to_string();
        }
        id.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                    ch
                } else {
                    '_'
                }
            })
            .take(64)
            .collect()
    };
    let messages = transform_messages(&context.messages, model, Some(&normalize));
    let mut contents: Vec<Value> = Vec::new();
    for message in messages {
        match message {
            Message::User(user) => {
                let parts = match user.content {
                    UserContent::Text(text) => vec![json!({"text": sanitize_surrogates(&text)})],
                    UserContent::Blocks(blocks) => blocks
                        .into_iter()
                        .map(|block| match block {
                            TextOrImageContent::Text(text) => {
                                json!({"text": sanitize_surrogates(&text.text)})
                            }
                            TextOrImageContent::Image(image) => {
                                json!({"inlineData":{"mimeType":image.mime_type,"data":image.data}})
                            }
                        })
                        .collect(),
                };
                if !parts.is_empty() {
                    contents.push(json!({"role":"user","parts":parts}));
                }
            }
            Message::Assistant(message) => {
                let same = message.provider == model.provider && message.model == model.id;
                let mut parts = Vec::new();
                for block in message.content {
                    match block {
                        AssistantContent::Text(text) if !text.text.trim().is_empty() => {
                            let mut part = json!({"text":sanitize_surrogates(&text.text)});
                            if same
                                && let Some(signature) =
                                    valid_signature(text.text_signature.as_deref())
                            {
                                part["thoughtSignature"] = json!(signature);
                            }
                            parts.push(part);
                        }
                        AssistantContent::Thinking(thinking)
                            if !thinking.thinking.trim().is_empty() =>
                        {
                            let mut part = json!({"text":sanitize_surrogates(&thinking.thinking)});
                            if same {
                                part["thought"] = json!(true);
                                if let Some(signature) =
                                    valid_signature(thinking.thinking_signature.as_deref())
                                {
                                    part["thoughtSignature"] = json!(signature);
                                }
                            }
                            parts.push(part);
                        }
                        AssistantContent::ToolCall(call) => {
                            let mut function = json!({"name":call.name,"args":call.arguments});
                            if requires_tool_call_id(&model.id) {
                                function["id"] = json!(call.id);
                            }
                            let mut part = json!({"functionCall":function});
                            if same
                                && let Some(signature) =
                                    valid_signature(call.thought_signature.as_deref())
                            {
                                part["thoughtSignature"] = json!(signature);
                            }
                            parts.push(part);
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role":"model","parts":parts}));
                }
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
                let images = if model.input.contains(&Modality::Image) {
                    result.content.iter().filter_map(|block| match block {
                        TextOrImageContent::Image(image) => Some(json!({"inlineData":{"mimeType":image.mime_type,"data":image.data}})),
                        TextOrImageContent::Text(_) => None,
                    }).collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let response = if !text.is_empty() {
                    text
                } else if !images.is_empty() {
                    "(see attached image)".to_string()
                } else {
                    String::new()
                };
                let mut function = Map::new();
                function.insert("name".into(), json!(result.tool_name));
                function.insert(
                    "response".into(),
                    if result.is_error {
                        json!({"error":sanitize_surrogates(&response)})
                    } else {
                        json!({"output":sanitize_surrogates(&response)})
                    },
                );
                let nested = gemini_major(&model.id).is_none_or(|major| major >= 3);
                if !images.is_empty() && nested {
                    function.insert("parts".into(), Value::Array(images.clone()));
                }
                if requires_tool_call_id(&model.id) {
                    function.insert("id".into(), json!(result.tool_call_id));
                }
                let part = json!({"functionResponse":function});
                let mut merged = false;
                if let Some(last) = contents.last_mut()
                    && last["role"] == "user"
                    && last["parts"].as_array().is_some_and(|parts| {
                        parts
                            .iter()
                            .any(|part| part.get("functionResponse").is_some())
                    })
                    && let Some(parts) = last["parts"].as_array_mut()
                {
                    parts.push(part.clone());
                    merged = true;
                }
                if !merged {
                    contents.push(json!({"role":"user","parts":[part]}));
                }
                if !images.is_empty() && !nested {
                    let mut parts = vec![json!({"text":"Tool result image:"})];
                    parts.extend(images);
                    contents.push(json!({"role":"user","parts":parts}));
                }
            }
        }
    }
    contents
}

pub(crate) fn convert_tools(tools: &[Tool], use_parameters: bool) -> Option<Value> {
    if tools.is_empty() {
        return None;
    }
    fn sanitize(value: &Value) -> Value {
        const META: &[&str] = &[
            "$schema",
            "$id",
            "$anchor",
            "$dynamicAnchor",
            "$vocabulary",
            "$comment",
            "$defs",
            "definitions",
        ];
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(key, _)| !META.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), sanitize(value)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(sanitize).collect()),
            _ => value.clone(),
        }
    }
    Some(Value::Array(vec![
        json!({"functionDeclarations":tools.iter().map(|tool| {
        let mut value = json!({"name":tool.name,"description":tool.description});
        value[if use_parameters {"parameters"} else {"parametersJsonSchema"}] = if use_parameters { sanitize(&tool.parameters) } else { tool.parameters.clone() };
        value
    }).collect::<Vec<_>>()}),
    ]))
}

pub(crate) fn map_stop_reason(reason: &str) -> pi_rs_ai_types::StopReason {
    match reason {
        "STOP" => pi_rs_ai_types::StopReason::Stop,
        "MAX_TOKENS" => pi_rs_ai_types::StopReason::Length,
        _ => pi_rs_ai_types::StopReason::Error,
    }
}
