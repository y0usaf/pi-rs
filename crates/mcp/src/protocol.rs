//! `protocol/types.ts` and `protocol/content.ts`: the protocol's result shapes, parsed from and
//! written as JSON.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] = [
    LATEST_PROTOCOL_VERSION,
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Implementation {
    pub name: String,
    pub version: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Root {
    pub uri: String,
    pub name: Option<String>,
}

impl Root {
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("uri".into(), Value::from(self.uri.clone()));
        if let Some(name) = &self.name {
            object.insert("name".into(), Value::from(name.clone()));
        }
        Value::Object(object)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: Value,
    pub server_info: Implementation,
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Annotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

impl Annotations {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.read_only_hint.is_none()
            && self.destructive_hint.is_none()
            && self.idempotent_hint.is_none()
            && self.open_world_hint.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub annotations: Option<Annotations>,
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn bool_field(object: &Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(Value::as_bool)
}

fn parse_annotations(value: Option<&Value>) -> Option<Annotations> {
    let object = value?.as_object()?;
    let annotations = Annotations {
        title: string_field(object, "title"),
        read_only_hint: bool_field(object, "readOnlyHint"),
        destructive_hint: bool_field(object, "destructiveHint"),
        idempotent_hint: bool_field(object, "idempotentHint"),
        open_world_hint: bool_field(object, "openWorldHint"),
    };
    (!annotations.is_empty()).then_some(annotations)
}

fn parse_tool(value: &Value) -> Option<Tool> {
    let object = value.as_object()?;
    let input_schema = object.get("inputSchema")?.as_object()?;
    Some(Tool {
        name: string_field(object, "name")?,
        title: string_field(object, "title"),
        description: string_field(object, "description"),
        input_schema: Value::Object(input_schema.clone()),
        output_schema: object
            .get("outputSchema")
            .and_then(Value::as_object)
            .map(|schema| Value::Object(schema.clone())),
        annotations: parse_annotations(object.get("annotations")),
    })
}

/// One page of `tools/list`: the tools and the cursor of the next page, if the server has more.
/// Some servers end pagination with `null` or `""` instead of omitting the cursor.
pub fn parse_tools_page(value: &Value) -> Result<(Vec<Tool>, Option<String>)> {
    let invalid = || Error::invalid("Invalid MCP tools/list result");
    let object = value.as_object().ok_or_else(invalid)?;
    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut parsed = Vec::with_capacity(tools.len());
    for tool in tools {
        parsed.push(
            parse_tool(tool)
                .ok_or_else(|| Error::invalid("Invalid entry in MCP tools/list result"))?,
        );
    }
    Ok((parsed, next_cursor(object)?))
}

fn next_cursor(object: &Map<String, Value>) -> Result<Option<String>> {
    match object.get("nextCursor") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) if cursor.is_empty() => Ok(None),
        Some(Value::String(cursor)) => Ok(Some(cursor.clone())),
        Some(_) => Err(Error::invalid("Invalid MCP tools/list cursor")),
    }
}

pub fn parse_initialize(value: &Value) -> Result<InitializeResult> {
    let invalid = || Error::invalid("Invalid MCP initialize result");
    let object = value.as_object().ok_or_else(invalid)?;
    let protocol_version = object
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let capabilities = object
        .get("capabilities")
        .filter(|value| value.is_object())
        .ok_or_else(invalid)?;
    let server_info = object
        .get("serverInfo")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let name = server_info
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let version = server_info
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let instructions = match object.get("instructions") {
        None => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => return Err(invalid()),
    };
    Ok(InitializeResult {
        protocol_version: protocol_version.into(),
        capabilities: capabilities.clone(),
        server_info: Implementation {
            name: name.into(),
            version: version.into(),
            title: string_field(server_info, "title"),
        },
        instructions,
    })
}

/// Whether the server initialized with `capabilities.tools` set, which is when it answers
/// `tools/list`.
pub fn has_tools(capabilities: &Value) -> bool {
    capabilities.get("tools").is_some()
}

pub fn has_resources(capabilities: &Value) -> bool {
    capabilities.get("resources").is_some()
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    ResourceLink {
        uri: String,
        name: String,
        title: Option<String>,
        description: Option<String>,
        mime_type: Option<String>,
        size: Option<u64>,
    },
    Resource {
        resource: EmbeddedResource,
    },
    Other {
        kind: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedResource {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob: Option<String>,
}

fn parse_content_block(value: &Value) -> ContentBlock {
    let Some(object) = value.as_object() else {
        return ContentBlock::Other {
            kind: String::new(),
        };
    };
    let text = |key: &str| string_field(object, key);
    match text("type").as_deref() {
        Some("text") => ContentBlock::Text {
            text: text("text").unwrap_or_default(),
        },
        Some("image") => ContentBlock::Image {
            data: text("data").unwrap_or_default(),
            mime_type: text("mimeType").unwrap_or_default(),
        },
        Some("audio") => ContentBlock::Audio {
            data: text("data").unwrap_or_default(),
            mime_type: text("mimeType").unwrap_or_default(),
        },
        Some("resource_link") => ContentBlock::ResourceLink {
            uri: text("uri").unwrap_or_default(),
            name: text("name").unwrap_or_default(),
            title: text("title"),
            description: text("description"),
            mime_type: text("mimeType"),
            size: object.get("size").and_then(Value::as_u64),
        },
        Some("resource") => {
            let resource = object
                .get("resource")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            ContentBlock::Resource {
                resource: EmbeddedResource {
                    uri: string_field(&resource, "uri").unwrap_or_default(),
                    mime_type: string_field(&resource, "mimeType"),
                    text: string_field(&resource, "text"),
                    blob: string_field(&resource, "blob"),
                },
            }
        }
        Some(kind) => ContentBlock::Other {
            kind: kind.to_owned(),
        },
        None => ContentBlock::Other {
            kind: String::new(),
        },
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
}

/// `content` is required by the spec, but servers that only return `structuredContent` omit it.
pub fn parse_call_tool_result(value: &Value) -> Result<CallToolResult> {
    let invalid = || Error::invalid("Invalid MCP tools/call result");
    let object = value.as_object().ok_or_else(invalid)?;
    let content = match object.get("content") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(blocks)) => blocks.iter().map(parse_content_block).collect(),
        Some(_) => return Err(invalid()),
    };
    Ok(CallToolResult {
        content,
        structured_content: object.get("structuredContent").cloned(),
        is_error: object
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}
