//! The child process: newline JSON in, one answer out per request, a budget on each.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use serde::Deserialize;

use crate::{EVENT_BUDGET, LOAD_BUDGET, Load};

/// How many of the child's stderr lines to keep for a failure message.
const STDERR_LINES: usize = 200;

/// What an extension asked pi-rs to do while it was answering.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Abort,
    Notify {
        text: String,
        level: String,
    },
    SendUserMessage {
        content: Value,
        deliver_as: Option<String>,
    },
    Shutdown,
    /// A member of the API pi-rs does not implement yet.
    Unsupported {
        member: String,
    },
}

/// A handler that threw, as upstream's `emitError` reports it.
#[derive(Clone, Debug, Deserialize)]
pub struct Failed {
    pub path: String,
    #[serde(default)]
    pub event: String,
    pub error: String,
}

/// One answer: what the event decided, and the calls the extension made on the way.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub value: Value,
    pub calls: Vec<Call>,
    pub errors: Vec<Failed>,
}

/// What the child asked pi-rs for while a handler was waiting on it: the call's name and arguments,
/// answered so the handler can go on.
pub type Ask<'a> = &'a mut dyn FnMut(&str, &Value) -> Result<Value, String>;

/// The run an event sees: upstream's `ExtensionContext` minus what pi-rs fills in per extension.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub mode: String,
    pub has_ui: bool,
    pub cwd: String,
    pub streaming: bool,
    pub pending: u64,
    pub updates: Map<String, Value>,
}

/// One event to hand the extensions: its name, its payload and the run it belongs to.
#[derive(Clone, Debug)]
pub struct Event<'a> {
    pub name: &'a str,
    pub payload: Value,
    pub ctx: &'a Ctx,
}

pub struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Result<String, ()>>,
    stderr: Arc<Mutex<Vec<String>>>,
    next: u64,
    failure: Option<String>,
}

impl Host {
    /// Starts the child from `dir`, which holds `node`, `host.mjs` and `node_modules`.
    pub fn start(dir: &Path, cwd: &str) -> Result<Self, String> {
        let program = dir.join("node");
        let mut child = Command::new(&program)
            .arg("--disable-warning=MODULE_TYPELESS_PACKAGE_JSON")
            .arg("--disable-warning=ExperimentalWarning")
            .arg("--experimental-transform-types")
            .arg(dir.join("host.mjs"))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start the extension host {}: {e}", program.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or("the extension host has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("the extension host has no stdout")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("the extension host has no stderr")?;

        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(Ok(line)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sender.send(Err(()));
        });

