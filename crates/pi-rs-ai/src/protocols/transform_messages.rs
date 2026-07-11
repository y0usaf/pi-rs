//! Port of `providers/transform-messages.ts` — cross-provider message
//! normalization shared by every protocol.

use std::collections::{HashMap, HashSet};

use pi_rs_ai_types::{
    AssistantContent, AssistantMessage, Message, Model, StopReason, TextContent,
    TextOrImageContent, ToolCall, ToolResultMessage, ToolResultRole, UserContent, now_ms,
};

const NON_VISION_USER_IMAGE_PLACEHOLDER: &str = "(image omitted: model does not support images)";
const NON_VISION_TOOL_IMAGE_PLACEHOLDER: &str =
    "(tool image omitted: model does not support images)";

/// Spec: the `normalizeToolCallId` callback — `(id, model, source)`.
pub type NormalizeToolCallId<'a> = &'a dyn Fn(&str, &Model, &AssistantMessage) -> String;

fn replace_images_with_placeholder(
    content: &[TextOrImageContent],
    placeholder: &str,
) -> Vec<TextOrImageContent> {
    let mut result = Vec::new();
    let mut previous_was_placeholder = false;

    for block in content {
        match block {
            TextOrImageContent::Image(_) => {
                if !previous_was_placeholder {
                    result.push(TextOrImageContent::Text(TextContent::new(placeholder)));
                }
                previous_was_placeholder = true;
            }
            TextOrImageContent::Text(text) => {
                result.push(block.clone());
                previous_was_placeholder = text.text == placeholder;
            }
        }
    }

    result
}

fn downgrade_unsupported_images(messages: &[Message], model: &Model) -> Vec<Message> {
    if model.input.contains(&pi_rs_ai_types::Modality::Image) {
        return messages.to_vec();
    }

    messages
        .iter()
        .map(|msg| match msg {
            Message::User(user) => match &user.content {
                UserContent::Blocks(blocks) => {
                    let mut user = user.clone();
                    user.content = UserContent::Blocks(replace_images_with_placeholder(
                        blocks,
                        NON_VISION_USER_IMAGE_PLACEHOLDER,
                    ));
                    Message::User(user)
                }
                UserContent::Text(_) => msg.clone(),
            },
            Message::ToolResult(tool_result) => {
                let mut tool_result = tool_result.clone();
                tool_result.content = replace_images_with_placeholder(
                    &tool_result.content,
                    NON_VISION_TOOL_IMAGE_PLACEHOLDER,
                );
                Message::ToolResult(tool_result)
            }
            Message::Assistant(_) => msg.clone(),
        })
        .collect()
}

