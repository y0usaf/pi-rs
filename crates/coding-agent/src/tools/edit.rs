//! `core/tools/edit.ts`.

use super::edit_diff::{
    Edit, apply_edits_to_normalized_content, detect_line_ending, generate_diff_string,
    generate_unified_patch, normalize_to_lf, restore_line_endings, strip_bom,
};
use super::{PromptContribution, ToolContext, ToolOutput, ToolSpec, check_aborted, str_arg};
use crate::node_fs;
use crate::paths::resolve_to_cwd;
use serde_json::{Map, Value, json};

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "edit",
        description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".to_owned(),
        parameters: json!({
            "type": "object",
            "required": ["path", "edits"],
            "properties": {
                "path": {"type": "string", "description": "Path to the file to edit (relative or absolute)"},
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["oldText", "newText"],
                        "properties": {
                            "oldText": {"type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},
                            "newText": {"type": "string", "description": "Replacement text for this targeted edit."}
                        }
                    },
                    "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead."
                }
            }
        }),
        prompt: PromptContribution {
            snippet: "Make precise file edits with exact text replacement, including multiple disjoint edits in one call",
            guidelines: &[
                "Use edit for precise changes (edits[].oldText must match exactly)",
                "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls",
                "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.",
                "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.",
            ],
        },
        prepare_arguments: Some(prepare_arguments),
        constrained_sampling: super::json_schema_sampling(),
        execute,
    }
}

fn is_single_edit(value: &Value) -> bool {
    value.get("oldText").is_some_and(Value::is_string)
        && value.get("newText").is_some_and(Value::is_string)
}

/// `prepareEditArguments`: `edits` sent as a JSON string or a single object, and a top-level
/// `oldText`/`newText` pair, become an `edits` array. The pair moves to the end of the keys.
pub fn prepare_arguments(input: Value) -> Value {
    let Value::Object(mut args) = input else {
        return input;
    };
    match args.get("edits") {
        Some(Value::String(text)) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                if parsed.is_array() {
                    args.insert("edits".to_owned(), parsed);
                } else if is_single_edit(&parsed) {
                    args.insert("edits".to_owned(), json!([parsed]));
                }
            }
        }
        Some(single) if is_single_edit(single) => {
            let edit = single.clone();
            args.insert("edits".to_owned(), json!([edit]));
        }
        _ => {}
    }
    let (Some(Value::String(old)), Some(Value::String(new))) =
        (args.get("oldText"), args.get("newText"))
    else {
        return Value::Object(args);
    };
    let pair = json!({"oldText": old, "newText": new});
    let mut edits = match args.get("edits") {
        Some(Value::Array(existing)) => existing.clone(),
        _ => Vec::new(),
    };
    edits.push(pair);
    let mut out = Map::new();
    for (key, value) in args {
        if key != "oldText" && key != "newText" && key != "edits" {
            out.insert(key, value);
        }
    }
    out.insert("edits".to_owned(), Value::Array(edits));
    Value::Object(out)
}

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    let path = str_arg(args, "path").unwrap_or_default();
    let edits: Vec<Edit> = match args.get("edits") {
        Some(Value::Array(items)) if !items.is_empty() => {
            serde_json::from_value(Value::Array(items.clone())).map_err(|e| e.to_string())?
        }
        _ => {
            return Err(
                "Edit tool input is invalid. edits must contain at least one replacement."
                    .to_owned(),
            );
        }
    };
    let absolute = resolve_to_cwd(path, ctx.cwd);
    check_aborted(ctx.cancel)?;
    node_fs::access(&absolute, node_fs::R_OK | node_fs::W_OK).map_err(|e| {
        format!(
            "Could not edit file: {path}. Error code: {}.",
            node_fs::code(&e)
        )
    })?;
    check_aborted(ctx.cancel)?;
    let bytes = std::fs::read(&absolute).map_err(|e| {
        if node_fs::code(&e) == "EISDIR" {
            "EISDIR: illegal operation on a directory, read".to_owned()
        } else {
            node_fs::message(&e, "open", &absolute)
        }
    })?;
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    check_aborted(ctx.cancel)?;
    let (bom, content) = strip_bom(&raw);
    let ending = detect_line_ending(content);
    let normalized = normalize_to_lf(content);
    let applied = apply_edits_to_normalized_content(&normalized, &edits, path)?;
    check_aborted(ctx.cancel)?;
    let final_content = format!(
        "{bom}{}",
        restore_line_endings(&applied.new_content, ending)
    );
    std::fs::write(&absolute, final_content)
        .map_err(|e| node_fs::message(&e, "open", &absolute))?;
    check_aborted(ctx.cancel)?;
    let diff = generate_diff_string(&applied.base_content, &applied.new_content);
    let patch = generate_unified_patch(path, &applied.base_content, &applied.new_content);
    let mut details = json!({"diff": diff.diff, "patch": patch});
    if let Some(line) = diff.first_changed_line {
        details["firstChangedLine"] = json!(line);
    }
    Ok(ToolOutput::text(format!(
        "Successfully replaced {} block(s) in {path}.",
        edits.len()
    ))
    .with_details(details))
}
