use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use pi_rt::cancel::CancelToken;
use pi_rt::turn::pause;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{EVENT_BUDGET, LOAD_BUDGET, Load, ProjectTrust, ProviderInfo, ToolInfo};

/// How many of the child's stderr lines to keep for a failure message.
const STDERR_LINES: usize = 200;

const CANCEL_GRACE: Duration = Duration::from_secs(5);

const EXIT_GRACE: Duration = Duration::from_secs(1);

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

const SIGKILL: i32 = 9;

fn reap(child: &mut Child, groups: &mut HashSet<i32>) {
    let deadline = Instant::now() + EXIT_GRACE;
    let exited = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => break false,
        }
    };
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    for group in groups.drain() {
        unsafe { kill(-group, SIGKILL) };
    }
}

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

pub trait Peer: Send + Sync {
    /// Answers one call the child waits on; `update` hands it a part of the answer early, and
    /// `cancel` fires when the child stops waiting.
    fn ask(
        &self,
        call: &str,
        args: &Value,
        update: &mut dyn FnMut(Value),
        cancel: &CancelToken,
    ) -> Result<Value, String>;
    fn call(&self, call: Call);
    fn failed(&self, failed: Failed);
    fn tools(&self, tools: Vec<ToolInfo>);
    fn providers(&self, providers: Vec<ProviderInfo>);
    fn virtual_models(&self, models: Vec<Value>);
}

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

pub struct Sources<'a> {
    pub dirs: &'a [String],
    pub paths: &'a [String],
    pub cli: &'a [String],
    pub flags: &'a [(String, Option<String>)],
    pub trusted: bool,
    pub settings: &'a Value,
}

#[derive(Clone, Copy, Debug)]
enum Limit {
    Idle(Duration),
    Unbounded,
}

enum Incoming {
    Line(Value),
    Asking,
    Answered,
    Cancelled,
    Closed,
}

struct Shared {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
    routes: Mutex<Option<HashMap<u64, Sender<Incoming>>>>,
    failure: Mutex<Option<String>>,
    groups: Mutex<HashSet<i32>>,
    asks: Mutex<HashMap<u64, CancelToken>>,
    stderr: Arc<Mutex<Vec<String>>>,
    next: AtomicU64,
}

#[derive(Clone)]
pub struct Host(Arc<Shared>);

