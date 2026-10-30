//! `core/bash-executor.ts`, `AgentSession.executeBash` and `messages.ts`: a command the
//! user asked for, with its output streamed and cancellable, kept as the `bashExecution`
//! message the model sees. `!!` runs store the entry outside that context.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pi_durable::ai::types::{
    Message, TextContent, TextType, UserBlock, UserContent, UserMessage, UserRole,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::harness::Conversation;
use pi_durable::harness::inbox::WriteDraft;
use pi_durable::harness::submissions::SubmissionDraft;

use crate::extensions::UserBash;
use crate::tools::bash::{kill_tree, shell_path};
use crate::tools::truncate::{DEFAULT_MAX_BYTES, Limits, truncate_tail};

/// The entry kind a `!` run is stored under.
pub const KIND: &str = "pi.bash";

const ROLLING_BYTES: usize = DEFAULT_MAX_BYTES * 2;
const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum ExitCode {
    Integer(i64),
    Float(f64),
}

impl From<i32> for ExitCode {
    fn from(code: i32) -> Self {
        Self::Integer(i64::from(code))
    }
}

impl PartialEq<i32> for ExitCode {
    fn eq(&self, other: &i32) -> bool {
        match self {
            Self::Integer(code) => *code == i64::from(*other),
            Self::Float(code) => *code == f64::from(*other),
        }
    }
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integer(code) => code.fmt(f),
            Self::Float(code) => code.fmt(f),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub output: String,
    pub exit_code: Option<ExitCode>,
    pub cancelled: bool,
    pub truncated: bool,
    pub full_output_path: Option<String>,
}

/// What a run needs besides its command.
pub struct Spawn<'a> {
    pub cwd: &'a str,
    pub shell: Option<&'a str>,
    pub prefix: Option<&'a str>,
    pub cancel: &'a CancelToken,
    pub on_chunk: &'a mut dyn FnMut(&str),
}

/// What a run's thread reports: the output as it arrives, then the result.
pub enum Update {
    Chunk(String),
    Done(Result<Outcome, String>),
}

pub enum Step {
    Started,
    Chunk(String),
    Answered(Outcome),
    Refused(String),
    Done(Result<Outcome, String>),
}

/// The run on its own thread, for a caller that polls while it works: the token that cancels it,
/// and the messages it sends.
pub fn start(
    command: &str,
    exclude_from_context: bool,
    cwd: &str,
    shell: Option<&str>,
    prefix: Option<&str>,
) -> (CancelToken, mpsc::Receiver<Update>) {
    spawn_run(
        command,
        exclude_from_context,
        cwd,
        shell,
        prefix,
        |step| match step {
            Step::Started => None,
            Step::Chunk(text) => Some(Update::Chunk(text)),
            Step::Answered(outcome) => Some(Update::Done(Ok(outcome))),
            Step::Refused(error) => Some(Update::Done(Err(error))),
            Step::Done(result) => Some(Update::Done(result)),
        },
    )
}

pub fn start_drawn(
    command: &str,
    exclude_from_context: bool,
    cwd: &str,
    shell: Option<&str>,
    prefix: Option<&str>,
) -> (CancelToken, mpsc::Receiver<Step>) {
    spawn_run(command, exclude_from_context, cwd, shell, prefix, Some)
}

fn spawn_run<T: Send + 'static>(
    command: &str,
    exclude_from_context: bool,
    cwd: &str,
    shell: Option<&str>,
    prefix: Option<&str>,
    message: fn(Step) -> Option<T>,
) -> (CancelToken, mpsc::Receiver<T>) {
    let token = CancelToken::new();
    let (tx, rx) = mpsc::channel();
    let abort = token.clone();
    let (command, cwd) = (command.to_owned(), cwd.to_owned());
    let (shell, prefix) = (shell.map(str::to_owned), prefix.map(str::to_owned));
    std::thread::spawn(move || {
        let emit = |step: Step| {
            if let Some(message) = message(step) {
                let _ = tx.send(message);
            }
        };
        run(
            &command,
            exclude_from_context,
            Spawn {
                cwd: &cwd,
                shell: shell.as_deref(),
                prefix: prefix.as_deref(),
                cancel: &abort,
                on_chunk: &mut |text| emit(Step::Chunk(text.to_owned())),
            },
            &emit,
        );
    });
    (token, rx)
}

fn run(command: &str, exclude_from_context: bool, spawn: Spawn<'_>, emit: &dyn Fn(Step)) {
    let decided = match crate::extensions::live() {
        Some(live) => live.user_bash(
            serde_json::json!({
                "command": command,
                "excludeFromContext": exclude_from_context,
                "cwd": spawn.cwd,
            }),
            &resolved(command, spawn.prefix),
            spawn.cancel,
            &mut || emit(Step::Started),
            &mut |delta| emit(Step::Chunk(delta)),
        ),
        None => UserBash::Local,
    };
    emit(match decided {
        UserBash::Answered(result) => Step::Answered(result),
        UserBash::Refused(error) => Step::Refused(error),
        UserBash::Ran(result) => Step::Done(result),
        UserBash::Local => {
            emit(Step::Started);
            Step::Done(execute(command, spawn))
        }
    });
}

