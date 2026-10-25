//! Upstream `core/extensions` and the extension half of `core/resource-loader.ts`: which paths
//! the `packages` setting, the `extensions` setting, `--extension` and the two standard
//! directories name, and the child that loads them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use pi_durable::ai::types::{
    AssistantMessage, Message, TextContent, TextType, UserBlock, UserContent,
};
use pi_durable::errors::Error;
use pi_durable::harness::events::AgentEvent;
use pi_durable::harness::generation::{GENERATION_HOOK, GenerationHooks};
use pi_durable::harness::types::{HookApi, HookRegistration};
use pi_extensions::{Call, Load};
use serde_json::{Value, json};

use crate::config::CONFIG_DIR_NAME;
use crate::paths::join;
use crate::settings::SettingsManager;

/// A command an extension registered, and the extension that registered it.
#[derive(Clone, Debug)]
pub struct ExtensionCommand {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Default)]
pub struct Extensions {
    pub commands: Vec<ExtensionCommand>,
    /// What loading has to say, one line each.
    pub notices: Vec<String>,
}

/// The message `sendUserMessage` asked for, as the harness takes one.
pub fn content_of(content: &Value) -> UserContent {
    let text = content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .or_else(|| content.as_str().map(str::to_owned))
        .unwrap_or_default();
    UserContent::Blocks(vec![UserBlock::Text(TextContent {
        kind: TextType,
        text,
        text_signature: None,
    })])
}

/// The text of a prompt that is one text block and images, as `input` handlers see it.
pub fn text_of(content: &UserContent) -> Option<String> {
    match content {
        UserContent::Blocks(blocks) => match blocks.first() {
            Some(UserBlock::Text(text)) => Some(text.text.clone()),
            _ => None,
        },
        UserContent::Text(text) => Some(text.clone()),
    }
}

pub fn with_text(content: UserContent, text: String) -> UserContent {
    match content {
        UserContent::Blocks(mut blocks) => {
            blocks[0] = UserBlock::Text(TextContent {
                kind: TextType,
                text,
                text_signature: None,
            });
            UserContent::Blocks(blocks)
        }
        UserContent::Text(_) => UserContent::Text(text),
    }
}

/// The mode an extension sees: upstream's `ExtensionMode`, which says "tui" when one is.
pub fn mode_name(mode: Option<crate::args::Mode>) -> &'static str {
    match mode {
        Some(crate::args::Mode::Rpc) => "rpc",
        Some(crate::args::Mode::Json) => "json",
        Some(crate::args::Mode::Text) => "print",
        None => "tui",
    }
}

/// The child's directory: `PI_EXTENSION_HOST`, else the copy the build installed beside the binary.
fn host_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_EXTENSION_HOST") {
        return Some(PathBuf::from(dir));
    }
    let installed = std::env::current_exe()
        .ok()?
        .parent()?
        .parent()?
        .join("share/pi-rs/extension-host");
    installed.is_dir().then_some(installed)
}

