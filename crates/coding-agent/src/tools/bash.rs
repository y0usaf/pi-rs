//! `core/tools/bash.ts` and the `OutputAccumulator` it streams into.

use super::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, Limits, TruncatedBy, TruncationResult, format_size,
    truncate_tail,
};
use super::{
    PromptContribution, ToolContext, ToolOutput, ToolSpec, js_number, num_arg, str_arg, text_block,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_TIMEOUT_MS: f64 = 2_147_483_647.0;
const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);
const UPDATE_THROTTLE: Duration = Duration::from_millis(100);

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "bash",
        description: format!(
            "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last {DEFAULT_MAX_LINES} lines or {}KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {"type": "string", "description": "Shell command to execute"},
                "timeout": {"type": "number", "description": "Timeout in seconds (optional, no default timeout)"}
            }
        }),
        prompt: PromptContribution {
            snippet: "Execute bash commands (ls, grep, find, etc.)",
            guidelines: &[
                "You can inspect PI_* environment variables for current model and session details.",
            ],
        },
        prepare_arguments: None,
        execute,
    }
}

fn timeout_ms(timeout: Option<f64>) -> Result<Option<f64>, String> {
    let Some(t) = timeout else { return Ok(None) };
    if !t.is_finite() || t <= 0.0 {
        return Err("Invalid timeout: must be a finite number of seconds".to_owned());
    }
    if t * 1000.0 > MAX_TIMEOUT_MS {
        return Err(format!(
            "Invalid timeout: maximum is {} seconds",
            js_number(MAX_TIMEOUT_MS / 1000.0)
        ));
    }
    Ok(Some(t * 1000.0))
}

/// `getShellConfig()`: `/bin/bash`, else `bash` on PATH, else `sh`.
pub fn shell_path(custom: Option<&str>) -> Result<String, String> {
    if let Some(custom) = custom {
        return if std::path::Path::new(custom).exists() {
            Ok(custom.to_owned())
        } else {
            Err(format!("Custom shell path not found: {custom}"))
        };
    }
    if std::path::Path::new("/bin/bash").exists() {
        return Ok("/bin/bash".to_owned());
    }
    for dir in std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
    {
        let candidate = format!("{dir}/bash");
        if std::path::Path::new(&candidate).is_file() {
            return Ok(candidate);
        }
    }
    Ok("sh".to_owned())
}

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

fn kill_tree(pid: u32) {
    // SAFETY: kill(2) takes plain integers.
    unsafe {
        if kill(-(pid as i32), 9) != 0 {
            kill(pid as i32, 9);
        }
    }
}

enum Ended {
    Exit(Option<i32>),
    Aborted,
    TimedOut,
}

fn execute(args: &Value, ctx: &mut ToolContext<'_>) -> Result<ToolOutput, String> {
    let command = str_arg(args, "command").unwrap_or_default();
    let timeout = num_arg(args, "timeout");
    let mut acc = Accumulator::new();
    if let Some(update) = ctx.on_update.as_deref_mut() {
        update("");
    }
    let ended = run(command, timeout, ctx, &mut acc);
    acc.finish();
    let snapshot = acc.snapshot(true);
    if let Some(update) = ctx.on_update.as_deref_mut() {
        update(&snapshot.content);
    }
    let ended = match ended {
        Ok(ended) => ended,
        Err(message) => return Err(message),
    };
    let status = |text: &str, status: &str| {
        if text.is_empty() {
            status.to_owned()
        } else {
            format!("{text}\n\n{status}")
        }
    };
    match ended {
        Ended::Aborted => Err(status(&acc.format(&snapshot, "").0, "Command aborted")),
        Ended::TimedOut => Err(status(
            &acc.format(&snapshot, "").0,
            &format!(
                "Command timed out after {} seconds",
                js_number(timeout.unwrap_or(0.0))
            ),
        )),
        Ended::Exit(code) => {
            let (text, details) = acc.format(&snapshot, "(no output)");
            let Some(code) = code else {
                return Err(status(&text, "Command terminated without an exit code"));
            };
            let content = if code != 0 {
                status(&text, &format!("Command exited with code {code}"))
            } else {
                text
            };
            Ok(ToolOutput {
                content: vec![text_block(content)],
                details,
                is_error: code != 0,
            })
        }
    }
}