fn resolved(command: &str, prefix: Option<&str>) -> String {
    match prefix.filter(|prefix| !prefix.is_empty()) {
        Some(prefix) => format!("{prefix}\n{command}"),
        None => command.to_owned(),
    }
}

/// `executeBashWithOperations`: run the command, hand each cleaned chunk to `on_chunk`, and keep
/// the last 50KB of it for the model.
pub fn execute(command: &str, spawn: Spawn<'_>) -> Result<Outcome, String> {
    let shell = shell_path(spawn.shell)?;
    if !std::path::Path::new(spawn.cwd).exists() {
        return Err(format!(
            "Working directory does not exist: {}\nCannot execute bash commands.",
            spawn.cwd
        ));
    }
    let resolved = resolved(command, spawn.prefix);
    let mut cmd = std::process::Command::new(&shell);
    cmd.arg("-c")
        .arg(&resolved)
        .current_dir(spawn.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0);
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
    let mut cleaner = Clean::new();
    let mut open = readers.len();
    let mut exit: Option<(Option<i32>, Instant)> = None;
    let mut killed = false;
    loop {
        if exit.is_none() {
            if !killed && spawn.cancel.is_cancelled() {
                kill_tree(pid);
                killed = true;
            }
            if let Ok(Some(status)) = child.try_wait() {
                exit = Some((status.code(), Instant::now()));
            }
        }
        if let Some((_, at)) = exit
            && (open == 0 || at.elapsed() >= EXIT_STDIO_GRACE)
        {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Some(bytes)) => {
                let text = cleaner.push(&bytes);
                if !text.is_empty() {
                    (spawn.on_chunk)(&text);
                }
            }
            Ok(None) => open -= 1,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => open = 0,
        }
    }
    while let Ok(Some(bytes)) = rx.try_recv() {
        let text = cleaner.push(&bytes);
        if !text.is_empty() {
            (spawn.on_chunk)(&text);
        }
    }
    let rest = cleaner.finish();
    if !rest.is_empty() {
        (spawn.on_chunk)(&rest);
    }
    let (output, truncated) = cleaner.tail();
    let cancelled = spawn.cancel.is_cancelled();
    Ok(Outcome {
        output,
        exit_code: if cancelled {
            None
        } else {
            exit.and_then(|(code, _)| code).map(ExitCode::from)
        },
        cancelled,
        truncated,
        full_output_path: cleaner.full_output_path(),
    })
}

