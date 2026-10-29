//! `core/tools/write.ts`.

use super::{PromptContribution, ToolContext, ToolOutput, ToolSpec, check_aborted, str_arg};
use crate::node_fs;
use crate::paths::resolve_to_cwd;
use serde_json::{Value, json};

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "write",
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.".to_owned(),
        parameters: json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": {"type": "string", "description": "Path to the file to write (relative or absolute)"},
                "content": {"type": "string", "description": "Content to write to the file"}
            }
        }),
        prompt: PromptContribution {
            snippet: "Create or overwrite files",
            guidelines: &["Use write only for new files or complete rewrites."],
        },
        prepare_arguments: None,
        constrained_sampling: super::json_schema_sampling(),
        execute,
    }
}

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    let path = str_arg(args, "path").unwrap_or_default();
    let content = str_arg(args, "content").unwrap_or_default();
    let absolute = resolve_to_cwd(path, ctx.cwd);
    let dir = std::path::Path::new(&absolute)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_owned());
    check_aborted(ctx.cancel)?;
    std::fs::create_dir_all(&dir).map_err(|e| node_fs::message(&e, "mkdir", &dir))?;
    check_aborted(ctx.cancel)?;
    std::fs::write(&absolute, content).map_err(|e| node_fs::message(&e, "open", &absolute))?;
    check_aborted(ctx.cancel)?;
    Ok(ToolOutput::text(format!("Successfully wrote to {path}")))
}
