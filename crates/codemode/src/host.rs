//! The host side of one execution (`packages/codemode/src/runtime/execution.ts` and `host.ts`).
//!
//! The host owns everything the script must not influence: the tool table, call records, output,
//! the deadline, the caller's abort, and the decoding of what the child reports. The child owns the
//! VM; the two exchange only the messages in [crate::protocol].
//!
//! One script runs in one child process, and a deadline or an abort kills that child, which is how a
//! script that computes without ever awaiting a tool is stopped: QuickJS's interrupts are
//! cooperative, so a spinning script never yields to a timer.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::protocol::{
    CallTarget, DeclaredGlobal, DeclaredTool, HostToWorker, OutputItem, VmStartData, WorkerToHost,
};

/// How often the host checks whether the caller aborted while the script runs.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// Runs one nested call: its name and arguments (already parsed from JSON) to its result, which
/// crosses back to the script as JSON. `Err` is the message the script's `Error` gets.
pub type ToolExecute<'a> = &'a mut dyn FnMut(&str, Option<&Value>) -> Result<Option<Value>, String>;

pub struct ExecuteOptions<'a> {
    /// Deadline for the whole script, including tool calls. `None` disables it.
    pub timeout_ms: Option<u64>,
    pub memory_limit_bytes: Option<usize>,
    pub cancelled: Option<&'a dyn Fn() -> bool>,
    pub execute: ToolExecute<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    Ok,
    Error,
    Cancelled,
}

impl CallStatus {
    pub fn name(self) -> &'static str {
        match self {
            CallStatus::Ok => "ok",
            CallStatus::Error => "error",
            CallStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NestedCall {
    pub name: String,
    pub status: CallStatus,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct StoreWrites {
    pub set: Map<String, Value>,
    pub delete: Vec<String>,
}

impl StoreWrites {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.delete.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The script threw or failed to parse.
    Script,
    /// The deadline expired; the child was killed.
    Timeout,
    /// The caller aborted; the child was killed.
    Aborted,
    /// The child failed outside the script's control.
    Sandbox,
}

impl ErrorKind {
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Script => "script",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Aborted => "aborted",
            ErrorKind::Sandbox => "sandbox",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub name: Option<String>,
    pub message: String,
    pub stack: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub ok: bool,
    pub value: Option<Value>,
    pub output: Vec<OutputItem>,
    pub calls: Vec<NestedCall>,
    pub store_writes: StoreWrites,
    pub error: Option<ScriptError>,
}

impl ScriptResult {
    fn failed(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            output: Vec::new(),
            calls: Vec::new(),
            store_writes: StoreWrites::default(),
            error: Some(ScriptError {
                kind,
                name: None,
                message: message.into(),
                stack: None,
            }),
        }
    }
}

/// Kills and reaps the child however the execution ends, so no `--codemode-vm` process outlives it.
struct SandboxChild {
    child: Child,
}

impl Drop for SandboxChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl SandboxChild {
    fn spawn(vm: &Path) -> std::io::Result<Self> {
        let child = Command::new(vm)
            .arg("--codemode-vm")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self { child })
    }

    fn stderr(&mut self) -> String {
        let mut text = String::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            let _ = stderr.read_to_string(&mut text);
        }
        text.trim().to_owned()
    }
}