fn run(
    command: &str,
    timeout: Option<f64>,
    ctx: &mut ToolContext<'_>,
    acc: &mut Accumulator,
) -> Result<Ended, String> {
    let timeout_ms = timeout_ms(timeout)?;
    if ctx.cancel.is_cancelled() {
        return Ok(Ended::Aborted);
    }
    let shell = shell_path(ctx.shell_path)?;
    if !std::path::Path::new(ctx.cwd).exists() {
        return Err(format!(
            "Working directory does not exist: {}\nCannot execute bash commands.",
            ctx.cwd
        ));
    }
    let mut cmd = Command::new(&shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(ctx.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for key in [
        "PI_SESSION_ID",
        "PI_SESSION_FILE",
        "PI_PROVIDER",
        "PI_MODEL",
        "PI_REASONING_LEVEL",
    ] {
        cmd.env_remove(key);
    }
    for (key, value) in ctx.env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().map_err(|e| {
        crate::node_fs::message(&e, "spawn", &shell)
            .replace(&format!(" '{shell}'"), &format!(" {shell}"))
    })?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    let mut readers = Vec::new();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            let mut stream = stream;
            let mut buf = [0u8; 65536];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(Some(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = tx.send(None);
        }));
    }
    drop(tx);
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_secs_f64(ms / 1000.0));
    let mut open = readers.len();
    let mut exit: Option<(Option<i32>, Instant)> = None;
    let mut killed: Option<Ended> = None;
    let mut last_update = Instant::now() - UPDATE_THROTTLE;
    let mut dirty = false;
    loop {
        if killed.is_none() {
            if ctx.cancel.is_cancelled() {
                kill_tree(pid);
                killed = Some(Ended::Aborted);
            } else if deadline.is_some_and(|d| Instant::now() >= d) {
                kill_tree(pid);
                killed = Some(Ended::TimedOut);
            }
        }
        if exit.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                let code = status.code().or_else(|| status.signal().map(|s| 128 + s));
                exit = Some((code, Instant::now()));
            }
        }
        if let Some((_, at)) = exit {
            if open == 0 || at.elapsed() >= EXIT_STDIO_GRACE {
                break;
            }
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Some(chunk)) => {
                acc.append(&chunk);
                dirty = true;
                if let Some((code, _)) = exit {
                    exit = Some((code, Instant::now()));
                }
            }
            Ok(None) => open -= 1,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => open = 0,
        }
        if dirty && last_update.elapsed() >= UPDATE_THROTTLE {
            if let Some(update) = ctx.on_update.as_deref_mut() {
                let snap = acc.snapshot(true);
                update(&snap.content);
            }
            last_update = Instant::now();
            dirty = false;
        }
    }
    while let Ok(Some(chunk)) = rx.try_recv() {
        acc.append(&chunk);
    }
    if let Some(ended) = killed {
        return Ok(ended);
    }
    Ok(Ended::Exit(exit.and_then(|(code, _)| code)))
}

pub struct Snapshot {
    pub content: String,
    pub truncation: TruncationResult,
    pub full_output_path: Option<String>,
}

/// `OutputAccumulator`: a rolling tail of the decoded output, the line and byte totals, and a temp
/// file with the raw bytes once the output passes the limits.
pub struct Accumulator {
    max_rolling: usize,
    pending: Vec<u8>,
    raw: Vec<u8>,
    tail: String,
    tail_at_line_start: bool,
    total_raw: usize,
    total_decoded: usize,
    completed_lines: usize,
    total_lines: usize,
    line_bytes: usize,
    open_line: bool,
    temp: Option<(String, std::fs::File)>,
}