/// A `packages` entry as a path: upstream's `PackageSource`, a string or an object naming one.
fn package_paths(settings: &SettingsManager) -> Vec<String> {
    settings
        .packages()
        .into_iter()
        .filter_map(|entry| match entry {
            Value::String(path) => Some(path),
            Value::Object(fields) => fields
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn resolve(path: &str, cwd: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        join(cwd, path)
    }
}

/// Loads what the settings and the flags name; nothing at all when they name nothing.
pub fn load(
    cwd: &str,
    agent_dir: &str,
    cli: &[String],
    no_extensions: bool,
    settings: &SettingsManager,
    mode: &str,
    has_ui: bool,
) -> Extensions {
    let mut extensions = Extensions::default();
    let dirs = if no_extensions {
        Vec::new()
    } else {
        vec![
            join(cwd, &format!("{CONFIG_DIR_NAME}/extensions")),
            join(agent_dir, "extensions"),
        ]
    };
    let mut paths: Vec<String> = cli.to_vec();
    if !no_extensions {
        paths.extend(package_paths(settings));
        paths.extend(settings.extension_paths());
    }
    for path in cli {
        let resolved = resolve(path, cwd);
        if !Path::new(&resolved).exists() {
            extensions
                .notices
                .push(format!("Extension path does not exist: {resolved}"));
        }
    }
    if paths.is_empty() && !dirs.iter().any(|dir| Path::new(dir).is_dir()) {
        return extensions;
    }
    let Some(dir) = host_dir() else {
        extensions.notices.push(
            "Warning: no extension host is installed, so no extension was loaded; set PI_EXTENSION_HOST"
                .to_owned(),
        );
        return extensions;
    };
    let host = match Live::start(&dir, cwd, mode, has_ui) {
        Ok(host) => host,
        Err(error) => {
            extensions.notices.push(format!("Warning: {error}"));
            return extensions;
        }
    };
    let load = match host.load(agent_dir, &dirs, &paths) {
        Ok(load) => load,
        Err(error) => {
            extensions.notices.push(format!("Warning: {error}"));
            return extensions;
        }
    };
    for extension in &load.extensions {
        for command in &extension.commands {
            host.0
                .commands
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(command.name.clone());
        }
    }
    let _ = LIVE.set(host);
    for error in &load.errors {
        extensions.notices.push(format!("Warning: {}", error.error));
    }
    for extension in load.extensions {
        for member in &extension.unsupported {
            extensions.notices.push(format!(
                "Warning: {} uses {member}, which pi-rs does not implement yet",
                extension.path
            ));
        }
        for command in extension.commands {
            extensions.commands.push(ExtensionCommand {
                name: command.name,
                description: command.description,
                path: extension.path.clone(),
            });
        }
    }
    extensions
}

/// The child, alive as long as the process: one thread owns it, so a handler that answers waits on
/// that thread alone and a wedged one costs the caller its budget, never the TUI.
#[derive(Clone)]
pub struct Live(Arc<Inner>);

struct Inner {
    jobs: mpsc::Sender<Job>,
    calls: Mutex<Vec<Call>>,
    state: Mutex<pi_extensions::Ctx>,
    dead: AtomicBool,
    settled: AtomicBool,
    started: AtomicBool,
    commands: Mutex<Vec<String>>,
}

struct Job {
    op: Op,
    reply: mpsc::Sender<std::result::Result<Value, String>>,
}

enum Op {
    Load {
        agent_dir: String,
        dirs: Vec<String>,
        paths: Vec<String>,
    },
    Event {
        name: String,
        payload: Value,
    },
    Command {
        name: String,
        args: String,
    },
}

/// The child this process loads its extensions in, once main has found them.
static LIVE: OnceLock<Live> = OnceLock::new();

pub fn live() -> Option<&'static Live> {
    LIVE.get()
}

impl Live {
    fn start(dir: &Path, cwd: &str, mode: &str, has_ui: bool) -> std::result::Result<Self, String> {
        let host = pi_extensions::Host::start(dir, cwd)?;
        let (jobs, queue) = mpsc::channel();
        let inner = Arc::new(Inner {
            jobs,
            calls: Mutex::new(Vec::new()),
            state: Mutex::new(pi_extensions::Ctx {
                mode: mode.to_owned(),
                has_ui,
                cwd: cwd.to_owned(),
                streaming: false,
                pending: 0,
            }),
            dead: AtomicBool::new(false),
            settled: AtomicBool::new(false),
            started: AtomicBool::new(false),
            commands: Mutex::new(Vec::new()),
        });
        let worker = Arc::clone(&inner);
        std::thread::spawn(move || serve(host, &worker, queue));
        Ok(Self(inner))
    }

    /// What the next handler sees of the run: the session's cwd, whether a turn is going, and how
    /// many messages wait to be sent.
    pub fn set_state(&self, cwd: &str, streaming: bool, pending: u64) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cwd = cwd.to_owned();
        state.streaming = streaming;
        state.pending = pending;
    }

    /// Loads what the settings and the flags named, in the child that goes on to serve events.
    fn load(
        &self,
        agent_dir: &str,
        dirs: &[String],
        paths: &[String],
    ) -> std::result::Result<Load, String> {
        let value = self.dispatch(Op::Load {
            agent_dir: agent_dir.to_owned(),
            dirs: dirs.to_vec(),
            paths: paths.to_vec(),
        })?;
        serde_json::from_value(value).map_err(|e| format!("the extension host answered {e}"))
    }

    /// Hands one event to the extensions and waits for what they made of it.
    pub fn event(&self, name: &str, payload: Value) -> Option<Value> {
        let answer = self.ask(name, payload).ok();
        if name == "agent_end" {
            self.0.settled.store(true, Ordering::SeqCst);
        }
        answer
    }

    /// The durable events that are extension events too: a run starting and ending.
    pub fn observe(&self, event: &AgentEvent) {
        let name = match event {
            AgentEvent::RunStart { .. } => "agent_start",
            AgentEvent::RunEnd { .. } => "agent_end",
            _ => return,
        };
        let payload = match event {
            AgentEvent::RunEnd { .. } => json!({ "messages": [] }),
            _ => json!({}),
        };
        self.event(name, payload);
    }

    /// Upstream's `input`: what the user typed, which a handler may rewrite or swallow. `None`
    /// means a handler took it, so nothing is sent.
    pub fn input(&self, text: &str, source: &str) -> Option<String> {
        let answer = self
            .ask("input", json!({ "text": text, "source": source }))
            .ok()?;
        match answer.get("action").and_then(Value::as_str) {
            Some("handled") => None,
            Some("transform") => Some(
                answer
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or(text)
                    .to_owned(),
            ),
            _ => Some(text.to_owned()),
        }
    }

    /// Runs the command an extension registered, with the text after its name.
    pub fn command(&self, name: &str, args: &str) {
        let _ = self.dispatch(Op::Command {
            name: name.to_owned(),
            args: args.to_owned(),
        });
    }

    /// Runs the line as an extension command when it names one, as upstream's
    /// `_tryExecuteExtensionCommand` does: the prompt is then not sent to the model.
    pub fn run_command(&self, typed: &str) -> bool {
        let Some(typed) = typed.strip_prefix('/') else {
            return false;
        };
        let (name, args) = typed.split_once(' ').unwrap_or((typed, ""));
        let known = self
            .0
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|command| command == name);
        if !known {
            return false;
        }
        self.command(name, args);
        true
    }

    /// `agent_settled`, once a run has ended and nothing is left to send.
    pub fn settle(&self, aborted: bool) {
        let pending = self
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending;
        if pending == 0 && self.0.settled.swap(false, Ordering::SeqCst) {
            self.event("agent_settled", json!({ "aborted": aborted }));
        }
    }

    /// The calls the extensions made, in the order they made them.
    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut *self.0.calls.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The durable hooks: a streamed message, and one that may be replaced when it ends.
    pub fn hooks(&self) -> HookRegistration {
        let streamed = self.clone();
        let ended = self.clone();
        HookRegistration::new(
            &GENERATION_HOOK,
            GenerationHooks {
                on_partial: Some(Arc::new(move |_api: &HookApi, message| {
                    streamed.partial(message.as_deref())
                })),
                message_end: Some(Arc::new(move |_api: &HookApi, message: &Message| {
                    ended.replaced(message)
                })),
                ..GenerationHooks::default()
            },
        )
    }

    fn partial(&self, message: Option<&AssistantMessage>) {
        let Some(message) = message else {
            return;
        };
        let started = self.0.started.swap(true, Ordering::SeqCst);
        let Ok(payload) = serde_json::to_value(message) else {
            return;
        };
        if !started {
            self.event("message_start", json!({ "message": payload }));
        }
        self.event("message_update", json!({ "message": payload }));
    }

    fn replaced(&self, message: &Message) -> std::result::Result<Option<Message>, Error> {
        self.0.started.store(false, Ordering::SeqCst);
        let answer = self
            .ask("message_end", json!({ "message": message }))
            .map_err(Error::Invalid)?;
        let Some(next) = answer.get("message") else {
            return Ok(None);
        };
        let next = serde_json::from_value(next.clone())
            .map_err(|e| Error::Invalid(format!("a message_end handler returned {e}")))?;
        Ok(Some(next))
    }

    fn ask(&self, name: &str, payload: Value) -> std::result::Result<Value, String> {
        self.dispatch(Op::Event {
            name: name.to_owned(),
            payload,
        })
    }

    fn dispatch(&self, op: Op) -> std::result::Result<Value, String> {
        let (reply, answer) = mpsc::channel();
        self.0
            .jobs
            .send(Job { op, reply })
            .map_err(|_| "the extension host is closed".to_owned())?;
        answer
            .recv_timeout(pi_extensions::EVENT_BUDGET + Duration::from_secs(1))
            .map_err(|_| "the extension host did not answer".to_owned())?
    }
}