/// Run one script in a fresh child of `vm` (the pi-rs binary, which has the hidden mode).
pub fn execute(
    vm: &Path,
    code: &str,
    tools: &[DeclaredTool],
    globals: &[DeclaredGlobal],
    store: &BTreeMap<String, String>,
    options: ExecuteOptions<'_>,
) -> ScriptResult {
    let mut sandbox = match SandboxChild::spawn(vm) {
        Ok(sandbox) => sandbox,
        Err(error) => {
            return ScriptResult::failed(
                ErrorKind::Sandbox,
                format!("Failed to start the codemode sandbox: {error}"),
            );
        }
    };
    let start = VmStartData {
        code: code.to_owned(),
        tools: tools.to_vec(),
        globals: globals.to_vec(),
        memory_limit_bytes: options.memory_limit_bytes,
        store: store.clone(),
    };
    let stdin = sandbox.child.stdin.take();
    let Some(mut stdin) = stdin else {
        return ScriptResult::failed(ErrorKind::Sandbox, "The sandbox has no stdin");
    };
    let line = start.to_line();
    if let Err(error) = stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush())
    {
        let stderr = sandbox.stderr();
        return ScriptResult::failed(
            ErrorKind::Sandbox,
            format!("Failed to start the script: {error} {stderr}")
                .trim_end()
                .to_owned(),
        );
    }
    let stdout = sandbox.child.stdout.take();
    let Some(stdout) = stdout else {
        return ScriptResult::failed(ErrorKind::Sandbox, "The sandbox has no stdout");
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let parsed = match line {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => serde_json::from_str::<WorkerToHost>(&line)
                    .map_err(|error| format!("bad message from the sandbox: {error}: {line}")),
                Err(error) => Err(format!("failed to read the sandbox's output: {error}")),
            };
            let failed = parsed.is_err();
            if sender.send(parsed).is_err() || failed {
                return;
            }
        }
    });

    let deadline = options
        .timeout_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    let mut output = Vec::new();
    let mut calls: Vec<NestedCall> = Vec::new();
    loop {
        if options.cancelled.is_some_and(|cancelled| cancelled()) {
            return ScriptResult::failed(ErrorKind::Aborted, "Execution aborted");
        }
        let wait = match deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return ScriptResult::failed(
                        ErrorKind::Timeout,
                        format!(
                            "Execution timed out after {} ms",
                            options.timeout_ms.unwrap_or_default()
                        ),
                    );
                }
                if options.cancelled.is_some() {
                    left.min(CANCEL_POLL)
                } else {
                    left
                }
            }
            None => CANCEL_POLL,
        };
        let message = match receiver.recv_timeout(wait) {
            Ok(Ok(message)) => message,
            Ok(Err(error)) => {
                let stderr = sandbox.stderr();
                return ScriptResult::failed(
                    ErrorKind::Sandbox,
                    format!("{error} {stderr}").trim_end().to_owned(),
                );
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                let stderr = sandbox.stderr();
                return ScriptResult::failed(
                    ErrorKind::Sandbox,
                    format!("The sandbox exited before the script settled. {stderr}")
                        .trim_end()
                        .to_owned(),
                );
            }
        };
        match message {
            WorkerToHost::Output { item } => output.push(item),
            WorkerToHost::Call {
                id,
                target,
                name,
                args,
            } => {
                let parsed: Option<Value> = match args {
                    Some(args) => match serde_json::from_str(&args) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            return ScriptResult::failed(
                                ErrorKind::Sandbox,
                                format!("call arguments are not valid JSON: {error}"),
                            );
                        }
                    },
                    None => None,
                };
                let record = (target == CallTarget::Tool).then(|| NestedCall {
                    name: name.clone(),
                    status: CallStatus::Cancelled,
                    duration_ms: 0.0,
                });
                let index = record.is_some().then(|| {
                    calls.push(record.clone().expect("a record"));
                    calls.len() - 1
                });
                let started = Instant::now();
                let outcome = (options.execute)(&name, parsed.as_ref());
                let reply = match outcome {
                    Ok(value) => HostToWorker::Result {
                        id,
                        ok: true,
                        payload: value.map(|value| value.to_string()),
                        properties: None,
                    },
                    Err(message) => HostToWorker::Result {
                        id,
                        ok: false,
                        payload: Some(message),
                        properties: None,
                    },
                };
                if let Err(error) = stdin
                    .write_all(reply.to_line().as_bytes())
                    .and_then(|()| stdin.flush())
                {
                    let stderr = sandbox.stderr();
                    return ScriptResult::failed(
                        ErrorKind::Sandbox,
                        format!("failed to answer a call: {error} {stderr}")
                            .trim_end()
                            .to_owned(),
                    );
                }
                if let Some(index) = index {
                    calls[index].status = if reply_ok(&reply) {
                        CallStatus::Ok
                    } else {
                        CallStatus::Error
                    };
                    calls[index].duration_ms = started.elapsed().as_secs_f64() * 1000.0;
                }
            }
            WorkerToHost::Done {
                ok,
                value,
                writes,
                error,
            } => {
                if !ok {
                    let error = match parse_script_error(error.as_deref().unwrap_or_default()) {
                        Ok(error) => error,
                        Err(message) => {
                            return ScriptResult::failed(ErrorKind::Sandbox, message);
                        }
                    };
                    return ScriptResult {
                        ok: false,
                        value: None,
                        output,
                        calls,
                        store_writes: StoreWrites::default(),
                        error: Some(error),
                    };
                }
                let value = match value {
                    Some(value) => match serde_json::from_str(&value) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            return ScriptResult::failed(
                                ErrorKind::Sandbox,
                                format!("return value is not valid JSON: {error}"),
                            );
                        }
                    },
                    None => None,
                };
                let store_writes = match parse_store_writes(writes.as_deref().unwrap_or("[]")) {
                    Ok(writes) => writes,
                    Err(message) => return ScriptResult::failed(ErrorKind::Sandbox, message),
                };
                return ScriptResult {
                    ok: true,
                    value,
                    output,
                    calls,
                    store_writes,
                    error: None,
                };
            }
            WorkerToHost::Crash { message } => {
                return ScriptResult {
                    ok: false,
                    value: None,
                    output,
                    calls,
                    store_writes: StoreWrites::default(),
                    error: Some(ScriptError {
                        kind: ErrorKind::Sandbox,
                        name: None,
                        message,
                        stack: None,
                    }),
                };
            }
        }
    }
}