/// Spec: `transformMessages` — image downgrade, cross-model thinking and
/// tool-call-id normalization, then synthetic tool results for orphaned
/// tool calls.
pub fn transform_messages(
    messages: &[Message],
    model: &Model,
    normalize_tool_call_id: Option<NormalizeToolCallId<'_>>,
) -> Vec<Message> {
    let mut tool_call_id_map: HashMap<String, String> = HashMap::new();
    let image_aware = downgrade_unsupported_images(messages, model);

    // First pass: per-message transformation (map order = message order,
    // so tool-result id remapping sees earlier assistant messages).
    let mut transformed: Vec<Message> = Vec::with_capacity(image_aware.len());
    for msg in &image_aware {
        match msg {
            Message::User(_) => transformed.push(msg.clone()),
            Message::ToolResult(tool_result) => {
                match tool_call_id_map.get(&tool_result.tool_call_id) {
                    Some(normalized) if *normalized != tool_result.tool_call_id => {
                        let mut tool_result = tool_result.clone();
                        tool_result.tool_call_id = normalized.clone();
                        transformed.push(Message::ToolResult(tool_result));
                    }
                    _ => transformed.push(msg.clone()),
                }
            }
            Message::Assistant(assistant) => {
                let is_same_model = assistant.provider == model.provider
                    && assistant.api == model.api
                    && assistant.model == model.id;

                let mut content: Vec<AssistantContent> = Vec::new();
                for block in &assistant.content {
                    match block {
                        AssistantContent::Thinking(thinking) => {
                            // Redacted thinking is opaque encrypted
                            // content, only valid for the same model.
                            if thinking.redacted.unwrap_or(false) {
                                if is_same_model {
                                    content.push(block.clone());
                                }
                                continue;
                            }
                            // Same model: keep signed thinking blocks
                            // even with empty text (encrypted reasoning).
                            if is_same_model
                                && thinking
                                    .thinking_signature
                                    .as_deref()
                                    .is_some_and(|s| !s.is_empty())
                            {
                                content.push(block.clone());
                                continue;
                            }
                            if thinking.thinking.trim().is_empty() {
                                continue;
                            }
                            if is_same_model {
                                content.push(block.clone());
                            } else {
                                content.push(AssistantContent::Text(TextContent::new(
                                    thinking.thinking.clone(),
                                )));
                            }
                        }
                        AssistantContent::Text(text) => {
                            if is_same_model {
                                content.push(block.clone());
                            } else {
                                // Cross-model text is rebuilt as bare
                                // `{type, text}` (drops textSignature).
                                content.push(AssistantContent::Text(TextContent::new(
                                    text.text.clone(),
                                )));
                            }
                        }
                        AssistantContent::ToolCall(tool_call) => {
                            let mut tool_call = tool_call.clone();
                            if !is_same_model && tool_call.thought_signature.is_some() {
                                tool_call.thought_signature = None;
                            }
                            if !is_same_model && let Some(normalize) = normalize_tool_call_id {
                                let normalized = normalize(&tool_call.id, model, assistant);
                                if normalized != tool_call.id {
                                    tool_call_id_map
                                        .insert(tool_call.id.clone(), normalized.clone());
                                    tool_call.id = normalized;
                                }
                            }
                            content.push(AssistantContent::ToolCall(tool_call));
                        }
                    }
                }

                let mut assistant = assistant.clone();
                assistant.content = content;
                transformed.push(Message::Assistant(assistant));
            }
        }
    }

    // Second pass: synthetic empty tool results for orphaned tool calls.
    let mut result: Vec<Message> = Vec::new();
    let mut pending_tool_calls: Vec<ToolCall> = Vec::new();
    let mut existing_tool_result_ids: HashSet<String> = HashSet::new();

    fn insert_synthetic_tool_results(
        result: &mut Vec<Message>,
        pending: &mut Vec<ToolCall>,
        existing: &mut HashSet<String>,
    ) {
        if pending.is_empty() {
            return;
        }
        for tool_call in pending.iter() {
            if !existing.contains(&tool_call.id) {
                result.push(Message::ToolResult(ToolResultMessage {
                    role: ToolResultRole::ToolResult,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    content: vec![TextOrImageContent::Text(TextContent::new(
                        "No result provided",
                    ))],
                    details: None,
                    is_error: true,
                    timestamp: now_ms(),
                }));
            }
        }
        pending.clear();
        existing.clear();
    }

    for msg in &transformed {
        match msg {
            Message::Assistant(assistant) => {
                insert_synthetic_tool_results(
                    &mut result,
                    &mut pending_tool_calls,
                    &mut existing_tool_result_ids,
                );

                // Skip errored/aborted assistant messages entirely:
                // incomplete turns that must not be replayed.
                if matches!(
                    assistant.stop_reason,
                    StopReason::Error | StopReason::Aborted
                ) {
                    continue;
                }

                let tool_calls: Vec<ToolCall> = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AssistantContent::ToolCall(tool_call) => Some(tool_call.clone()),
                        _ => None,
                    })
                    .collect();
                if !tool_calls.is_empty() {
                    pending_tool_calls = tool_calls;
                    existing_tool_result_ids.clear();
                }

                result.push(msg.clone());
            }
            Message::ToolResult(tool_result) => {
                existing_tool_result_ids.insert(tool_result.tool_call_id.clone());
                result.push(msg.clone());
            }
            Message::User(_) => {
                // User message interrupts the tool flow.
                insert_synthetic_tool_results(
                    &mut result,
                    &mut pending_tool_calls,
                    &mut existing_tool_result_ids,
                );
                result.push(msg.clone());
            }
        }
    }

    // Unresolved tool calls at conversation end.
    insert_synthetic_tool_results(
        &mut result,
        &mut pending_tool_calls,
        &mut existing_tool_result_ids,
    );

    result
}