/// `bashExecutionToText`: what the model reads in place of the message the run was stored as.
pub fn to_text(command: &str, outcome: &Outcome) -> String {
    let mut text = format!("Ran `{command}`\n");
    if outcome.output.is_empty() {
        text.push_str("(no output)");
    } else {
        text.push_str(&format!("```\n{}\n```", outcome.output));
    }
    if outcome.cancelled {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(code) = outcome.exit_code.filter(|code| *code != 0) {
        text.push_str(&format!("\n\nCommand exited with code {code}"));
    }
    if outcome.truncated
        && let Some(path) = &outcome.full_output_path
    {
        text.push_str(&format!("\n\n[Output truncated. Full output: {path}]"));
    }
    text
}

/// `recordBashResult`: the run as an entry, which the conversation applies now or at the next
/// boundary, as upstream defers it while the agent streams.
pub fn record(
    conversation: &Conversation,
    cancel: &CancelToken,
    command: &str,
    outcome: &Outcome,
    exclude_from_context: bool,
) -> Result<(), String> {
    conversation
        .submit(
            SubmissionDraft::Write {
                request_id: None,
                entry: entry(command, outcome, exclude_from_context),
            },
            cancel,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The entry a run is stored as: its data, and the message the model reads unless the entry is
/// kept out of the context.
pub fn entry(command: &str, outcome: &Outcome, exclude_from_context: bool) -> WriteDraft {
    WriteDraft {
        kind: KIND.to_owned(),
        model: (!exclude_from_context).then(|| vec![user_message(&to_text(command, outcome))]),
        data: Some(serde_json::json!({
            "command": command,
            "output": outcome.output,
            "exitCode": outcome.exit_code,
            "cancelled": outcome.cancelled,
            "truncated": outcome.truncated,
            "fullOutputPath": outcome.full_output_path,
            "excludeFromContext": exclude_from_context,
        })),
        head: None,
        edits: None,
    }
}

fn user_message(text: &str) -> Message {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default();
    Message::User(Arc::new(UserMessage {
        role: UserRole,
        content: UserContent::Blocks(vec![UserBlock::Text(TextContent {
            kind: TextType,
            text: text.to_owned(),
            text_signature: None,
        })]),
        timestamp,
    }))
}

/// The decoded, cleaned output still worth keeping, and the file holding all of it.
struct Clean {
    pending: Vec<u8>,
    /// An escape sequence a chunk boundary cut in two.
    escape: String,
    chunks: Vec<String>,
    rolling: usize,
    total: usize,
    /// The rolling tail starts in the middle of a line.
    mid_line: bool,
    temp: Option<(String, std::fs::File)>,
}

impl Clean {
    fn new() -> Self {
        Clean {
            pending: Vec::new(),
            escape: String::new(),
            chunks: Vec::new(),
            rolling: 0,
            total: 0,
            mid_line: false,
            temp: None,
        }
    }

    fn push(&mut self, data: &[u8]) -> String {
        self.pending.extend_from_slice(data);
        let decoded = self.decode(false);
        self.clean(&decoded)
    }

    fn finish(&mut self) -> String {
        let decoded = self.decode(true);
        let mut out = self.clean(&decoded);
        let held = std::mem::take(&mut self.escape);
        let rest = cleaned(&held);
        if !rest.is_empty() {
            self.append(&rest);
            out.push_str(&rest);
        }
        out
    }

    /// The last 2000 lines or 50KB of the run, and whether it was cut.
    fn tail(&mut self) -> (String, bool) {
        let full = self.chunks.concat();
        let text = if self.mid_line {
            full.split_once('\n').map_or("", |(_, rest)| rest)
        } else {
            full.as_str()
        };
        let truncation = truncate_tail(text, Limits::default());
        if truncation.truncated {
            self.ensure_temp();
            (truncation.content, true)
        } else {
            (text.to_owned(), false)
        }
    }

    fn full_output_path(&self) -> Option<String> {
        self.temp.as_ref().map(|(path, _)| path.clone())
    }

    fn decode(&mut self, end: bool) -> String {
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    out.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    out.push_str(std::str::from_utf8(&self.pending[..valid]).unwrap_or_default());
                    match (e.error_len(), end) {
                        (Some(len), _) => {
                            out.push('\u{fffd}');
                            self.pending.drain(..valid + len);
                        }
                        (None, false) => {
                            self.pending.drain(..valid);
                            break;
                        }
                        (None, true) => {
                            out.push_str(&String::from_utf8_lossy(&self.pending[valid..]));
                            self.pending.clear();
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    fn clean(&mut self, text: &str) -> String {
        let joined = std::mem::take(&mut self.escape) + text;
        let (complete, held) = split_incomplete_ansi_suffix(&joined);
        self.escape = held.to_owned();
        let cleaned = cleaned(complete);
        if !cleaned.is_empty() {
            self.append(&cleaned);
        }
        cleaned
    }

    fn append(&mut self, text: &str) {
        self.total += text.len();
        if self.total > DEFAULT_MAX_BYTES {
            self.ensure_temp();
        }
        if let Some((_, file)) = &mut self.temp {
            let _ = file.write_all(text.as_bytes());
        }
        self.rolling += text.len();
        self.chunks.push(text.to_owned());
        while self.rolling > ROLLING_BYTES && self.chunks.len() > 1 {
            let removed = self.chunks.remove(0);
            self.rolling -= removed.len();
            self.mid_line = !removed.ends_with('\n');
        }
    }

    fn ensure_temp(&mut self) {
        if self.temp.is_some() {
            return;
        }
        let path = crate::output_files::new_path("pi-bash", ".log");
        if let Ok(mut file) = crate::output_files::create(&path) {
            for chunk in &self.chunks {
                let _ = file.write_all(chunk.as_bytes());
            }
            self.temp = Some((path, file));
        }
    }
}

/// `sanitizeBinaryOutput(stripAnsi(text))` with carriage returns dropped.
fn cleaned(text: &str) -> String {
    pi_tui::utils::strip_terminal_sequences(text)
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\r' | '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}'
                    | '\u{fff9}'..='\u{fffb}'
            )
        })
        .collect()
}

/// `splitIncompleteAnsiSuffix`: the text, and the escape sequence at its end that needs the next
/// chunk to finish it.
fn split_incomplete_ansi_suffix(text: &str) -> (&str, &str) {
    let Some(start) = text.rfind('\x1b') else {
        return (text, "");
    };
    let rest = &text[start..];
    let complete = match rest.as_bytes().get(1) {
        None => false,
        Some(b'[') => rest[2..].bytes().any(|b| (0x40..=0x7e).contains(&b)),
        Some(b']') => rest[2..].contains('\x07') || rest[2..].contains("\x1b\\"),
        Some(_) => rest.len() >= 2,
    };
    if complete {
        (text, "")
    } else {
        (&text[..start], rest)
    }
}
