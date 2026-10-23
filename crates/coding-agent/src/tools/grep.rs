//! `core/tools/grep.ts`: ripgrep's JSON output, formatted as `path:line: text`.

use super::truncate::{
    DEFAULT_MAX_BYTES, GREP_MAX_LINE_LENGTH, Limits, format_size, truncate_head, truncate_line,
};
use super::{
    PromptContribution, ToolContext, ToolOutput, ToolSpec, check_aborted, js_number, num_arg,
    str_arg,
};
use crate::paths::{relative, resolve_to_cwd};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

const DEFAULT_LIMIT: f64 = 100.0;

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "grep",
        description: format!(
            "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore. Output is truncated to {DEFAULT_LIMIT} matches or {}KB (whichever is hit first). Long lines are truncated to {GREP_MAX_LINE_LENGTH} chars.",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {"type": "string", "description": "Search pattern (regex or literal string)"},
                "path": {"type": "string", "description": "Directory or file to search (default: current directory)"},
                "glob": {"type": "string", "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'"},
                "ignoreCase": {"type": "boolean", "description": "Case-insensitive search (default: false)"},
                "literal": {"type": "boolean", "description": "Treat pattern as literal string instead of regex (default: false)"},
                "context": {"type": "number", "description": "Number of lines to show before and after each match (default: 0)"},
                "limit": {"type": "number", "description": "Maximum number of matches to return (default: 100)"}
            }
        }),
        prompt: PromptContribution {
            snippet: "Search file contents for patterns (respects .gitignore)",
            guidelines: &[],
        },
        prepare_arguments: None,
        execute,
    }
}

/// `basename` of a POSIX path.
pub(crate) fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

fn split_file_lines(content: &str) -> Vec<String> {
    content
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_owned)
        .collect()
}

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    check_aborted(ctx.cancel)?;
    let pattern = str_arg(args, "pattern").unwrap_or_default();
    let rg = crate::config::tool_path("rg")
        .ok_or("ripgrep (rg) is not available and could not be downloaded")?;
    let search = resolve_to_cwd(
        str_arg(args, "path")
            .filter(|p| !p.is_empty())
            .unwrap_or("."),
        ctx.cwd,
    );
    let is_dir = std::fs::metadata(&search)
        .map_err(|_| format!("Path not found: {search}"))?
        .is_dir();
    let context = num_arg(args, "context").filter(|c| *c > 0.0).unwrap_or(0.0);
    let limit = num_arg(args, "limit").unwrap_or(DEFAULT_LIMIT).max(1.0);
    let format_path = |file: &str| -> String {
        if is_dir {
            let rel = relative(&search, file);
            if !rel.is_empty() && !rel.starts_with("..") {
                return rel;
            }
        }
        basename(file).to_owned()
    };

    let mut cmd = Command::new(&rg);
    cmd.args([
        "--no-config",
        "--json",
        "--line-number",
        "--color=never",
        "--hidden",
    ]);
    if args.get("ignoreCase").and_then(Value::as_bool) == Some(true) {
        cmd.arg("--ignore-case");
    }
    if args.get("literal").and_then(Value::as_bool) == Some(true) {
        cmd.arg("--fixed-strings");
    }
    if let Some(glob) = str_arg(args, "glob").filter(|g| !g.is_empty()) {
        cmd.args(["--glob", glob]);
    }
    cmd.arg("--").arg(pattern).arg(&search);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to run ripgrep: {}",
                crate::node_fs::message(&e, "spawn", &rg)
                    .replace(&format!(" '{rg}'"), &format!(" {rg}"))
            )
        })?;
    let mut stderr_pipe = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text
    });
    let mut matches: Vec<(String, f64, Option<String>)> = Vec::new();
    let mut limit_reached = false;
    let mut killed = false;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if ctx.cancel.is_cancelled() {
                let _ = child.kill();
                killed = true;
                break;
            }
            if line.trim().is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if event.get("type").and_then(Value::as_str) != Some("match") {
                continue;
            }
            let data = &event["data"];
            let path = data["path"]["text"].as_str().filter(|p| !p.is_empty());
            if let (Some(path), Some(n)) = (path, data["line_number"].as_f64()) {
                matches.push((
                    path.to_owned(),
                    n,
                    data["lines"]["text"].as_str().map(str::to_owned),
                ));
            }
            if matches.len() as f64 >= limit {
                limit_reached = true;
                let _ = child.kill();
                killed = true;
                break;
            }
        }
    }
    let status = child.wait();
    let stderr = stderr_thread.join().unwrap_or_default();
    if ctx.cancel.is_cancelled() {
        return Err("Operation aborted".to_owned());
    }
    let code = status.ok().and_then(|s| s.code());
    if !killed && code != Some(0) && code != Some(1) {
        let message = stderr.trim();
        return Err(if message.is_empty() {
            format!(
                "ripgrep exited with code {}",
                code.map_or("null".to_owned(), |c| c.to_string())
            )
        } else {
            message.to_owned()
        });
    }
    if matches.is_empty() {
        return Ok(ToolOutput::text("No matches found"));
    }

    let mut cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut lines_truncated = false;
    let mut out: Vec<String> = Vec::new();
    for (file, number, text) in &matches {
        let rel = format_path(file);
        if let (0.0, Some(text)) = (context, text) {
            let clean = text.replace("\r\n", "\n").replace('\r', "");
            let clean = clean.strip_suffix('\n').unwrap_or(&clean);
            let (shown, cut) = truncate_line(clean, GREP_MAX_LINE_LENGTH);
            lines_truncated |= cut;
            out.push(format!("{rel}:{}: {shown}", js_number(*number)));
            continue;
        }
        let lines = cache.entry(file.clone()).or_insert_with(|| {
            std::fs::read(file)
                .map(|b| split_file_lines(&String::from_utf8_lossy(&b)))
                .unwrap_or_default()
        });
        if lines.is_empty() {
            out.push(format!(
                "{rel}:{}: (unable to read file)",
                js_number(*number)
            ));
            continue;
        }
        let (start, end) = if context > 0.0 {
            (
                (number - context).max(1.0),
                (lines.len() as f64).min(number + context),
            )
        } else {
            (*number, *number)
        };
        let mut current = start;
        while current <= end {
            let text = if current.fract() == 0.0 {
                lines
                    .get(current as usize - 1)
                    .map(String::as_str)
                    .unwrap_or("")
            } else {
                ""
            };
            let (shown, cut) = truncate_line(&text.replace('\r', ""), GREP_MAX_LINE_LENGTH);
            lines_truncated |= cut;
            if current == *number {
                out.push(format!("{rel}:{}: {shown}", js_number(current)));
            } else {
                out.push(format!("{rel}-{}- {shown}", js_number(current)));
            }
            current += 1.0;
        }
    }

    let truncation = truncate_head(
        &out.join("\n"),
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
            "{} matches limit reached. Use limit={} for more, or refine pattern",
            js_number(limit),
            js_number(limit * 2.0)
        ));
        details.insert("matchLimitReached".to_owned(), json!(limit));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert("truncation".to_owned(), json!(truncation));
    }
    if lines_truncated {
        notices.push(format!(
            "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
        ));
        details.insert("linesTruncated".to_owned(), json!(true));
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