fn reply_ok(reply: &HostToWorker) -> bool {
    matches!(reply, HostToWorker::Result { ok: true, .. })
}

/// Payloads that do not decode are reported as a broken bridge, as upstream's host does.
fn parse_store_writes(json: &str) -> Result<StoreWrites, String> {
    let entries: Value = serde_json::from_str(json)
        .map_err(|error| format!("store writes are not valid JSON: {error}"))?;
    let Value::Array(entries) = entries else {
        return Err("store writes are not an array".to_owned());
    };
    let mut writes = StoreWrites::default();
    for entry in entries {
        let Value::Array(parts) = entry else {
            return Err("store writes contain a malformed entry".to_owned());
        };
        let key = parts.first().and_then(Value::as_str);
        let valid = match (key, parts.len()) {
            (Some(_), 1) => true,
            (Some(_), 2) => parts[1].is_string(),
            _ => false,
        };
        let Some(key) = key else {
            return Err("store writes contain a malformed entry".to_owned());
        };
        if !valid {
            return Err("store writes contain a malformed entry".to_owned());
        }
        match parts.get(1) {
            Some(value) => {
                let value: Value = serde_json::from_str(value.as_str().unwrap_or_default())
                    .map_err(|error| {
                        format!("store value for \"{key}\" is not valid JSON: {error}")
                    })?;
                writes.set.insert(key.to_owned(), value);
            }
            None => writes.delete.push(key.to_owned()),
        }
    }
    Ok(writes)
}

fn parse_script_error(json: &str) -> Result<ScriptError, String> {
    let parsed: Value = serde_json::from_str(json)
        .map_err(|error| format!("script error is not valid JSON: {error}"))?;
    let Value::Object(fields) = parsed else {
        return Err("script error is not an object".to_owned());
    };
    let message = match fields.get("message") {
        Some(Value::String(message)) => message.clone(),
        _ => return Err("script error is malformed".to_owned()),
    };
    let optional = |key: &str| -> Result<Option<String>, String> {
        match fields.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err("script error is malformed".to_owned()),
        }
    };
    Ok(ScriptError {
        kind: ErrorKind::Script,
        name: optional("name")?,
        message,
        stack: optional("stack")?,
    })
}
