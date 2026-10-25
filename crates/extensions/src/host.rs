//! The child process: newline JSON in, one answer out per request, a budget on each.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{LOAD_BUDGET, Load};

/// How many of the child's stderr lines to keep for a failure message.
const STDERR_LINES: usize = 200;

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
    ) -> Result<Load, String> {
        let request =
            json!({"op": "load", "cwd": cwd, "agentDir": agent_dir, "dirs": dirs, "paths": paths});
        let answer = self.request(request, LOAD_BUDGET)?;
        serde_json::from_value(answer).map_err(|e| format!("the extension host answered {e}"))
    }

    fn request(&mut self, mut message: Value, budget: Duration) -> Result<Value, String> {
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
                        return Err(format!("the extension host asked for {call}"));
                    }
                    if value.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if value.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(value.get("value").cloned().unwrap_or(Value::Null));
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

impl Drop for Host {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