pub struct Pending {
    shared: Arc<Shared>,
    id: u64,
    limit: Limit,
    sender: Sender<Incoming>,
    receiver: Receiver<Incoming>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Host {
    /// Starts the child from `dir`, which holds `node`, `host.mjs` and `node_modules`.
    pub fn start(dir: &Path, cwd: &str, peer: Arc<dyn Peer>) -> Result<Self, String> {
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

        let collected = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&collected);
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut buffer = [0; 4096];
            let mut line = Vec::new();
            loop {
                let count = match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => count,
                };
                let bytes = &buffer[..count];
                let _ = std::io::stderr().lock().write_all(bytes);
                for part in bytes.split_inclusive(|byte| *byte == b'\n') {
                    line.extend_from_slice(part);
                    if part.ends_with(b"\n") {
                        keep(
                            &kept,
                            String::from_utf8_lossy(&line)
                                .trim_end_matches(['\r', '\n'])
                                .to_owned(),
                        );
                        line.clear();
                    }
                }
            }
            if !line.is_empty() {
                keep(&kept, String::from_utf8_lossy(&line).into_owned());
            }
        });

        let shared = Arc::new(Shared {
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(child),
            routes: Mutex::new(Some(HashMap::new())),
            failure: Mutex::new(None),
            groups: Mutex::new(HashSet::new()),
            asks: Mutex::new(HashMap::new()),
            stderr: collected,
            next: AtomicU64::new(1),
        });
        let reader = Arc::downgrade(&shared);
        std::thread::spawn(move || read(&reader, stdout, &peer));
        Ok(Self(shared))
    }

    pub fn failure(&self) -> Option<String> {
        lock(&self.0.failure).clone()
    }

    /// Loads `paths`, discovering what sits inside `dirs`: upstream's `discoverAndLoadExtensions`.
    pub fn load(
        &self,
        cwd: &str,
        agent_dir: &str,
        sources: &Sources<'_>,
        ctx: &Ctx,
    ) -> Result<Load, String> {
        let flags: Vec<Value> = sources
            .flags
            .iter()
            .map(|(name, value)| match value {
                Some(value) => json!([name, value]),
                None => json!([name, true]),
            })
            .collect();
        let request = json!({
            "op": "load",
            "cwd": cwd,
            "agentDir": agent_dir,
            "dirs": sources.dirs,
            "paths": sources.paths,
            "cli": sources.cli,
            "flags": flags,
            "mode": ctx.mode,
            "hasUI": ctx.has_ui,
            "trusted": sources.trusted,
            "settings": sources.settings,
            "updates": ctx.updates,
        });
        let value = self
            .begin(request, Limit::Idle(LOAD_BUDGET))?
            .wait(None, &mut |_| {})?;
        serde_json::from_value(value).map_err(|e| format!("the extension host answered {e}"))
    }

    pub fn project_trust(
        &self,
        cwd: &str,
        agent_dir: &str,
        sources: &Sources<'_>,
        ctx: &Ctx,
    ) -> Result<ProjectTrust, String> {
        let request = json!({
            "op": "projectTrust",
            "cwd": cwd,
            "agentDir": agent_dir,
            "dirs": sources.dirs,
            "paths": sources.paths,
            "cli": sources.cli,
            "settings": sources.settings,
            "mode": ctx.mode,
            "hasUI": ctx.has_ui,
            "updates": ctx.updates,
        });
        let value = self
            .begin(request, Limit::Idle(LOAD_BUDGET))?
            .wait(None, &mut |_| {})?;
        serde_json::from_value(value).map_err(|e| format!("the extension host answered {e}"))
    }

    /// Hands one event to the extensions, which may answer it and call back into pi-rs.
    pub fn event(&self, name: &str, payload: Value, ctx: &Ctx) -> Result<Pending, String> {
        let mut request = with_ctx(json!({ "op": "event", "name": name }), ctx);
        request["event"] = payload;
        self.begin(request, Limit::Idle(EVENT_BUDGET))
    }

    /// Runs a command an extension registered, with the arguments the user typed after it.
    pub fn command(&self, name: &str, args: &str, ctx: &Ctx) -> Result<Pending, String> {
        let request = with_ctx(json!({ "op": "command", "name": name, "args": args }), ctx);
        self.begin(request, Limit::Idle(EVENT_BUDGET))
    }

    pub fn tool(
        &self,
        name: &str,
        call_id: &str,
        params: Value,
        announced: Option<Value>,
        ctx: &Ctx,
    ) -> Result<Pending, String> {
        let mut request = with_ctx(
            json!({ "op": "tool", "name": name, "toolCallId": call_id }),
            ctx,
        );
        request["params"] = params;
        if let Some(args) = announced {
            request["announce"] = json!({ "args": args });
        }
        self.begin(request, Limit::Unbounded)
    }

    pub fn prepare(&self, name: &str, args: Value, ctx: &Ctx) -> Result<Pending, String> {
        let mut request = with_ctx(json!({ "op": "prepare", "name": name }), ctx);
        request["args"] = args;
        self.begin(request, Limit::Idle(EVENT_BUDGET))
    }

    pub fn user_bash(&self, event: Value, resolved: &str, ctx: &Ctx) -> Result<Pending, String> {
        let mut request = with_ctx(json!({ "op": "user_bash", "resolved": resolved }), ctx);
        request["event"] = event;
        self.begin(request, Limit::Unbounded)
    }

    /// Streams one request through the `streamSimple` of a provider an extension registered; each
    /// event arrives as an update.
    pub fn stream(
        &self,
        provider: &str,
        model: Value,
        context: Value,
        options: Value,
        hooks: bool,
    ) -> Result<Pending, String> {
        let request = json!({
            "op": "stream",
            "provider": provider,
            "model": model,
            "context": context,
            "options": options,
            "hooks": hooks,
        });
        self.begin(request, Limit::Unbounded)
    }

    pub fn route(&self, request: Value, ctx: &Ctx) -> Result<Pending, String> {
        let mut message = with_ctx(json!({ "op": "route" }), ctx);
        message["route"] = request;
        self.begin(message, Limit::Unbounded)
    }

    fn begin(&self, mut message: Value, limit: Limit) -> Result<Pending, String> {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = channel();
        {
            let mut routes = lock(&self.0.routes);
            let Some(routes) = routes.as_mut() else {
                return Err(self
                    .failure()
                    .unwrap_or_else(|| "the extension host is closed".to_owned()));
            };
            routes.insert(id, sender.clone());
        }
        let pending = Pending {
            shared: Arc::clone(&self.0),
            id,
            limit,
            sender,
            receiver,
        };
        message["id"] = json!(id);
        self.0.write(&message)?;
        Ok(pending)
    }
}

