//! `extensions/mcp/tools.ts`: a server tool as one of the agent's own, named
//! `mcp__<server>__<tool>`, and its result as the text and images a model takes.

use std::sync::Arc;
use std::time::Duration;

use pi_durable::ai::types::{
    ImageContent, ImageType, TextContent, TextType, Tool as AgentTool, UserBlock,
};
use pi_durable::harness::types::{
    OutputLimitsSpec, ToolExecute, ToolExecutionResult, ToolRegistration,
};
use pi_mcp::content::{LlmContent, to_llm_content};
use pi_mcp::protocol::{CallToolResult, ContentBlock, Tool};
use serde_json::{Map, Value};

use crate::mcp::runtime::Connection;
use crate::tools::truncate::format_size;

/// Provider tool names are limited to 64 characters of `[A-Za-z0-9_-]`.
const MAX_TOOL_NAME_LENGTH: usize = 64;
/// Model-facing text of an MCP result beyond this is cut in the middle.
pub const MCP_OUTPUT_MAX_BYTES: usize = 20 * 1024;

/// `mcp__<server>__<tool>`, sanitized and shortened with a hash suffix when too long. Everything
/// but `[A-Za-z0-9_]` becomes `_`, as Codex does. `is_taken` reports names used by a different
/// MCP tool: sanitizing can map two tools to one name (`a-b` and `a_b`), which then get the suffix.
pub fn tool_name(server: &str, tool: &str, is_taken: &dyn Fn(&str) -> bool) -> String {
    let name: String = format!("mcp__{server}__{tool}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.len() <= MAX_TOOL_NAME_LENGTH && !is_taken(&name) {
        return name;
    }
    let digest = ring::digest::digest(
        &ring::digest::SHA256,
        format!("{server}\0{tool}").as_bytes(),
    );
    let hash: String = digest.as_ref()[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let keep = MAX_TOOL_NAME_LENGTH - hash.len() - 1;
    let mut truncated: String = name.chars().take(keep).collect();
    truncated.push('_');
    truncated.push_str(&hash);
    truncated
}

/// Tool input schemas must be objects. MCP servers may omit `type`, and some providers reject
/// object schemas without `properties`.
fn parameters(schema: &Value) -> Value {
    let mut object = schema.as_object().cloned().unwrap_or_default();
    object
        .entry("type")
        .or_insert_with(|| Value::from("object"));
    object
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    Value::Object(object)
}

fn text_block(text: impl Into<String>) -> UserBlock {
    UserBlock::Text(TextContent {
        kind: TextType,
        text: text.into(),
        text_signature: None,
    })
}

fn image_block(data: String, mime_type: &str) -> UserBlock {
    UserBlock::Image(ImageContent {
        kind: ImageType,
        data,
        mime_type: mime_type.to_owned(),
    })
}

/// File extension for a saved binary resource: the one its URI ends in, else `.bin`.
fn extension_of(uri: &str) -> String {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let tail = path.rsplit('/').next().unwrap_or(path);
    match tail.rsplit_once('.') {
        Some((_, extension))
            if !extension.is_empty()
                && extension.len() <= 8
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            format!(".{extension}")
        }
        _ => ".bin".into(),
    }
}

/// Blobs of these types are shown as text.
fn is_text_mime_type(mime_type: Option<&str>) -> bool {
    let Some(mime_type) = mime_type else {
        return false;
    };
    let kind = mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    kind.starts_with("text/")
        || kind == "application/json"
        || kind.ends_with("+json")
        || kind.ends_with("+xml")
}

/// `utils/output-files.ts`: the full text of a truncated result, or a binary resource, in a file
/// only this user can read.
pub fn save_output(data: &[u8], extension: &str) -> Result<String, String> {
    use std::io::Write;
    let path = crate::output_files::new_path("pi-mcp", extension);
    let mut file = crate::output_files::create(&path).map_err(|error| error.to_string())?;
    file.write_all(data).map_err(|error| error.to_string())?;
    Ok(path)
}

fn resource_block(block: &ContentBlock, readable_resources: bool) -> Vec<UserBlock> {
    match block {
        ContentBlock::ResourceLink {
            uri,
            name,
            title,
            description,
            mime_type,
            size,
        } => {
            let details: Vec<String> = [
                mime_type.clone(),
                size.map(|size| format_size(size as usize)),
            ]
            .into_iter()
            .flatten()
            .collect();
            let read = if readable_resources {
                format!(
                    ". Read it with {}",
                    crate::mcp::config::READ_MCP_RESOURCE_TOOL
                )
            } else {
                String::new()
            };
            let details = if details.is_empty() {
                String::new()
            } else {
                format!(" ({})", details.join(", "))
            };
            let description = description
                .as_ref()
                .map(|description| format!(": {description}"))
                .unwrap_or_default();
            vec![text_block(format!(
                "[Resource {uri} \"{}\"{details}{description}{read}]",
                title.as_deref().unwrap_or(name)
            ))]
        }
        ContentBlock::Resource { resource } => {
            if let Some(text) = &resource.text {
                return vec![text_block(text.clone())];
            }
            let Some(blob) = &resource.blob else {
                return Vec::new();
            };
            let mime_type = resource.mime_type.clone();
            if is_text_mime_type(mime_type.as_deref()) {
                return vec![text_block(
                    String::from_utf8_lossy(&base64_decode(blob)).into_owned(),
                )];
            }
            let data = base64_decode(blob);
            let kind = format!(
                "{}, {}",
                mime_type.as_deref().unwrap_or("unknown type"),
                format_size(data.len())
            );
            match save_output(&data, &extension_of(&resource.uri)) {
                Ok(path) => vec![text_block(format!(
                    "[Binary resource {} ({kind}) saved to {path}]",
                    resource.uri
                ))],
                Err(error) => vec![text_block(format!(
                    "[Binary resource {} ({kind}) could not be saved: {error}]",
                    resource.uri
                ))],
            }
        }
        other => llm_blocks(std::slice::from_ref(other)),
    }
}