fn serve(mut host: pi_extensions::Host, inner: &Inner, queue: mpsc::Receiver<Job>) {
    while let Ok(Job { op, reply }) = queue.recv() {
        let ctx = inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let answer = match &op {
            Op::Load {
                agent_dir,
                dirs,
                paths,
            } => host
                .load(&ctx.cwd, agent_dir, dirs, paths, &ctx)
                .and_then(|load| {
                    serde_json::to_value(load)
                        .map(|value| pi_extensions::Reply {
                            value,
                            ..pi_extensions::Reply::default()
                        })
                        .map_err(|e| e.to_string())
                }),
            Op::Event { name, payload } => host.event(&pi_extensions::Event {
                name,
                payload: payload.clone(),
                ctx: &ctx,
            }),
            Op::Command { name, args } => host.command(name, args, &ctx),
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                if !inner.dead.swap(true, Ordering::SeqCst) {
                    inner.push(Call::Notify {
                        text: format!("Warning: {error}"),
                        level: "warning".to_owned(),
                    });
                }
                let _ = reply.send(Err(error));
                continue;
            }
        };
        for failed in &answer.errors {
            inner.push(Call::Notify {
                text: format!("Extension error ({}): {}", failed.path, failed.error),
                level: "error".to_owned(),
            });
        }
        for call in answer.calls {
            inner.push(call);
        }
        let _ = reply.send(Ok(answer.value));
    }
}

impl Inner {
    fn push(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(call);
    }
}
