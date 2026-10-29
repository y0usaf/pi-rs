//! `core/tools/find.ts`: fd with a glob, paths relative to the search directory.

use super::truncate::{DEFAULT_MAX_BYTES, Limits, format_size, truncate_head};
use super::{
    PromptContribution, ToolContext, ToolOutput, ToolSpec, check_aborted, js_number, num_arg,
    str_arg,
};
use crate::paths::{relative, resolve_to_cwd};
use serde_json::{Map, Value, json};
use std::io::Read;
use std::process::{Command, Stdio};

const DEFAULT_LIMIT: f64 = 1000.0;

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "find",
        description: format!(
            "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore. Output is truncated to {DEFAULT_LIMIT} results or {}KB (whichever is hit first).",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'"},
                "path": {"type": "string", "description": "Directory to search in (default: current directory)"},
                "limit": {"type": "number", "description": "Maximum number of results (default: 1000)"}
            }
        }),
        prompt: PromptContribution {
            snippet: "Find files by glob pattern (respects .gitignore)",
            guidelines: &[],
        },
        prepare_arguments: None,
        constrained_sampling: None,
        execute,
    }
}

/// `relativizeFindResultPath`.
pub fn relativize(result: &str, search: &str) -> String {
    let trailing = result.ends_with('/');
    let rel = if result.starts_with('/') {
        relative(search, &crate::paths::normalize_absolute(result))
    } else {
        result.to_owned()
    };
    if trailing && !rel.ends_with('/') {
        format!("{rel}/")
    } else {
        rel
    }
}

fn inside_git_repo(start: &str) -> bool {
    let mut current = std::path::PathBuf::from(start);
    loop {
        if current.join(".git").exists() {
            return true;
        }
        if !current.pop() {
            return false;
        }
    }
}

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    check_aborted(ctx.cancel)?;
    let pattern = str_arg(args, "pattern").unwrap_or_default();
    let search = resolve_to_cwd(
        str_arg(args, "path")
            .filter(|p| !p.is_empty())
            .unwrap_or("."),
        ctx.cwd,
    );
    let limit = num_arg(args, "limit").unwrap_or(DEFAULT_LIMIT);
    let fd =
        crate::config::tool_path("fd").ok_or("fd is not available and could not be downloaded")?;
    check_aborted(ctx.cancel)?;
    let mut cmd = Command::new(&fd);
    cmd.args(["--glob", "--color=never", "--hidden"]);
    if !inside_git_repo(&search) {
        cmd.arg("--no-require-git");
    }
    cmd.arg("--max-results").arg(js_number(limit));
    let mut effective = pattern.to_owned();
    if pattern.contains('/') {
        cmd.arg("--full-path");
        if !pattern.starts_with('/') && !pattern.starts_with("**/") && pattern != "**" {
            effective = format!("**/{pattern}");
        }
    }
    cmd.arg("--").arg(&effective).arg(&search);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to run fd: {}",
                crate::node_fs::message(&e, "spawn", &fd)
                    .replace(&format!(" '{fd}'"), &format!(" {fd}"))
            )
        })?;
    let mut stderr_pipe = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    let status = child.wait();
    let stderr = stderr_thread.join().unwrap_or_default();
    check_aborted(ctx.cancel)?;
    let text = String::from_utf8_lossy(&stdout);
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') || text.is_empty() {
        lines.pop();
    }
    let joined = lines.join("\n");
    let code = status.ok().and_then(|s| s.code());
    if code != Some(0) && joined.is_empty() {
        let message = stderr.trim();
        return Err(if message.is_empty() {
            format!(
                "fd exited with code {}",
                code.map_or("null".to_owned(), |c| c.to_string())
            )
        } else {
            message.to_owned()
        });
    }
    if joined.is_empty() {
        return Ok(ToolOutput::text("No files found matching pattern"));
    }
    let results: Vec<String> = lines
        .iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l).trim())
        .filter(|l| !l.is_empty())
        .map(|l| relativize(l, &search))
        .collect();
    let limit_reached = results.len() as f64 >= limit;
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
            "{} results limit reached. Use limit={} for more, or refine pattern",
            js_number(limit),
            js_number(limit * 2.0)
        ));
        details.insert("resultLimitReached".to_owned(), json!(limit));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert("truncation".to_owned(), json!(truncation));
    }
    if !notices.is_empty() {
        output = format!("{output}\n\n[{}]", notices.join(". "));
    }
    let result = ToolOutput::text(output);
    Ok(if details.is_empty() {
        result
    } else {
        result.with_details(Value::Object(details))
    })
}