        let collected = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&collected);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                kept.push(line);
                if kept.len() > STDERR_LINES {
                    kept.remove(0);
                }
            }
        });

        Ok(Self {
            child,
            stdin: Some(stdin),
            lines,
            stderr: collected,
            next: 1,
            failure: None,
        })
    }

    /// Loads `paths`, discovering what sits inside `dirs`: upstream's `discoverAndLoadExtensions`.
    pub fn load(
        &mut self,
        cwd: &str,
        agent_dir: &str,
        dirs: &[String],
        paths: &[String],
        ctx: &Ctx,
    ) -> Result<Load, String> {
        let request = json!({
            "op": "load",
            "cwd": cwd,
            "agentDir": agent_dir,
            "dirs": dirs,
            "paths": paths,
            "mode": ctx.mode,
            "hasUI": ctx.has_ui,
            "trusted": true,
            "updates": ctx.updates,
        });
        let reply = self.answer(request, LOAD_BUDGET, &mut |_, _| {
            Err("the extensions cannot call pi-rs while they are loading".to_owned())
        })?;
        serde_json::from_value(reply.value).map_err(|e| format!("the extension host answered {e}"))
    }

    /// Hands one event to the extensions, which may answer it and call back into pi-rs.
    pub fn event(&mut self, event: &Event<'_>, ask: Ask<'_>) -> Result<Reply, String> {
        let request = json!({
            "op": "event",
            "name": event.name,
            "event": event.payload,
            "mode": event.ctx.mode,
            "hasUI": event.ctx.has_ui,
            "cwd": event.ctx.cwd,
            "streaming": event.ctx.streaming,
            "pending": event.ctx.pending,
            "updates": event.ctx.updates,
        });
        self.answer(request, EVENT_BUDGET, ask)
    }

    /// Runs a command an extension registered, with the arguments the user typed after it.
    pub fn command(
        &mut self,
        name: &str,
        args: &str,
        ctx: &Ctx,
        ask: Ask<'_>,
    ) -> Result<Reply, String> {
        let request = json!({
            "op": "command",
            "name": name,
            "args": args,
            "mode": ctx.mode,
            "hasUI": ctx.has_ui,
            "cwd": ctx.cwd,
            "streaming": ctx.streaming,
            "pending": ctx.pending,
            "updates": ctx.updates,
        });
        self.answer(request, EVENT_BUDGET, ask)
    }

    /// Sends `message` and reads the answer together with the calls that came with it.
    fn answer(&mut self, message: Value, budget: Duration, ask: Ask<'_>) -> Result<Reply, String> {
        let value = self.request(message, budget, ask)?;
        Ok(Reply {
            value: value.get("value").cloned().unwrap_or(Value::Null),
            calls: calls_of(value.get("calls")),
            errors: errors_of(value.get("errors")),
        })
    }

    fn request(
        &mut self,
        mut message: Value,
        budget: Duration,
        ask: Ask<'_>,
    ) -> Result<Value, String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let id = self.next;
        self.next += 1;
        message["id"] = json!(id);
        self.write(&message)?;
        loop {
            match self.lines.recv_timeout(budget) {
                Ok(Ok(line)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        self.note(line);
                        continue;
                    };
                    if let Some(call) = value.get("call").and_then(Value::as_str) {
                        let id = value.get("callId").cloned().unwrap_or(Value::Null);
                        let reply = match ask(call, &value) {
                            Ok(value) => json!({ "answer": id, "ok": true, "value": value }),
                            Err(error) => json!({ "answer": id, "ok": false, "error": error }),
                        };
                        self.write(&reply)?;
                        continue;
                    }
                    if value.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if value.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(value);
                    }
                    let error = value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("no error message");
                    return Err(error.to_owned());
                }
                Ok(Err(())) => return Err(self.exited()),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(self.stop(format!(
                        "the extension host did not answer within {}s",
                        budget.as_secs()
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => return Err(self.exited()),
            }
        }
    }

    fn write(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("the extension host is closed")?;
        writeln!(stdin, "{message}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("cannot write to the extension host: {e}"))
    }

    /// Extension code writes on fd 1 too, as `stdio: "inherit"` children of it do.
    fn note(&self, line: String) {
        let mut kept = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        kept.push(line);
        if kept.len() > STDERR_LINES {
            kept.remove(0);
        }
    }

    fn exited(&mut self) -> String {
        let status = self
            .child
            .wait()
            .map(|status| status.to_string())
            .unwrap_or_else(|e| e.to_string());
        self.stop(format!("the extension host exited ({status})"))
    }

    fn stop(&mut self, reason: String) -> String {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let message = match tail.len() {
            0 => reason,
            _ => format!("{reason}: {}", tail.join("; ")),
        };
        drop(tail);
        self.failure = Some(message.clone());
        message
    }
}

/// One call the extension made while a request was being served.
fn call_of(value: &Value) -> Option<Call> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match value.get("call").and_then(Value::as_str)? {
        "abort" => Some(Call::Abort),
        "notify" => Some(Call::Notify {
            text: text("text"),
            level: text("level"),
        }),
        "sendUserMessage" => Some(Call::SendUserMessage {
            content: value.get("content").cloned().unwrap_or(Value::Null),
            deliver_as: value
                .get("deliverAs")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        "shutdown" => Some(Call::Shutdown),
        "unsupported" => Some(Call::Unsupported {
            member: text("member"),
        }),
        _ => None,
    }
}

fn calls_of(value: Option<&Value>) -> Vec<Call> {
    value
        .and_then(Value::as_array)
        .map(|calls| calls.iter().filter_map(call_of).collect())
        .unwrap_or_default()
}

fn errors_of(value: Option<&Value>) -> Vec<Failed> {
    value
        .and_then(Value::as_array)
        .map(|errors| {
            errors
                .iter()
                .filter_map(|error| serde_json::from_value(error.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
