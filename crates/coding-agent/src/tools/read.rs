//! `core/tools/read.ts`.

use super::mime::{base64, detect_supported_image_mime_type_from_file};
use super::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, Limits, TruncatedBy, format_size, truncate_head,
};
use super::{
    PromptContribution, ToolContext, ToolOutput, ToolSpec, check_aborted, image_block, js_number,
    num_arg, str_arg, text_block,
};
use crate::node_fs;
use crate::paths::resolve_read_path;
use pi_durable::ai::types::UserBlock;
use serde_json::{Value, json};

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "read",
        description: format!(
            "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to {DEFAULT_MAX_LINES} lines or {}KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {"type": "string", "description": "Path to the file to read (relative or absolute)"},
                "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
                "limit": {"type": "number", "description": "Maximum number of lines to read"}
            }
        }),
        prompt: PromptContribution {
            snippet: "Read file contents",
            guidelines: &["Use read to examine files instead of cat or sed."],
        },
        prepare_arguments: None,
        constrained_sampling: super::json_schema_sampling(),
        execute,
    }
}

/// `Array.prototype.slice`'s index: truncated, negative from the end, clamped to `len`.
fn slice_index(index: f64, len: usize) -> usize {
    let len_f = len as f64;
    let i = if index.is_nan() { 0.0 } else { index.trunc() };
    let i = if i < 0.0 {
        (len_f + i).max(0.0)
    } else {
        i.min(len_f)
    };
    i as usize
}

const NON_VISION_NOTE: &str =
    "[Current model does not support images. The image will be omitted from this request.]";

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    check_aborted(ctx.cancel)?;
    let path = str_arg(args, "path").unwrap_or_default();
    let offset = num_arg(args, "offset");
    let limit = num_arg(args, "limit");
    let absolute = resolve_read_path(path, ctx.cwd);
    node_fs::access(&absolute, node_fs::R_OK)
        .map_err(|e| node_fs::message(&e, "access", &absolute))?;
    check_aborted(ctx.cancel)?;
    let mime = detect_supported_image_mime_type_from_file(&absolute)
        .map_err(|e| read_error(&e, &absolute))?;
    let bytes = std::fs::read(&absolute).map_err(|e| read_error(&e, &absolute))?;
    check_aborted(ctx.cancel)?;
    let output = match mime {
        Some(mime) => image_output(&bytes, mime, ctx.model_takes_images),
        None => read_text(&String::from_utf8_lossy(&bytes), path, offset, limit)?,
    };
    Ok(ToolOutput {
        structured_content: Some(read_output(&output.content)),
        ..output
    })
}

fn read_output(content: &[UserBlock]) -> Value {
    let text = content
        .iter()
        .find_map(|block| match block {
            UserBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .unwrap_or_default();
    match content.iter().find_map(|block| match block {
        UserBlock::Image(image) => Some(image),
        _ => None,
    }) {
        Some(image) => json!({
            "type": "image",
            "data": image.data,
            "mimeType": image.mime_type,
            "note": text,
        }),
        None => json!(text),
    }
}

fn read_error(error: &std::io::Error, path: &str) -> String {
    if node_fs::code(error) == "EISDIR" {
        "EISDIR: illegal operation on a directory, read".to_owned()
    } else {
        node_fs::message(error, "open", path)
    }
}

/// Images go as they are. Upstream converts BMP to PNG and shrinks large images with Photon,
/// which has no Rust port here; BMP is omitted the way upstream omits images it cannot convert.
fn image_output(bytes: &[u8], mime: &str, model_takes_images: bool) -> ToolOutput {
    let note = (!model_takes_images).then_some(NON_VISION_NOTE);
    if mime == "image/bmp" {
        let mut text = format!(
            "Read image file [{mime}]\n[Image omitted: could not be converted to a supported inline image format.]"
        );
        if let Some(note) = note {
            text = format!("{text}\n{note}");
        }
        return ToolOutput::text(text);
    }
    let mut text = format!("Read image file [{mime}]");
    if let Some(note) = note {
        text = format!("{text}\n{note}");
    }
    ToolOutput {
        content: vec![text_block(text), image_block(base64(bytes), mime)],
        details: None,
        is_error: false,
        structured_content: None,
    }
}

/// The text branch, on the decoded file.
pub fn read_text(
    text: &str,
    path: &str,
    offset: Option<f64>,
    limit: Option<f64>,
) -> Result<ToolOutput, String> {
    let all: Vec<&str> = text.split('\n').collect();
    let total = all.len();
    let start = match offset {
        Some(o) if o != 0.0 && !o.is_nan() => (o - 1.0).max(0.0),
        _ => 0.0,
    };
    let start_display = start + 1.0;
    if start >= total as f64 {
        return Err(format!(
            "Offset {} is beyond end of file ({total} lines total)",
            offset
                .map(js_number)
                .unwrap_or_else(|| "undefined".to_owned())
        ));
    }
    let from = slice_index(start, total);
    let mut user_limited = None;
    let selected = match limit {
        Some(limit) => {
            let end = (start + limit).min(total as f64);
            user_limited = Some(end - start);
            let to = slice_index(end, total);
            if to > from {
                all[from..to].join("\n")
            } else {
                String::new()
            }
        }
        None => all[from..].join("\n"),
    };
    let truncation = truncate_head(&selected, Limits::default());
    let max = format_size(DEFAULT_MAX_BYTES);
    let output;
    let mut details = None;
    if truncation.first_line_exceeds_limit {
        let size = format_size(all[from].len());
        let line = js_number(start_display);
        output = format!(
            "[Line {line} is {size}, exceeds {max} limit. Use bash: sed -n '{line}p' {path} | head -c {DEFAULT_MAX_BYTES}]"
        );
        details = Some(json!({ "truncation": truncation }));
    } else if truncation.truncated {
        let end_display = start_display + truncation.output_lines as f64 - 1.0;
        let range = format!("{}-{}", js_number(start_display), js_number(end_display));
        let next = js_number(end_display + 1.0);
        output = if truncation.truncated_by == Some(TruncatedBy::Lines) {
            format!(
                "{}\n\n[Showing lines {range} of {total}. Use offset={next} to continue.]",
                truncation.content
            )
        } else {
            format!(
                "{}\n\n[Showing lines {range} of {total} ({max} limit). Use offset={next} to continue.]",
                truncation.content
            )
        };
        details = Some(json!({ "truncation": truncation }));
    } else if let Some(limited) = user_limited.filter(|l| start + l < total as f64) {
        let remaining = total as f64 - (start + limited);
        output = format!(
            "{}\n\n[{} more lines in file. Use offset={} to continue.]",
            truncation.content,
            js_number(remaining),
            js_number(start + limited + 1.0)
        );
    } else {
        output = truncation.content;
    }
    Ok(ToolOutput {
        content: vec![text_block(output)],
        details,
        is_error: false,
        structured_content: None,
    })
}