impl Accumulator {
    pub fn new() -> Self {
        Self {
            max_rolling: DEFAULT_MAX_BYTES * 2,
            pending: Vec::new(),
            raw: Vec::new(),
            tail: String::new(),
            tail_at_line_start: true,
            total_raw: 0,
            total_decoded: 0,
            completed_lines: 0,
            total_lines: 0,
            line_bytes: 0,
            open_line: false,
            temp: None,
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        self.total_raw += data.len();
        self.pending.extend_from_slice(data);
        let text = decode_stream(&mut self.pending, false);
        self.append_text(&text);
        if self.temp.is_some() || self.over_limits() {
            self.ensure_temp();
            if let Some((_, file)) = &mut self.temp {
                let _ = file.write_all(data);
            }
        } else {
            self.raw.extend_from_slice(data);
        }
    }

    pub fn finish(&mut self) {
        let text = decode_stream(&mut self.pending, true);
        self.append_text(&text);
        if self.over_limits() {
            self.ensure_temp();
        }
    }

    fn over_limits(&self) -> bool {
        self.total_raw > DEFAULT_MAX_BYTES
            || self.total_decoded > DEFAULT_MAX_BYTES
            || self.total_lines > DEFAULT_MAX_LINES
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.total_decoded += text.len();
        self.tail.push_str(text);
        if self.tail.len() > self.max_rolling * 2 {
            let mut start = self.tail.len() - self.max_rolling;
            while !self.tail.is_char_boundary(start) {
                start += 1;
            }
            self.tail_at_line_start = self.tail.as_bytes()[start - 1] == b'\n';
            self.tail = self.tail[start..].to_owned();
        }
        let newlines = text.matches('\n').count();
        if newlines == 0 {
            self.line_bytes += text.len();
            self.open_line = true;
        } else {
            self.completed_lines += newlines;
            let rest = &text[text.rfind('\n').unwrap_or(0) + 1..];
            self.line_bytes = rest.len();
            self.open_line = !rest.is_empty();
        }
        self.total_lines = self.completed_lines + usize::from(self.open_line);
    }

    fn ensure_temp(&mut self) {
        if self.temp.is_some() {
            return;
        }
        let path = crate::output_files::new_path("pi-bash", ".log");
        if let Ok(mut file) = crate::output_files::create(&path) {
            let _ = file.write_all(&self.raw);
            self.raw.clear();
            self.temp = Some((path, file));
        }
    }

    pub fn snapshot(&mut self, persist_if_truncated: bool) -> Snapshot {
        let text = if self.tail_at_line_start {
            self.tail.as_str()
        } else {
            self.tail
                .find('\n')
                .map_or(self.tail.as_str(), |i| &self.tail[i + 1..])
        };
        let tail = truncate_tail(text, Limits::default());
        let truncated =
            self.total_lines > DEFAULT_MAX_LINES || self.total_decoded > DEFAULT_MAX_BYTES;
        let truncated_by = truncated.then(|| {
            tail.truncated_by
                .unwrap_or(if self.total_decoded > DEFAULT_MAX_BYTES {
                    TruncatedBy::Bytes
                } else {
                    TruncatedBy::Lines
                })
        });
        let truncation = TruncationResult {
            truncated,
            truncated_by,
            total_lines: self.total_lines,
            total_bytes: self.total_decoded,
            max_lines: DEFAULT_MAX_LINES,
            max_bytes: DEFAULT_MAX_BYTES,
            ..tail
        };
        if persist_if_truncated && truncated {
            self.ensure_temp();
        }
        Snapshot {
            content: truncation.content.clone(),
            truncation,
            full_output_path: self.temp.as_ref().map(|(p, _)| p.clone()),
        }
    }

    /// `formatOutput`: the text with its truncation notice, and the details.
    pub fn format(&self, snapshot: &Snapshot, empty_text: &str) -> (String, Option<Value>) {
        let t = &snapshot.truncation;
        let mut text = if snapshot.content.is_empty() {
            empty_text.to_owned()
        } else {
            snapshot.content.clone()
        };
        if !t.truncated {
            return (text, None);
        }
        let mut details = json!({ "truncation": t });
        let path = snapshot
            .full_output_path
            .clone()
            .unwrap_or_else(|| "undefined".to_owned());
        if let Some(p) = &snapshot.full_output_path {
            details["fullOutputPath"] = json!(p);
        }
        let start = t.total_lines + 1 - t.output_lines;
        let end = t.total_lines;
        if t.last_line_partial {
            text += &format!(
                "\n\n[Showing last {} of line {end} (line is {}). Full output: {path}]",
                format_size(t.output_bytes),
                format_size(self.line_bytes)
            );
        } else if t.truncated_by == Some(TruncatedBy::Lines) {
            text += &format!(
                "\n\n[Showing lines {start}-{end} of {}. Full output: {path}]",
                t.total_lines
            );
        } else {
            text += &format!(
                "\n\n[Showing lines {start}-{end} of {} ({} limit). Full output: {path}]",
                t.total_lines,
                format_size(DEFAULT_MAX_BYTES)
            );
        }
        (text, Some(details))
    }
}

impl Default for Accumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// `TextDecoder.decode(data, { stream })`: decodes what it can and keeps an incomplete trailing
/// sequence for the next call; invalid bytes become U+FFFD.
fn decode_stream(pending: &mut Vec<u8>, flush: bool) -> String {
    let mut out = String::new();
    let mut rest: &[u8] = pending;
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                out.push_str(text);
                rest = &[];
                break;
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(good).unwrap_or_default());
                match e.error_len() {
                    Some(n) => {
                        out.push('\u{fffd}');
                        rest = &bad[n..];
                    }
                    None => {
                        if flush {
                            out.push('\u{fffd}');
                            rest = &[];
                        } else {
                            rest = bad;
                        }
                        break;
                    }
                }
            }
        }
    }
    *pending = rest.to_vec();
    out
}
