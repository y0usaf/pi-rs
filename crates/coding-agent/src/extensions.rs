//! Upstream `core/extensions` and the extension half of `core/resource-loader.ts`: which paths
//! the `packages` setting, the `extensions` setting, `--extension` and the two standard
//! directories name, and the child that loads them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use pi_durable::ai::models::Models as ModelsTrait;
use pi_durable::ai::registry::Providers;
use pi_durable::ai::types::{
    AssistantMessage, CacheRetention, Context, Message, Model, SimpleStreamOptions, SystemContent,
    SystemMessage, SystemRole, TextContent, TextType, Tool as AgentTool, TranscriptContext,
    UserBlock, UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::errors::Error;
use pi_durable::harness::events::AgentEvent;
use pi_durable::harness::generation::{GENERATION_HOOK, GenerationHooks};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::output::OutputChunk;
use pi_durable::harness::registry::Registry;
use pi_durable::harness::types::{
    HookApi, HookRegistration, OutputLimitsSpec, ToolControl, ToolExecute, ToolExecutionApi,
    ToolExecutionMode, ToolExecutionResult, ToolExposure, ToolRegistration,
};
use pi_extensions::{Call, Failed, Load, Peer, Pending, ToolInfo};
use serde_json::{Map, Value, json};

use crate::config::CONFIG_DIR_NAME;
use crate::paths::join;
use crate::print::Loadout;
use crate::settings::SettingsManager;

/// What an extension sees of the models: the session's own, the catalog `find` searches, and the
/// providers whose credentials are configured.
pub struct ExtensionModels {
    pub providers: Providers,
    pub current: Model,
    pub list: Vec<Model>,
    pub configured: Vec<String>,
}

impl ExtensionModels {
    /// One call the child made while a handler waited on it, answered here.
    fn call(&self, name: &str, args: &Value) -> std::result::Result<Value, String> {
        match name {
            "complete" => self.complete(args),
            other => Err(format!(
                "pi-rs does not implement the extension call {other}"
            )),
        }
    }

    /// `modelRegistry.complete`: one request with the session's auth, answered whole.
    fn complete(&self, args: &Value) -> std::result::Result<Value, String> {
        let field = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let (provider, id) = (field("provider"), field("modelId"));
        let model = self
            .providers
            .get_model(provider, id)
            .ok_or_else(|| format!("no model {provider}/{id} is configured"))?;
        let context: Context =
            serde_json::from_value(args.get("context").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("the request context does not load: {e}"))?;
        let mut messages = Vec::new();
        if let Some(prompt) = context.system_prompt {
            messages.push(Message::System(SystemMessage {
                role: SystemRole,
                content: SystemContent::Text(prompt),
                sections: None,
                tools_added: None,
                tools_removed: None,
                timestamp: now_ms(),
            }));
        }
        messages.extend(context.messages);
        let mut options = SimpleStreamOptions {
            cache_retention: Some(CacheRetention::None),
            ..SimpleStreamOptions::default()
        };
        if let Some(tokens) = args.pointer("/options/maxTokens").and_then(Value::as_u64) {
            options.max_tokens = Some(tokens);
        }
        if let Some(level) = args
            .pointer("/options/reasoningEffort")
            .and_then(Value::as_str)
        {
            options.reasoning = crate::run_config::parse_level(level)
                .and_then(pi_durable::harness::generation::reasoning);
        }
        let message = self
            .providers
            .complete_simple(&model, TranscriptContext { messages }, options)
            .map_err(|error| error.0)?;
        serde_json::to_value(message).map_err(|e| e.to_string())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

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
            lock(&host.0.commands).push(command.name.clone());
        }
    }
    host.0.sink.tools(load.tools.clone());
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

#[derive(Clone)]
pub struct Live(Arc<Inner>);

struct Inner {
    host: pi_extensions::Host,
    sink: Arc<Sink>,
    state: Mutex<pi_extensions::Ctx>,
    updates: Mutex<Map<String, Value>>,
    dead: AtomicBool,
    settled: AtomicBool,
    started: AtomicBool,
    commands: Mutex<Vec<String>>,
}

#[derive(Default)]
struct Sink {
    calls: Mutex<Vec<Call>>,
    models: Mutex<Option<Arc<ExtensionModels>>>,
    tools: Mutex<Vec<ToolInfo>>,
    registries: Mutex<Vec<(Weak<Registry>, Arc<Loadout>)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Peer for Sink {
    fn ask(&self, call: &str, args: &Value) -> std::result::Result<Value, String> {
        let models = lock(&self.models).clone();
        match models {
            Some(models) => models.call(call, args),
            None => Err(format!("pi-rs has no model to answer {call} with yet")),
        }
    }

    fn call(&self, call: Call) {
        lock(&self.calls).push(call);
    }

    fn failed(&self, failed: Failed) {
        self.call(Call::Notify {
            text: format!("Extension error ({}): {}", failed.path, failed.error),
            level: "error".to_owned(),
        });
    }

    fn tools(&self, tools: Vec<ToolInfo>) {
        *lock(&self.tools) = tools;
        let registries: Vec<(Arc<Registry>, Arc<Loadout>)> = {
            let mut registries = lock(&self.registries);
            registries.retain(|(registry, _)| registry.strong_count() > 0);
            registries
                .iter()
                .filter_map(|(registry, loadout)| Some((registry.upgrade()?, Arc::clone(loadout))))
                .collect()
        };
        for (registry, loadout) in registries {
            if let Err(error) = self.install(&registry, &loadout) {
                self.call(Call::Notify {
                    text: format!("Warning: the extensions' tools were not installed: {error}"),
                    level: "warning".to_owned(),
                });
            }
        }
    }
}

impl Sink {
    fn install(&self, registry: &Registry, loadout: &Loadout) -> std::result::Result<(), Error> {
        let tools: Vec<ToolRegistration> = lock(&self.tools).iter().map(registration).collect();
        registry.install(loadout.with(&tools))
    }
}

fn registration(info: &ToolInfo) -> ToolRegistration {
    let name = info.name.clone();
    let exposure = info
        .exposure
        .as_deref()
        .and_then(|exposure| serde_json::from_value::<ToolExposure>(json!(exposure)).ok())
        .unwrap_or_default();
    let activated = matches!(exposure, ToolExposure::Direct | ToolExposure::ModelOnly)
        && info.default_active != Some(false);
    let execute: ToolExecute = Arc::new(move |args, api, cancel| match live() {
        Some(live) => live.run_tool(&name, args, api, cancel),
        None => Err(Error::Invalid("no extension host is running".to_owned())),
    });
    ToolRegistration {
        tool: AgentTool {
            name: info.name.clone(),
            description: info.description.clone(),
            parameters: info.parameters.clone(),
            constrained_sampling: None,
        },
        replay: None,
        execution_mode: info
            .execution_mode
            .as_deref()
            .and_then(|mode| serde_json::from_value::<ToolExecutionMode>(json!(mode)).ok()),
        output_limits: Some(OutputLimitsSpec {
            max_bytes: Some(usize::MAX),
            max_lines: Some(usize::MAX),
            retain: None,
        }),
        prepare_arguments: None,
        exposure,
        hidden: !activated,
        execute,
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Outcome {
    #[serde(default)]
    content: Vec<UserBlock>,
    #[serde(default)]
    details: Option<Value>,
    #[serde(default)]
    is_error: Option<bool>,
    #[serde(default)]
    terminate: Option<bool>,
}

fn failed_result(message: String) -> ToolExecutionResult {
    ToolExecutionResult {
        content: Some(vec![UserBlock::Text(TextContent {
            kind: TextType,
            text: message,
            text_signature: None,
        })]),
        is_error: Some(true),
        details: Some(json!({})),
        ..ToolExecutionResult::default()
    }
}

fn text_blocks(blocks: Option<&Value>) -> String {
    blocks
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// The child this process loads its extensions in, once main has found them.
static LIVE: OnceLock<Live> = OnceLock::new();

pub fn live() -> Option<&'static Live> {
    LIVE.get()
}

impl Live {
    fn start(dir: &Path, cwd: &str, mode: &str, has_ui: bool) -> std::result::Result<Self, String> {
        let sink = Arc::new(Sink::default());
        let host = pi_extensions::Host::start(dir, cwd, Arc::clone(&sink) as Arc<dyn Peer>)?;
        Ok(Self(Arc::new(Inner {
            host,
            sink,
            state: Mutex::new(pi_extensions::Ctx {
                mode: mode.to_owned(),
                has_ui,
                cwd: cwd.to_owned(),
                streaming: false,
                pending: 0,
                updates: Map::new(),
            }),
            updates: Mutex::new(Map::new()),
            dead: AtomicBool::new(false),
            settled: AtomicBool::new(false),
            started: AtomicBool::new(false),
            commands: Mutex::new(Vec::new()),
        })))
    }

    /// What the next handler sees of the run: the session's cwd, whether a turn is going, and how
    /// many messages wait to be sent.
    pub fn set_state(&self, cwd: &str, streaming: bool, pending: u64) {
        let mut state = lock(&self.0.state);
        state.cwd = cwd.to_owned();
        state.streaming = streaming;
        state.pending = pending;
    }

    pub fn set_session(
        &self,
        harness: &Harness,
        root: &Conversation,
        path: Option<&str>,
        cwd: &str,
        cancel: &CancelToken,
    ) {
        let session_id = crate::sessions::export::session_id(path);
        let Ok(mut entries) = crate::sessions::export::lines(root, cancel, &session_id, cwd) else {
            return;
        };
        let header = entries.remove(0);
        self.update(
            "session",
            json!({
                "header": header,
                "entries": entries,
                "file": path,
                "dir": path.map(crate::paths::dirname),
                "name": crate::session_info::name(harness, cancel),
            }),
        );
    }

    /// Hands the extensions the models `ctx.model`, `ctx.modelRegistry.find` and `complete` read.
    pub fn set_models(&self, models: ExtensionModels) {
        self.update(
            "models",
            json!({
                "current": models.current,
                "list": models.list,
                "configured": models.configured,
            }),
        );
        *lock(&self.0.sink.models) = Some(Arc::new(models));
    }

    pub fn set_tools(&self, builtin: &[ToolRegistration], mcp: &[ToolRegistration]) {
        let info =
            |name: &str, description: &str, parameters: &Value, exposure: Value, source: &str| {
                json!({
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                    "exposure": exposure,
                    "sourceInfo": {
                        "path": format!("builtin:{source}"),
                        "source": "builtin",
                        "scope": "temporary",
                        "origin": "top-level",
                    },
                })
            };
        let registered = builtin
            .iter()
            .map(|r| (r, r.tool.name.as_str()))
            .chain(mcp.iter().map(|r| (r, "mcp")));
        let mut active = Vec::new();
        let mut all = Vec::new();
        for (registration, source) in registered {
            let tool = &registration.tool;
            if registration.exposure.declarable() {
                active.push(tool.name.clone());
            }
            let exposure = serde_json::to_value(registration.exposure).unwrap_or(Value::Null);
            all.push(info(
                &tool.name,
                &tool.description,
                &tool.parameters,
                exposure,
                source,
            ));
        }
        let known = |name: &str| builtin.iter().any(|r| r.tool.name == name);
        for spec in crate::tools::ALL_TOOL_NAMES
            .iter()
            .filter(|name| !known(name))
            .filter_map(|name| crate::tools::spec(name))
        {
            all.push(info(
                spec.name,
                &spec.description,
                &spec.parameters,
                json!("direct"),
                spec.name,
            ));
        }
        if !known(crate::codemode::TOOL_NAME) {
            let tool = crate::codemode::registration(crate::codemode::description()).tool;
            all.push(info(
                &tool.name,
                &tool.description,
                &tool.parameters,
                json!("direct"),
                &tool.name,
            ));
        }
        self.update("tools", json!({ "active": active, "all": all }));
    }

    pub fn attach(
        &self,
        registry: &Arc<Registry>,
        loadout: Loadout,
    ) -> std::result::Result<(), Error> {
        let loadout = Arc::new(loadout);
        self.0.sink.install(registry, &loadout)?;
        lock(&self.0.sink.registries).push((Arc::downgrade(registry), loadout));
        Ok(())
    }

    fn update(&self, key: &str, value: Value) {
        lock(&self.0.updates).insert(key.to_owned(), value);
    }

    /// Loads what the settings and the flags named, in the child that goes on to serve events.
    fn load(
        &self,
        agent_dir: &str,
        dirs: &[String],
        paths: &[String],
    ) -> std::result::Result<Load, String> {
        let ctx = self.ctx();
        let cwd = ctx.cwd.clone();
        self.0.host.load(&cwd, agent_dir, dirs, paths, &ctx)
    }

    fn ctx(&self) -> pi_extensions::Ctx {
        let mut ctx = lock(&self.0.state).clone();
        ctx.updates = std::mem::take(&mut *lock(&self.0.updates));
        ctx
    }

    fn begin(
        &self,
        start: impl FnOnce(
            &pi_extensions::Host,
            &pi_extensions::Ctx,
        ) -> std::result::Result<Pending, String>,
    ) -> std::result::Result<Pending, String> {
        let mut updates = lock(&self.0.updates);
        let mut ctx = lock(&self.0.state).clone();
        ctx.updates = std::mem::take(&mut *updates);
        let pending = start(&self.0.host, &ctx);
        drop(updates);
        pending.inspect_err(|_| self.died())
    }

    fn died(&self) {
        if let Some(error) = self.0.host.failure()
            && !self.0.dead.swap(true, Ordering::SeqCst)
        {
            self.0.sink.call(Call::Notify {
                text: format!("Warning: {error}"),
                level: "warning".to_owned(),
            });
        }
    }

    fn finish(
        &self,
        pending: std::result::Result<Pending, String>,
    ) -> std::result::Result<Value, String> {
        pending
            .and_then(|pending| pending.wait(None, &mut |_| {}))
            .inspect_err(|_| self.died())
    }

    fn run_tool(
        &self,
        name: &str,
        args: Value,
        api: &dyn ToolExecutionApi,
        cancel: &CancelToken,
    ) -> std::result::Result<ToolExecutionResult, Error> {
        if cancel.is_cancelled() {
            return Err(Error::Aborted);
        }
        let call_id = api.call_id().to_owned();
        let pending = self
            .begin(|host, ctx| host.tool(name, &call_id, args, ctx))
            .map_err(Error::Invalid)?;
        let mut shown = String::new();
        let mut details: Option<Value> = None;
        let answer = pending.wait(Some(cancel), &mut |partial| {
            let text = text_blocks(partial.get("content"));
            if let Some(added) = text.strip_prefix(shown.as_str())
                && !added.is_empty()
            {
                let _ = api.output(OutputChunk::Text(added), None);
                shown = text;
            }
            if let Some(next) = partial.get("details").filter(|next| !next.is_null())
                && details.as_ref() != Some(next)
            {
                details = Some(next.clone());
                let _ = api.details(next.clone(), cancel);
            }
        });
        if cancel.is_cancelled() {
            return Err(Error::Aborted);
        }
        let value = match answer {
            Ok(value) => value,
            Err(error) => {
                self.died();
                return Ok(failed_result(error));
            }
        };
        let outcome: Outcome = match serde_json::from_value(value) {
            Ok(outcome) => outcome,
            Err(e) => return Ok(failed_result(format!("Tool {name} returned {e}"))),
        };
        Ok(ToolExecutionResult {
            content: Some(outcome.content),
            is_error: outcome.is_error,
            details: Some(outcome.details.unwrap_or(Value::Null)),
            control: outcome
                .terminate
                .filter(|terminate| *terminate)
                .map(|_| ToolControl {
                    terminate: true,
                    ..ToolControl::default()
                }),
            ..ToolExecutionResult::default()
        })
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
        let _ = self.finish(self.begin(|host, ctx| host.command(name, args, ctx)));
    }

    /// Runs the line as an extension command when it names one, as upstream's
    /// `_tryExecuteExtensionCommand` does: the prompt is then not sent to the model.
    pub fn run_command(&self, typed: &str) -> bool {
        let Some(typed) = typed.strip_prefix('/') else {
            return false;
        };
        let (name, args) = typed.split_once(' ').unwrap_or((typed, ""));
        let known = lock(&self.0.commands).iter().any(|command| command == name);
        if !known {
            return false;
        }
        self.command(name, args);
        true
    }

    /// `agent_settled`, once a run has ended and nothing is left to send.
    pub fn settle(&self, aborted: bool) {
        let pending = lock(&self.0.state).pending;
        if pending == 0 && self.0.settled.swap(false, Ordering::SeqCst) {
            self.event("agent_settled", json!({ "aborted": aborted }));
        }
    }

    /// The calls the extensions made, in the order they made them.
    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut *lock(&self.0.sink.calls))
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
        self.finish(self.begin(|host, ctx| host.event(name, payload, ctx)))
    }
}
