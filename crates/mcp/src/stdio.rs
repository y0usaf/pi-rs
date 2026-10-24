//! `transports/stdio.ts`: a server as a child process, one JSON-RPC message per line.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::transport::{DEFAULT_MAX_MESSAGE_BYTES, Transport};

const MAX_STDERR_BYTES: usize = 64 * 1024;
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_millis(2_000);
const STDIN_CLOSE_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct StdioTransportOptions {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    /// Set on top of the inherited environment.
    pub env: Vec<(String, String)>,
    pub max_message_bytes: usize,
    /// How long the server gets to exit after stdin closes, before SIGKILL.
    pub close_timeout: Duration,
}

impl StdioTransportOptions {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: ".".into(),
            env: Vec::new(),
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            close_timeout: DEFAULT_CLOSE_TIMEOUT,
        }
    }
}

enum Incoming {
    Message(Value),
    Error(String),
    Closed,
}

pub struct StdioTransport {
    options: StdioTransportOptions,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    incoming: Option<Receiver<Incoming>>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    stderr_thread: Option<std::thread::JoinHandle<()>>,
    started: bool,
    closed: bool,
}

impl StdioTransport {
    pub fn new(options: StdioTransportOptions) -> Self {
        Self {
            options,
            child: None,
            stdin: None,
            incoming: None,
            stderr_tail: Arc::new(Mutex::new(Vec::new())),
            stderr_thread: None,
            started: false,
            closed: false,
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    fn start(&mut self) -> Result<()> {
        if self.started {
            return Err(Error::transport("MCP stdio transport already started"));
        }
        if self.closed {
            return Err(Error::closed("MCP connection closed"));
        }
        self.started = true;
        let mut command = Command::new(&self.options.command);
        command
            .args(&self.options.args)
            .current_dir(&self.options.cwd)
            .envs(self.options.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| {
            Error::transport(format!("spawn {}: {error}", self.options.command))
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        self.stdin = child.stdin.take();
        self.child = Some(child);
        let (sender, receiver) = channel();
        self.incoming = Some(receiver);
        if let Some(stdout) = stdout {
            let max = self.options.max_message_bytes;
            std::thread::spawn(move || {
                read_messages(BufReader::new(stdout), max, &sender);
            });
        }
        if let Some(stderr) = stderr {
            let tail = Arc::clone(&self.stderr_tail);
            self.stderr_thread = Some(std::thread::spawn(move || read_stderr(stderr, &tail)));
        }
        Ok(())
    }
}

fn read_messages(reader: impl Read, max: usize, sender: &std::sync::mpsc::Sender<Incoming>) {
    let mut reader = BufReader::new(reader);
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => {
                if !line.is_empty() {
                    deliver(&line, max, sender);
                }
                let _ = sender.send(Incoming::Closed);
                return;
            }
            Ok(_) => {
                deliver(&line, max, sender);
            }
            Err(error) => {
                let _ = sender.send(Incoming::Error(error.to_string()));
                let _ = sender.send(Incoming::Closed);
                return;
            }
        }
    }
}

fn deliver(line: &[u8], max: usize, sender: &std::sync::mpsc::Sender<Incoming>) {
    if line.len() > max {
        let _ = sender.send(Incoming::Error(format!(
            "MCP stdio message exceeds {max} bytes"
        )));
        return;
    }
    let text = String::from_utf8_lossy(line);
    let text = text.trim_end_matches(['\r', '\n']);
    if text.trim().is_empty() {
        return;
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value) => {
            let _ = sender.send(Incoming::Message(value));
        }
        Err(error) => {
            let _ = sender.send(Incoming::Error(error.to_string()));
        }
    }
}

fn read_stderr(mut stderr: impl Read, tail: &Mutex<Vec<u8>>) {
    let mut buffer = [0u8; 8 * 1024];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                if let Ok(mut tail) = tail.lock() {
                    tail.extend_from_slice(&buffer[..read]);
                    if tail.len() > MAX_STDERR_BYTES {
                        let excess = tail.len() - MAX_STDERR_BYTES;
                        tail.drain(..excess);
                    }
                }
            }
        }
    }
}

impl Transport for StdioTransport {
    fn start(&mut self) -> Result<()> {
        StdioTransport::start(self)
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(Error::closed("MCP connection closed"));
        };
        if self.closed {
            return Err(Error::closed("MCP connection closed"));
        }
        let mut payload =
            serde_json::to_vec(message).map_err(|e| Error::transport(e.to_string()))?;
        payload.push(b'\n');
        stdin
            .write_all(&payload)
            .and_then(|()| stdin.flush())
            .map_err(|error| Error::closed(format!("MCP connection closed: {error}")))
    }

    fn next_message(&mut self, timeout: Duration) -> Result<Value> {
        let Some(incoming) = self.incoming.as_ref() else {
            return Err(Error::closed("MCP connection closed"));
        };
        match incoming.recv_timeout(timeout) {
            Ok(Incoming::Message(value)) => Ok(value),
            Ok(Incoming::Error(error)) => Err(Error::transport(error)),
            Ok(Incoming::Closed) => Err(Error::closed("MCP connection closed")),
            Err(RecvTimeoutError::Timeout) => Err(Error::Timeout(timeout.as_millis() as u64)),
            Err(RecvTimeoutError::Disconnected) => Err(Error::closed("MCP connection closed")),
        }
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.stdin = None;
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let grace = self.options.close_timeout.min(STDIN_CLOSE_GRACE);
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        // The server's last words are usually written as it dies.
        if let Some(handle) = self.stderr_thread.take() {
            for _ in 0..20 {
                if handle.is_finished() {
                    let _ = handle.join();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn stderr_tail(&self) -> Option<String> {
        let tail = self.stderr_tail.lock().ok()?;
        let text = String::from_utf8_lossy(&tail).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.close();
    }
}
