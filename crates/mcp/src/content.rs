//! `toLlmContent`: a tool result's content blocks as the text and images a model takes.

use crate::protocol::{CallToolResult, ContentBlock, EmbeddedResource};

#[derive(Debug, Clone, PartialEq)]
pub enum LlmContent {
    Text(String),
    Image { data: String, mime_type: String },
}

fn resource_to_llm_content(resource: &EmbeddedResource) -> LlmContent {
    if let Some(text) = &resource.text {
        return LlmContent::Text(text.clone());
    }
    let mime_type = resource.mime_type.clone();
    if let (Some(blob), Some(mime_type)) = (&resource.blob, mime_type.as_deref())
        && mime_type.starts_with("image/")
    {
        return LlmContent::Image {
            data: blob.clone(),
            mime_type: mime_type.to_owned(),
        };
    }
    LlmContent::Text(format!(
        "[binary resource {} ({}) omitted]",
        resource.uri,
        mime_type.as_deref().unwrap_or("unknown type")
    ))
}

fn block_to_llm_content(block: &ContentBlock) -> LlmContent {
    match block {
        ContentBlock::Text { text } => LlmContent::Text(text.clone()),
        ContentBlock::Image { data, mime_type } => LlmContent::Image {
            data: data.clone(),
            mime_type: mime_type.clone(),
        },
        ContentBlock::Audio { mime_type, .. } => {
            LlmContent::Text(format!("[audio {mime_type} omitted]"))
        }
        ContentBlock::ResourceLink { name, uri, .. } => LlmContent::Text(format!("{name}: {uri}")),
        ContentBlock::Resource { resource } => resource_to_llm_content(resource),
        ContentBlock::Other { kind } => {
            LlmContent::Text(format!("[unsupported MCP content {kind}]"))
        }
    }
}

/// A result's content blocks as model content. A result without content blocks but with
/// `structuredContent` becomes its JSON, since servers should, but do not always, mirror
/// structured results as text.
pub fn to_llm_content(result: &CallToolResult) -> Vec<LlmContent> {
    let mut content: Vec<LlmContent> = result.content.iter().map(block_to_llm_content).collect();
    if content.is_empty()
        && let Some(structured) = &result.structured_content
    {
        content.push(LlmContent::Text(
            serde_json::to_string_pretty(structured).unwrap_or_default(),
        ));
    }
    content
}
