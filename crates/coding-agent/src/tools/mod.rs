//! The built-in tools, with upstream's names, schemas, descriptions and output text
//! (`packages/coding-agent/src/core/tools`).
//!
//! Each tool is a [`ToolSpec`]: what the model is told about it, and an `execute` that runs it
//! in a working directory. `Ok` is a result; `Err(message)` is a thrown error, which the agent
//! shows the model as an error result holding only `message`, as upstream's agent loop does.

use pi_durable::ai::types::{ImageContent, ImageType, TextContent, TextType, UserBlock};
use pi_durable::cancel::CancelToken;
use serde_json::Value;

pub mod bash;
pub mod edit;
mod edit_diff;

mod jsdiff;
pub mod ls;
mod mime;
pub mod read;
pub mod truncate;
pub mod write;

/// What a tool contributes to the system prompt (`*ToolSystemPromptContribution`).
pub struct PromptContribution {
    pub snippet: &'static str,
    pub guidelines: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub content: Vec<UserBlock>,
    pub details: Option<Value>,
    /// A result the model sees as failed without the tool throwing (bash's non-zero exit).
    pub is_error: bool,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![text_block(text)],
            details: None,
            is_error: false,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

pub fn text_block(text: impl Into<String>) -> UserBlock {
    UserBlock::Text(TextContent {
        kind: TextType,
        text: text.into(),
        text_signature: None,
    })
}

pub fn image_block(data: String, mime_type: &str) -> UserBlock {
    UserBlock::Image(ImageContent {
        kind: ImageType,
        data,
        mime_type: mime_type.to_owned(),
    })
}

/// Where a tool runs, and what it may stream as it goes (bash's partial output).
pub struct ToolContext<'a> {
    pub cwd: &'a str,
    pub cancel: &'a CancelToken,
    /// Whether the model takes images; read adds a note when it does not.
    pub model_takes_images: bool,
    /// `shellPath` from settings.
    pub shell_path: Option<&'a str>,
    /// Variables set for bash commands (`PI_SESSION_ID`, `PI_MODEL`, ..., and PATH with the agent's
    /// bin directory).
    pub env: &'a [(String, String)],
    /// Called with bash's output so far (the tail it would show), throttled.
    pub on_update: Option<&'a mut dyn FnMut(&str)>,
}

pub type Execute = fn(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String>;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: String,
    /// JSON schema, keys in typebox's order (`type`, `required`, `properties`).
    pub parameters: Value,
    pub prompt: PromptContribution,
    /// Repairs malformed arguments before validation (`prepareArguments`).
    pub prepare_arguments: Option<fn(Value) -> Value>,
    pub execute: Execute,
}

/// Upstream's `allToolNames` minus `powershell` (Windows only).
pub const ALL_TOOL_NAMES: [&str; 7] = ["read", "bash", "edit", "write", "grep", "find", "ls"];

/// The tools on when none are named: `read`, `bash`, `edit`, `write`.
pub const DEFAULT_TOOL_NAMES: [&str; 4] = ["read", "bash", "edit", "write"];

/// The spec of a built-in tool, or None for an unknown name.
pub fn spec(name: &str) -> Option<ToolSpec> {
    match name {
        "read" => Some(read::spec()),
        "write" => Some(write::spec()),
        "edit" => Some(edit::spec()),
        "bash" => Some(bash::spec()),

        "ls" => Some(ls::spec()),
        _ => None,
    }
}

pub(crate) fn check_aborted(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("Operation aborted".to_owned())
    } else {
        Ok(())
    }
}

/// A string argument, after validation has checked it is one.
pub(crate) fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub(crate) fn num_arg(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

/// A JavaScript number as `${n}` prints it, for whole numbers and plain fractions.
pub(crate) fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}
