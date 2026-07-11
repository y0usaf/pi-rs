//! Port of `providers/github-copilot-headers.ts`.

use pi_rs_ai_types::{Message, TextOrImageContent, UserContent};

/// Spec: `inferCopilotInitiator` — `X-Initiator` is "agent" when the
/// last message is not a user message.
pub fn infer_copilot_initiator(messages: &[Message]) -> &'static str {
    match messages.last() {
        Some(Message::User(_)) | None => "user",
        Some(_) => "agent",
    }
}

/// Spec: `hasCopilotVisionInput` — any image in user (block-form) or
/// tool-result content.
pub fn has_copilot_vision_input(messages: &[Message]) -> bool {
    messages.iter().any(|msg| match msg {
        Message::User(user) => match &user.content {
            UserContent::Blocks(blocks) => has_image(blocks),
            UserContent::Text(_) => false,
        },
        Message::ToolResult(tool_result) => has_image(&tool_result.content),
        Message::Assistant(_) => false,
    })
}

fn has_image(blocks: &[TextOrImageContent]) -> bool {
    blocks
        .iter()
        .any(|block| matches!(block, TextOrImageContent::Image(_)))
}

/// Spec: `buildCopilotDynamicHeaders`.
pub fn build_copilot_dynamic_headers(
    messages: &[Message],
    has_images: bool,
) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "X-Initiator".to_string(),
            infer_copilot_initiator(messages).to_string(),
        ),
        (
            "Openai-Intent".to_string(),
            "conversation-edits".to_string(),
        ),
    ];
    if has_images {
        headers.push(("Copilot-Vision-Request".to_string(), "true".to_string()));
    }
    headers
}