fn base64_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return out,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    let _ = ALPHABET;
    out
}

fn llm_blocks(blocks: &[ContentBlock]) -> Vec<UserBlock> {
    to_llm_content(&CallToolResult {
        content: blocks.to_vec(),
        structured_content: None,
        is_error: false,
    })
    .into_iter()
    .map(|content| match content {
        LlmContent::Text(text) => text_block(text),
        LlmContent::Image { data, mime_type } => image_block(data, &mime_type),
    })
    .collect()
}

fn text_of(content: &[UserBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            UserBlock::Text(text) => Some(text.text.as_str()),
            UserBlock::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count().max(1)
    }
}

/// `truncateMiddle`: the start and the end of `text`, with what they leave out counted in the
/// middle.
fn truncate_middle(text: &str, max_bytes: usize) -> Option<(String, usize, usize)> {
    if text.len() <= max_bytes {
        return None;
    }
    let mut head_end = max_bytes / 2;
    while head_end > 0 && !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len() - (max_bytes - max_bytes / 2);
    while tail_start < text.len() && !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let removed = text[head_end..tail_start].chars().count();
    Some((
        format!(
            "{}…{removed} chars truncated…{}",
            &text[..head_end],
            &text[tail_start..]
        ),
        line_count(text),
        removed,
    ))
}

/// Keep the model-facing text within {@link MCP_OUTPUT_MAX_BYTES}, saving the whole text when it is
/// cut.
fn limit_output(content: Vec<UserBlock>) -> (Vec<UserBlock>, Option<String>) {
    let combined = text_of(&content);
    let Some((truncated, lines, _removed)) = truncate_middle(&combined, MCP_OUTPUT_MAX_BYTES)
    else {
        return (content, None);
    };
    let (path, where_) = match save_output(combined.as_bytes(), ".txt") {
        Ok(path) => {
            let where_ = format!("[Full output: {path} (read it with offset/limit)]");
            (Some(path), where_)
        }
        Err(error) => (None, format!("[Could not save the full output: {error}]")),
    };
    let tokens = combined.len().div_ceil(4);
    let text = format!(
        "Warning: truncated output (original token count: {tokens})\nTotal output lines: {lines}\n\n{truncated}\n\n{where_}"
    );
    let mut limited = vec![text_block(text)];
    limited.extend(
        content
            .into_iter()
            .filter(|block| matches!(block, UserBlock::Image(_))),
    );
    (limited, path)
}

fn convert_result(
    server: &str,
    tool: &str,
    result: CallToolResult,
    readable_resources: bool,
) -> ToolExecutionResult {
    let mut content: Vec<UserBlock> = Vec::new();
    for block in &result.content {
        content.extend(resource_block(block, readable_resources));
    }
    if content.is_empty() {
        content.extend(
            to_llm_content(&result)
                .into_iter()
                .map(|content| match content {
                    LlmContent::Text(text) => text_block(text),
                    LlmContent::Image { data, mime_type } => image_block(data, &mime_type),
                }),
        );
    }
    if result.is_error && text_of(&content).is_empty() {
        content.push(text_block(format!(
            "MCP tool {server}/{tool} returned an error"
        )));
    }
    let (content, full_output_path) = limit_output(content);
    let mut details = Map::new();
    details.insert("server".into(), Value::from(server));
    details.insert("tool".into(), Value::from(tool));
    if let Some(path) = full_output_path {
        details.insert("fullOutputPath".into(), Value::from(path));
    }
    ToolExecutionResult {
        content: Some(content),
        is_error: Some(result.is_error),
        details: Some(Value::Object(details)),
        ..ToolExecutionResult::default()
    }
}

/// One of the server's tools as the agent's own.
pub fn registration(
    connection: Arc<Connection>,
    server: &str,
    tool: &Tool,
    name: String,
    timeout: Duration,
    readable_resources: bool,
) -> ToolRegistration {
    let title = tool.title.clone();
    let description = tool
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_owned)
        .or(title)
        .unwrap_or_else(|| format!("MCP tool {} from server {server}", tool.name));
    let tool_name = tool.name.clone();
    let server_name = server.to_owned();
    let execute: ToolExecute = Arc::new(move |args, _api, cancel| {
        if cancel.is_cancelled() {
            return Err(pi_durable::errors::Error::Aborted);
        }
        let arguments = match args {
            Value::Null => None,
            args => Some(args),
        };
        let result = connection.call_tool(&tool_name, arguments, timeout);
        Ok(match result {
            Ok(result) => convert_result(&server_name, &tool_name, result, readable_resources),
            Err(message) => ToolExecutionResult {
                content: Some(vec![text_block(message)]),
                is_error: Some(true),
                details: Some(Value::Object(Map::new())),
                ..ToolExecutionResult::default()
            },
        })
    });
    ToolRegistration {
        tool: AgentTool {
            name,
            description,
            parameters: parameters(&tool.input_schema),
            constrained_sampling: None,
        },
        replay: None,
        execution_mode: None,
        output_limits: Some(OutputLimitsSpec {
            max_bytes: Some(usize::MAX),
            max_lines: Some(usize::MAX),
            retain: None,
        }),
        prepare_arguments: None,
        exposure: pi_durable::harness::types::ToolExposure::Direct,
        hidden: false,
        execute,
    }
}
