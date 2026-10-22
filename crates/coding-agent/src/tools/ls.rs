//! `core/tools/ls.ts`.

use super::truncate::{DEFAULT_MAX_BYTES, Limits, format_size, truncate_head};
use super::{
    PromptContribution, ToolContext, ToolOutput, ToolSpec, check_aborted, js_number, num_arg,
    str_arg,
};
use crate::collate::locale_compare;
use crate::node_fs;
use crate::paths::{join, resolve_to_cwd};
use serde_json::{Map, Value, json};

const DEFAULT_LIMIT: f64 = 500.0;

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "ls",
        description: format!(
            "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to {} entries or {}KB (whichever is hit first).",
            DEFAULT_LIMIT,
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory to list (default: current directory)"},
                "limit": {"type": "number", "description": "Maximum number of entries to return (default: 500)"}
            }
        }),
        prompt: PromptContribution {
            snippet: "List directory contents",
            guidelines: &[],
        },
        prepare_arguments: None,
        execute,
    }
}

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    check_aborted(ctx.cancel)?;
    let path = str_arg(args, "path")
        .filter(|p| !p.is_empty())
        .unwrap_or(".");
    let dir = resolve_to_cwd(path, ctx.cwd);
    let limit = num_arg(args, "limit").unwrap_or(DEFAULT_LIMIT);
    let meta = match std::fs::metadata(&dir) {
        Ok(meta) => meta,
        Err(e) if matches!(node_fs::code(&e), "ENOENT" | "ENOTDIR") => {
            return Err(format!("Path not found: {dir}"));
        }
        Err(e) => return Err(node_fs::message(&e, "stat", &dir)),
    };
    if !meta.is_dir() {
        return Err(format!("Not a directory: {dir}"));
    }
    let mut entries: Vec<String> = std::fs::read_dir(&dir)
        .and_then(|it| {
            it.map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect()
        })
        .map_err(|e| {
            format!(
                "Cannot read directory: {}",
                node_fs::message(&e, "scandir", &dir)
            )
        })?;
    entries.sort_by(|a, b| locale_compare(&a.to_lowercase(), &b.to_lowercase()));
    let mut results: Vec<String> = Vec::new();
    let mut limit_reached = false;
    for entry in entries {
        if results.len() as f64 >= limit {
            limit_reached = true;
            break;
        }
        let Ok(meta) = std::fs::metadata(join(&dir, &entry)) else {
            continue;
        };
        results.push(if meta.is_dir() {
            format!("{entry}/")
        } else {
            entry
        });
    }
    if results.is_empty() {
        return Ok(ToolOutput::text("(empty directory)"));
    }
    let truncation = truncate_head(
        &results.join("\n"),
        Limits {
            max_lines: Some(usize::MAX),
            max_bytes: None,
        },
    );
    let mut output = truncation.content.clone();
    let mut details = Map::new();
    let mut notices = Vec::new();
    if limit_reached {
        notices.push(format!(
            "{} entries limit reached. Use limit={} for more",
            js_number(limit),
            js_number(limit * 2.0)
        ));
        details.insert("entryLimitReached".to_owned(), json!(limit));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert("truncation".to_owned(), json!(truncation));
    }
    if !notices.is_empty() {
        output = format!("{output}\n\n[{}]", notices.join(". "));
    }
    let out = ToolOutput::text(output);
    Ok(if details.is_empty() {
        out
    } else {
        out.with_details(Value::Object(details))
    })
}
