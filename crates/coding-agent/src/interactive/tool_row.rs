//! `components/tool-execution.ts` with the built-in tools' renderers (`core/tools/renderers`):
//! one row per tool call, its background pending, green or red, the call on top and the result
//! below, cut to a preview until Ctrl+O expands it.

use std::time::Instant;

use pi_durable::ai::types::{ToolResultMessage, UserBlock};
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::tui::Component;
use serde_json::{Map, Value};

use super::theme::{bg_fn, bold, fg};
use crate::tools::truncate::format_size;

const DEFAULT_MAX_BYTES: usize = 50 * 1024;
const COLLAPSED_ARGS_CHARS: usize = 100;

/// A tool's result as the row shows it.
#[derive(Clone, Default)]
pub struct ShownResult {
    pub text: String,
    pub details: Option<Value>,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
}

impl ShownResult {
    pub fn from_message(message: &ToolResultMessage) -> Self {
        ShownResult {
            text: text_output(&message.content),
            details: message.details.clone(),
            is_error: message.is_error,
            duration_ms: message.duration_ms,
        }
    }
}

/// `getTextOutput`: the text blocks without escape sequences or CRs, images as a note.
pub fn text_output(content: &[UserBlock]) -> String {
    let mut parts = Vec::new();
    for block in content {
        match block {
            UserBlock::Text(t) => parts.push(
                pi_tui::utils::strip_terminal_sequences(&t.text).replace('\r', ""),
            ),
            UserBlock::Image(i) => parts.push(format!("[Image: {}]", i.mime_type)),
        }
    }
    parts.join("\n")
}

pub struct ToolRow {
    pub name: String,
    pub call_id: String,
    args: Map<String, Value>,
    expanded: bool,
    started: Option<Instant>,
    partial: Option<String>,
    result: Option<ShownResult>,
    spacer: Spacer,
    body: Text,
    dirty: bool,
}