fn with_ctx(mut request: Value, ctx: &Ctx) -> Value {
    request["mode"] = json!(ctx.mode);
    request["hasUI"] = json!(ctx.has_ui);
    request["cwd"] = json!(ctx.cwd);
    request["streaming"] = json!(ctx.streaming);
    request["pending"] = json!(ctx.pending);
    request["updates"] = Value::Object(ctx.updates.clone());
    request
}

impl Pending {
    pub fn wait(
        self,
        cancel: Option<&CancelToken>,
        update: &mut dyn FnMut(Value),
    ) -> Result<Value, String> {
        let _cancelled = cancel.map(|token| {
            let sender = self.sender.clone();
            token.subscribe(move || {
                let _ = sender.send(Incoming::Cancelled);
            })
        });
        let mut heard = Instant::now();
        let mut asking = 0usize;
        let mut cancelled: Option<Instant> = None;
        loop {
            let idle = match self.limit {
                Limit::Idle(budget) if asking == 0 => Some(heard + budget),
                _ => None,
            };
            let grace = cancelled.map(|at| at + CANCEL_GRACE);
            let deadline = idle.into_iter().chain(grace).min();
            let incoming = match deadline {
                Some(deadline) => pause(|| {
                    self.receiver
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                }),
                None => pause(|| self.receiver.recv()).map_err(|_| RecvTimeoutError::Disconnected),
            };
            match incoming {
                Ok(Incoming::Line(value)) => {
                    heard = Instant::now();
                    if let Some(partial) = value.get("update") {
                        update(partial.clone());
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
                Ok(Incoming::Asking) => asking += 1,
                Ok(Incoming::Answered) => {
                    asking = asking.saturating_sub(1);
                    heard = Instant::now();
                }
                Ok(Incoming::Cancelled) => {
                    if cancelled.is_none() {
                        cancelled = Some(Instant::now());
                        self.shared.write(&json!({ "cancel": self.id }))?;
                    }
                }
                Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(lock(&self.shared.failure)
                        .clone()
                        .unwrap_or_else(|| "the extension host is closed".to_owned()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if grace.is_some_and(|grace| Instant::now() >= grace) {
                        return Err("the extension did not stop when cancelled".to_owned());
                    }
                    if let Limit::Idle(budget) = self.limit {
                        return Err(self.shared.stop(format!(
                            "the extension host did not answer within {}s",
                            budget.as_secs()
                        )));
                    }
                }
            }
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(routes) = lock(&self.shared.routes).as_mut() {
            routes.remove(&self.id);
        }
    }
}

impl Shared {
    fn write(&self, message: &Value) -> Result<(), String> {
        let mut stdin = lock(&self.stdin);
        let stdin = stdin.as_mut().ok_or("the extension host is closed")?;
        writeln!(stdin, "{message}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("cannot write to the extension host: {e}"))
    }

    fn route(&self, id: u64, incoming: Incoming) {
        if let Some(sender) = lock(&self.routes)
            .as_ref()
            .and_then(|routes| routes.get(&id))
        {
            let _ = sender.send(incoming);
        }
    }

    fn receive(self: &Arc<Self>, line: String, peer: &Arc<dyn Peer>) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            keep(&self.stderr, line);
            return;
        };
        let Some(call) = value.get("call").and_then(Value::as_str) else {
            if let Some(id) = value.get("id").and_then(Value::as_u64) {
                self.route(id, Incoming::Line(value));
            }
            return;
        };
        if let Some(call_id) = value.get("callId").cloned() {
            let request = value.get("request").and_then(Value::as_u64);
            if let Some(id) = request {
                self.route(id, Incoming::Asking);
            }
            let cancel = CancelToken::new();
            if let Some(key) = call_id.as_u64() {
                lock(&self.asks).insert(key, cancel.clone());
            }
            let shared = Arc::clone(self);
            let peer = Arc::clone(peer);
            let call = call.to_owned();
            std::thread::spawn(move || {
                let mut update = |part: Value| {
                    let _ = shared.write(&json!({ "answer": call_id, "update": part }));
                };
                let reply = match peer.ask(&call, &value, &mut update, &cancel) {
                    Ok(value) => json!({ "answer": call_id, "ok": true, "value": value }),
                    Err(error) => json!({ "answer": call_id, "ok": false, "error": error }),
                };
                if let Some(key) = call_id.as_u64() {
                    lock(&shared.asks).remove(&key);
                }
                let _ = shared.write(&reply);
                if let Some(id) = request {
                    shared.route(id, Incoming::Answered);
                }
            });
            return;
        }
        match call {
            "cancel" => {
                let asked = value.get("ask").and_then(Value::as_u64);
                if let Some(cancel) = asked.and_then(|key| lock(&self.asks).get(&key).cloned()) {
                    cancel.cancel();
                }
            }
            "providers" => {
                let providers = value.get("providers").cloned().unwrap_or(Value::Null);
                match serde_json::from_value(providers) {
                    Ok(providers) => peer.providers(providers),
                    Err(e) => peer.failed(Failed {
                        path: "registerProvider".to_owned(),
                        event: String::new(),
                        error: format!("the extension host sent providers pi-rs cannot read: {e}"),
                    }),
                }
            }
            "group" | "ungroup" => {
                let Some(pid) = value
                    .get("pid")
                    .and_then(Value::as_i64)
                    .and_then(|pid| i32::try_from(pid).ok())
                    .filter(|pid| *pid > 1)
                else {
                    return;
                };
                let mut groups = lock(&self.groups);
                if call == "group" {
                    groups.insert(pid);
                } else {
                    groups.remove(&pid);
                }
            }
            "error" => {
                if let Ok(failed) = serde_json::from_value(value) {
                    peer.failed(failed);
                }
            }
            "virtualModels" => {
                let models = match value.get("models") {
                    Some(Value::Array(models)) => models.clone(),
                    _ => Vec::new(),
                };
                peer.virtual_models(models);
            }
            "tools" => {
                let tools = value.get("tools").cloned().unwrap_or(Value::Null);
                match serde_json::from_value(tools) {
                    Ok(tools) => peer.tools(tools),
                    Err(e) => peer.failed(Failed {
                        path: "registerTool".to_owned(),
                        event: String::new(),
                        error: format!("the extension host sent tools pi-rs cannot read: {e}"),
                    }),
                }
            }
            _ => {
                if let Some(call) = call_of(&value) {
                    peer.call(call);
                }
            }
        }
    }

    fn stop(&self, reason: String) -> String {
        let message = {
            let mut failure = lock(&self.failure);
            if failure.is_none() {
                let tail = lock(&self.stderr);
                *failure = Some(match tail.len() {
                    0 => reason,
                    _ => format!("{reason}: {}", tail.join("; ")),
                });
            }
            failure.clone().unwrap_or_default()
        };
        let routes = lock(&self.routes).take();
        for sender in routes.into_iter().flat_map(HashMap::into_values) {
            let _ = sender.send(Incoming::Closed);
        }
        for (_, cancel) in lock(&self.asks).drain() {
            cancel.cancel();
        }
        *lock(&self.stdin) = None;
        reap(&mut lock(&self.child), &mut lock(&self.groups));
        message
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        *lock(&self.stdin) = None;
        reap(&mut lock(&self.child), &mut lock(&self.groups));
    }
}

fn read(shared: &Weak<Shared>, stdout: ChildStdout, peer: &Arc<dyn Peer>) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        let Some(shared) = shared.upgrade() else {
            return;
        };
        shared.receive(line, peer);
    }
    if let Some(shared) = shared.upgrade() {
        let status = lock(&shared.child)
            .wait()
            .map(|status| status.to_string())
            .unwrap_or_else(|e| e.to_string());
        shared.stop(format!("the extension host exited ({status})"));
    }
}

/// Extension code writes on fd 1 too, as `stdio: "inherit"` children of it do.
fn keep(kept: &Mutex<Vec<String>>, line: String) {
    let mut kept = lock(kept);
    kept.push(line);
    if kept.len() > STDERR_LINES {
        kept.remove(0);
    }
}

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