fn s<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn shorten_path(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

fn tool_path(raw: Option<&str>, fallback: &str) -> String {
    match raw.filter(|p| !p.is_empty()).or(Some(fallback).filter(|f| !f.is_empty())) {
        Some(p) => fg("accent", &shorten_path(p)),
        None => fg("toolOutput", "..."),
    }
}

fn title(name: &str) -> String {
    fg("toolTitle", &bold(name))
}

fn expand_hint(prefix: &str) -> String {
    format!(
        "{} {}{}",
        fg("muted", prefix),
        fg("dim", "ctrl+o") + &fg("muted", " to expand"),
        fg("muted", ")")
    )
}

/// `formatToolDuration`.
pub fn duration(ms: u64) -> String {
    let seconds = ms as f64 / 1000.0;
    if seconds < 60.0 {
        return format!("{seconds:.1}s");
    }
    let total = ms / 1000;
    let (minutes, rest) = (total / 60, total % 60);
    if minutes < 60 {
        return format!("{minutes}m {rest}s");
    }
    format!("{}h {}m {rest}s", minutes / 60, minutes % 60)
}

/// `formatToolCallWithArgs` for tools without their own renderer.
fn generic_call(name: &str, args: &Map<String, Value>, expanded: bool) -> String {
    let header = title(name);
    if args.is_empty() {
        return header;
    }
    if expanded {
        let lines: Vec<String> = args
            .iter()
            .map(|(k, v)| {
                let text = match v {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                format!("  {k}: {}", text.replace('\t', "   ").replace('\r', "").replace('\n', "\n    "))
            })
            .collect();
        return format!("{header}\n{}", fg("muted", &lines.join("\n")));
    }
    let pairs: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let pairs = pairs.join(" ");
    let preview = if pairs.chars().count() > COLLAPSED_ARGS_CHARS {
        format!("{}...", pairs.chars().take(COLLAPSED_ARGS_CHARS - 3).collect::<String>())
    } else {
        pairs
    };
    format!("{header} {}", fg("muted", &preview))
}

/// The first `max` lines (or all when expanded) in `toolOutput`, with the "more lines" hint.
fn head_preview(output: &str, max: usize, expanded: bool) -> String {
    let lines: Vec<&str> = output.split('\n').collect();
    let shown = if expanded { lines.len() } else { max.min(lines.len()) };
    let mut text = format!(
        "\n{}",
        lines[..shown].iter().map(|l| fg("toolOutput", &l.replace('\t', "   "))).collect::<Vec<_>>().join("\n")
    );
    if lines.len() > shown {
        text += &expand_hint(&format!("\n... ({} more lines,", lines.len() - shown));
    }
    text
}

impl ToolRow {
    pub fn new(name: &str, call_id: &str, args: Map<String, Value>, _cwd: &str, expanded: bool) -> Self {
        let mut row = ToolRow {
            name: name.to_string(),
            call_id: call_id.to_string(),
            args,
            expanded,
            started: None,
            partial: None,
            result: None,
            spacer: Spacer::new(1),
            body: Text::with_bg("", 1, 1, bg_fn("toolPendingBg")),
            dirty: true,
        };
        row.refresh();
        row
    }

    pub fn set_args(&mut self, args: Map<String, Value>) {
        self.args = args;
        self.dirty = true;
    }

    pub fn start(&mut self) {
        self.started.get_or_insert_with(Instant::now);
        self.dirty = true;
    }

    pub fn partial(&self) -> Option<String> {
        self.partial.clone()
    }

    pub fn set_partial(&mut self, output: String) {
        self.partial = Some(output);
        self.dirty = true;
    }

    pub fn set_result(&mut self, result: ShownResult) {
        self.result = Some(result);
        self.dirty = true;
    }

    pub fn is_running(&self) -> bool {
        self.started.is_some() && self.result.is_none()
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.dirty = true;
    }

    /// Redraw the elapsed timer of a running row.
    pub fn tick(&mut self) {
        if self.is_running() {
            self.dirty = true;
        }
    }

    fn timer(&self) -> Option<String> {
        match &self.result {
            Some(r) => r.duration_ms.map(|ms| format!("Took {}", duration(ms))),
            None => self
                .started
                .map(|t| format!("Elapsed {}", duration(t.elapsed().as_millis() as u64))),
        }
    }

    fn call_text(&self) -> String {
        let a = &self.args;
        match self.name.as_str() {
            "bash" => {
                let command = match s(a, "command") {
                    Some(c) if !c.is_empty() => c.to_string(),
                    _ => fg("toolOutput", "..."),
                };
                let timeout = a.get("timeout").and_then(Value::as_f64);
                let suffix = timeout
                    .map(|t| fg("muted", &format!(" (timeout {t}s)")))
                    .unwrap_or_default();
                fg("toolTitle", &bold(&format!("$ {command}"))) + &suffix
            }
            "read" => {
                let path = tool_path(s(a, "path").or(s(a, "file_path")), "");
                let range = match (a.get("offset").and_then(Value::as_u64), a.get("limit").and_then(Value::as_u64)) {
                    (None, None) => String::new(),
                    (offset, limit) => {
                        let start = offset.unwrap_or(1);
                        let end = limit.map(|l| (start + l - 1).to_string()).unwrap_or_default();
                        fg("warning", &format!(":{start}{}", if end.is_empty() { String::new() } else { format!("-{end}") }))
                    }
                };
                format!("{} {path}{range}", title("read"))
            }
            "edit" | "write" => {
                let path = tool_path(s(a, "path").or(s(a, "file_path")), "");
                let mut text = format!("{} {path}", title(&self.name));
                if self.name == "write"
                    && let Some(content) = s(a, "content").filter(|c| !c.is_empty())
                {
                    text += "\n";
                    text += &head_preview(content.trim_end_matches('\n'), 10, self.expanded);
                }
                text
            }
            "ls" => {
                let mut text = format!("{} {}", title("ls"), tool_path(s(a, "path"), "."));
                if let Some(limit) = a.get("limit") {
                    text += &fg("toolOutput", &format!(" (limit {limit})"));
                }
                text
            }
            "grep" | "find" => {
                let pattern = s(a, "pattern").unwrap_or("");
                let shown = if self.name == "grep" { format!("/{pattern}/") } else { pattern.to_string() };
                let mut text = format!(
                    "{} {}{}",
                    title(&self.name),
                    fg("accent", &shown),
                    fg("toolOutput", &format!(" in {}", s(a, "path").unwrap_or(".")))
                );
                if let Some(glob) = s(a, "glob") {
                    text += &fg("toolOutput", &format!(" ({glob})"));
                }
                if let Some(limit) = a.get("limit") {
                    text += &fg("toolOutput", &format!(" limit {limit}"));
                }
                text
            }
            _ => generic_call(&self.name, a, self.expanded),
        }
    }
}

/// `renderDiff`, without the intra-line highlight of a one-line change.
pub fn render_diff(diff: &str) -> String {
    let re = regex::Regex::new(r"^([+\-\s])(\s*\d*)\s(.*)$").expect("diff line pattern");
    diff.split('\n')
        .map(|line| match re.captures(line) {
            None => fg("toolDiffContext", line),
            Some(c) => {
                let (prefix, num, content) = (&c[1], &c[2], c[3].replace('\t', "   "));
                match prefix {
                    "-" => fg("toolDiffRemoved", &format!("-{num} {content}")),
                    "+" => fg("toolDiffAdded", &format!("+{num} {content}")),
                    _ => fg("toolDiffContext", &format!(" {num} {content}")),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncation_limit(details: Option<&Value>) -> Option<String> {
    let t = details?.get("truncation")?;
    if t.get("truncated").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let max = t.get("maxBytes").and_then(Value::as_u64).map_or(DEFAULT_MAX_BYTES, |m| m as usize);
    Some(format!("{} limit", format_size(max)))
}

impl ToolRow {
    fn result_text(&self) -> String {
        let (output, details, is_error) = match (&self.result, &self.partial) {
            (Some(r), _) => (r.text.clone(), r.details.as_ref(), r.is_error),
            (None, Some(p)) => (p.clone(), None, false),
            (None, None) => return String::new(),
        };
        let expanded = self.expanded;
        match self.name.as_str() {
            "bash" => {
                let mut text = String::new();
                let mut out = output.trim().to_string();
                let full = details.and_then(|d| d.get("fullOutputPath")).and_then(Value::as_str);
                if self.result.is_some()
                    && let Some(path) = full
                    && let Some(cut) = out.rfind("\n\n[")
                    && out[cut..].contains(path)
                {
                    out = out[..cut].trim_end().to_string();
                }
                if !out.is_empty() {
                    let lines: Vec<&str> = out.split('\n').collect();
                    let keep = if expanded { lines.len() } else { 5.min(lines.len()) };
                    text += "\n\n";
                    if lines.len() > keep {
                        text += &expand_hint(&format!("... ({} earlier lines,", lines.len() - keep));
                        text += "\n";
                    }
                    text += &lines[lines.len() - keep..].iter().map(|l| fg("toolOutput", l)).collect::<Vec<_>>().join("\n");
                }
                let mut warnings = Vec::new();
                if let Some(path) = full {
                    warnings.push(format!("Full output: {path}"));
                }
                if let Some(t) = details.and_then(|d| d.get("truncation"))
                    && t.get("truncated").and_then(Value::as_bool) == Some(true)
                {
                    let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
                    if t.get("truncatedBy").and_then(Value::as_str) == Some("lines") {
                        warnings.push(format!("Truncated: showing {} of {} lines", n("outputLines"), n("totalLines")));
                    } else {
                        let max = t.get("maxBytes").and_then(Value::as_u64).map_or(DEFAULT_MAX_BYTES, |m| m as usize);
                        warnings.push(format!("Truncated: {} lines shown ({} limit)", n("outputLines"), format_size(max)));
                    }
                }
                if !warnings.is_empty() {
                    text += &format!("\n\n{}", fg("warning", &format!("[{}]", warnings.join(". "))));
                }
                if let Some(timer) = self.timer() {
                    text += &format!("\n\n{}", fg("muted", &timer));
                }
                text
            }
            "read" if !expanded && !is_error => String::new(),
            "edit" if !is_error => details
                .and_then(|d| d.get("diff"))
                .and_then(Value::as_str)
                .map(|d| format!("\n{}", render_diff(d)))
                .unwrap_or_default(),
            "edit" | "write" if is_error => format!("\n{}", fg("error", &output)),
            "write" => String::new(),
            "read" | "ls" | "grep" | "find" => {
                let out = output.trim_end_matches('\n');
                let max = match self.name.as_str() {
                    "read" => 10,
                    "grep" => 15,
                    _ => 20,
                };
                if is_error {
                    return format!("\n{}", fg("error", out));
                }
                let mut text = if out.trim().is_empty() { String::new() } else { head_preview(out.trim(), max, expanded) };
                let mut warnings = Vec::new();
                let detail = |k: &str| details.and_then(|d| d.get(k)).filter(|v| !v.is_null() && *v != &Value::Bool(false));
                match self.name.as_str() {
                    "ls" => {
                        if let Some(n) = detail("entryLimitReached") {
                            warnings.push(format!("{n} entries limit"));
                        }
                    }
                    "find" => {
                        if let Some(n) = detail("resultLimitReached") {
                            warnings.push(format!("{n} results limit"));
                        }
                    }
                    "grep" => {
                        if let Some(n) = detail("matchLimitReached") {
                            warnings.push(format!("{n} matches limit"));
                        }
                    }
                    _ => {}
                }
                if self.name != "read"
                    && let Some(limit) = truncation_limit(details)
                {
                    warnings.push(limit);
                }
                if self.name == "grep" && detail("linesTruncated").is_some() {
                    warnings.push("some lines truncated".into());
                }
                if !warnings.is_empty() {
                    text += &format!("\n{}", fg("warning", &format!("[Truncated: {}]", warnings.join(", "))));
                }
                text
            }
            _ => {
                if output.is_empty() {
                    return String::new();
                }
                head_preview(&output, 10, expanded)
            }
        }
    }

    fn refresh(&mut self) {
        let bg = match &self.result {
            None => "toolPendingBg",
            Some(r) if r.is_error => "toolErrorBg",
            Some(_) => "toolSuccessBg",
        };
        self.body.set_custom_bg_fn(Some(bg_fn(bg)));
        self.body.set_text(&format!("{}{}", self.call_text(), self.result_text()));
        self.dirty = false;
    }
}

impl Component for ToolRow {
    fn render(&mut self, width: usize) -> Vec<String> {
        if self.dirty {
            self.refresh();
        }
        let mut lines = self.spacer.render(width);
        lines.extend(self.body.render(width));
        lines
    }
    fn invalidate(&mut self) {
        self.body.invalidate();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        pi_tui::utils::strip_terminal_sequences(s)
    }

    #[test]
    fn durations_read_like_upstream() {
        assert_eq!(duration(4200), "4.2s");
        assert_eq!(duration(90_000), "1m 30s");
        assert_eq!(duration(7_384_000), "2h 3m 4s");
    }

    #[test]
    fn bash_row_keeps_the_last_five_lines() {
        let mut args = Map::new();
        args.insert("command".into(), Value::String("seq 8".into()));
        let mut row = ToolRow::new("bash", "c1", args, "/", false);
        row.set_result(ShownResult {
            text: "1\n2\n3\n4\n5\n6\n7\n8".into(),
            duration_ms: Some(1200),
            ..ShownResult::default()
        });
        let lines: Vec<String> = row.render(60).iter().map(|l| plain(l).trim_end().to_string()).collect();
        assert_eq!(
            lines,
            ["", "", " $ seq 8", "", " ... (3 earlier lines, ctrl+o to expand)", " 4", " 5", " 6", " 7", " 8", "", " Took 1.2s", ""]
        );
    }

    #[test]
    fn diff_lines_take_their_colors() {
        let out = plain(&render_diff("-1 old\n+1 new\n 2 same"));
        assert_eq!(out, "-1 old\n+1 new\n 2 same");
    }
}
