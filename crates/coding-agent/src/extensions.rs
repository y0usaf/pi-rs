//! Upstream `core/extensions` and the extension half of `core/resource-loader.ts`: which paths
//! the `packages` setting, the `extensions` setting, `--extension` and the two standard
//! directories name, and the child that loads them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use pi_durable::ai::event_stream::AssistantMessageEventStream;
use pi_durable::ai::models::Models as ModelsTrait;
use pi_durable::ai::registry::{Api, Origin, Providers};
use pi_durable::ai::stream::{fail, new_output};
use pi_durable::ai::types::{
    AssistantMessage, AssistantMessageEvent, CachePoint, CacheRetention, Context, Message, Model,
    SimpleStreamOptions, SystemContent, SystemMessage, SystemRole, TextContent, TextType,
    Tool as AgentTool, TranscriptContext, UserBlock, UserContent,
};
use pi_durable::ai::utils::transcript::{get_current_system_message, normalize_context};
use pi_durable::cancel::CancelToken;
use pi_durable::errors::Error;
use pi_durable::harness::events::AgentEvent;
use pi_durable::harness::generation::{GENERATION_HOOK, GenerationHooks};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::output::OutputChunk;
use pi_durable::harness::registry::Registry;
use pi_durable::harness::tool::ToolRound;
use pi_durable::harness::types::{
    HookApi, HookRegistration, OutputLimitsSpec, ToolControl, ToolExecute, ToolExecutionApi,
    ToolExecutionMode, ToolExecutionResult, ToolExposure, ToolRegistration,
};
use pi_durable::ids::EntryId;
use pi_durable::types::EntryRecord;
use pi_extensions::{Call, Failed, Load, Peer, Pending, ProviderInfo, ToolInfo};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{Map, Value, json};

mod runtime;

pub use runtime::request_hooks;

use crate::config::CONFIG_DIR_NAME;
use crate::loadout::{Entry, Loadout, PromptView, Source, ToolsView};
use crate::models::ModelRegistry;
use crate::paths::join;
use crate::settings::SettingsManager;
use crate::system_prompt::PromptInput;

mod calls;

/// What an extension sees of the models: the session's own, the catalog `find` searches, and the
/// providers whose credentials are configured.
struct ExtensionModels {
    providers: Providers,
    registry: Arc<ModelRegistry>,
    current: Model,
    list: Vec<Model>,
    configured: Vec<String>,
}

impl ExtensionModels {
    fn of(registry: &Arc<ModelRegistry>, model: &Model) -> Self {
        let mut configured: Vec<String> = registry
            .available()
            .into_iter()
            .map(|model| model.provider)
            .collect();
        configured.extend(
            registry
                .stored_credentials()
                .into_iter()
                .map(|(provider, _)| provider)
                .filter(|provider| registry.has_configured_auth(provider)),
        );
        configured.sort();
        configured.dedup();
        ExtensionModels {
            providers: crate::print::providers(Arc::clone(registry), model),
            registry: Arc::clone(registry),
            current: model.clone(),
            list: registry.models().to_vec(),
            configured,
        }
    }

    fn refresh(&self, args: &Value, cancel: &CancelToken) -> (Value, ExtensionModels) {
        let allow_network = args
            .get("allowNetwork")
            .and_then(Value::as_bool)
            .unwrap_or(self.registry.network);
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        let only: Option<Vec<String>> = take(args, "providers");
        let refreshed = self
            .registry
            .refresh(allow_network, force, only.as_deref(), cancel);
        let next = ExtensionModels::of(&self.registry, &self.current);
        let answer = json!({
            "aborted": refreshed.aborted,
            "errors": refreshed.errors,
            "list": next.list,
            "configured": next.configured,
        });
        (answer, next)
    }

    /// One call the child made while a handler waited on it, answered here.
    fn call(
        &self,
        name: &str,
        args: &Value,
        update: &mut dyn FnMut(Value),
        cancel: &CancelToken,
    ) -> std::result::Result<Value, String> {
        match name {
            "complete" => self.complete(args),
            "stream" => self.stream(args, update, cancel),
            "providerAuth" => self.provider_auth(args),
            other => Err(format!(
                "pi-rs does not implement the extension call {other}"
            )),
        }
    }

    /// A built-in provider's `streamSimple`: pi-rs's client for the model's API, each event handed
    /// back as it comes. A context with upstream's `systemPrompt` and `tools` is normalized first.
    fn stream(
        &self,
        args: &Value,
        update: &mut dyn FnMut(Value),
        cancel: &CancelToken,
    ) -> std::result::Result<Value, String> {
        let model: Model = serde_json::from_value(args.get("model").cloned().unwrap_or_default())
            .map_err(|e| format!("the request model does not load: {e}"))?;
        let context: Context =
            serde_json::from_value(args.get("context").cloned().unwrap_or_default())
                .map_err(|e| format!("the request context does not load: {e}"))?;
        let options = options_of(
            args.get("options").unwrap_or(&Value::Null),
            Some(cancel.clone()),
        );
        let stream = self
            .providers
            .stream_builtin(&model, normalize_context(&context), options);
        while let Some(event) = stream.next_event() {
            update(serde_json::to_value(&event).map_err(|e| e.to_string())?);
        }
        Ok(Value::Null)
    }

    /// `modelRegistry.getProviderAuth`: the credentials a request to the provider gets, nothing
    /// when it has none.
    fn provider_auth(&self, args: &Value) -> std::result::Result<Value, String> {
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !self.registry.has_configured_auth(provider) {
            return Ok(Value::Null);
        }
        let auth = self
            .registry
            .provider_auth(provider, crate::auth::OAUTH_MINIMUM_VALIDITY_MS)?;
        let mut resolved = Map::new();
        if let Some(key) = auth.api_key {
            resolved.insert("apiKey".to_owned(), json!(key));
        }
        if !auth.headers.is_empty() {
            resolved.insert("headers".to_owned(), json!(auth.headers));
        }
        if let Some(base_url) = auth.base_url {
            resolved.insert("baseUrl".to_owned(), json!(base_url));
        }
        let mut answer = json!({ "auth": resolved });
        if !auth.env.is_empty() {
            answer["env"] = json!(auth.env);
        }
        Ok(answer)
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

fn take<T: DeserializeOwned>(value: &Value, key: &str) -> Option<T> {
    value
        .get(key)
        .filter(|field| !field.is_null())
        .and_then(|field| serde_json::from_value(field.clone()).ok())
}

/// A request's options as upstream's `SimpleStreamOptions` names them.
fn options_json(options: &SimpleStreamOptions) -> Value {
    let mut out = Map::new();
    let fields = [
        ("apiKey", json!(options.api_key)),
        ("baseUrl", json!(options.base_url)),
        ("sessionId", json!(options.session_id)),
        ("reasoning", json!(options.reasoning)),
        ("cacheRetention", json!(options.cache_retention)),
        ("deferred", json!(options.deferred.then_some(true))),
        ("maxTokens", json!(options.max_tokens)),
        ("transport", json!(options.transport)),
        ("timeoutMs", json!(options.timeout_ms)),
        ("maxRetries", json!(options.max_retries)),
        ("maxRetryDelayMs", json!(options.max_retry_delay_ms)),
        ("headers", json!(options.headers)),
        ("metadata", json!(options.metadata)),
        ("temperature", json!(options.temperature)),
        ("toolChoice", json!(options.tool_choice)),
    ];
    for (key, value) in fields {
        if !value.is_null() {
            out.insert(key.to_owned(), value);
        }
    }
    if !options.cache_points.is_empty() {
        let points = options
            .cache_points
            .iter()
            .map(|point| json!({ "message": point.message, "block": point.block }))
            .collect();
        out.insert("cachePoints".to_owned(), Value::Array(points));
    }
    Value::Object(out)
}

fn options_of(value: &Value, signal: Option<CancelToken>) -> SimpleStreamOptions {
    let cache_points = value
        .get("cachePoints")
        .and_then(Value::as_array)
        .map(|points| {
            points
                .iter()
                .filter_map(|point| {
                    Some(CachePoint {
                        message: usize::try_from(point.get("message")?.as_u64()?).ok()?,
                        block: point
                            .get("block")
                            .and_then(Value::as_u64)
                            .and_then(|block| usize::try_from(block).ok()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    SimpleStreamOptions {
        signal,
        api_key: take(value, "apiKey"),
        base_url: take(value, "baseUrl"),
        session_id: take(value, "sessionId"),
        reasoning: take(value, "reasoning"),
        cache_retention: take(value, "cacheRetention"),
        deferred: value
            .get("deferred")
            .is_some_and(|deferred| !deferred.is_null() && deferred != &Value::Bool(false)),
        max_tokens: take(value, "maxTokens"),
        transport: take(value, "transport"),
        timeout_ms: take(value, "timeoutMs"),
        max_retries: take(value, "maxRetries"),
        max_retry_delay_ms: take(value, "maxRetryDelayMs"),
        headers: take(value, "headers"),
        metadata: take(value, "metadata"),
        cache_points,
        temperature: take(value, "temperature"),
        tool_choice: take(value, "toolChoice"),
        hooks: Default::default(),
    }
}

struct Streamed {
    model: Value,
    context: Value,
    options: Value,
    hooks: bool,
}

/// A provider an extension registered with its own `streamSimple`: each request goes to the child.
struct Routed {
    provider: String,
}

impl Api for Routed {
    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        let out = AssistantMessageEventStream::new();
        let Some(live) = live() else {
            fail(
                &out,
                new_output(model),
                None,
                "no extension host is running".to_owned(),
            );
            return out;
        };
        let signal = options.signal.clone();
        if let Some(signal) = &signal {
            out.bind_cancel_with(signal, new_output(model));
        }
        let request = Streamed {
            model: serde_json::to_value(model).unwrap_or_default(),
            context: json!({ "messages": context.messages }),
            options: options_json(&options),
            hooks: options.hooks.any(),
        };
        let (live, provider, model, sink) = (
            live.clone(),
            self.provider.clone(),
            model.clone(),
            out.clone(),
        );
        std::thread::spawn(move || live.stream(&provider, &model, request, signal.as_ref(), &sink));
        out
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
    pub errors: Vec<String>,
    pub loaded: Vec<LoadedExtension>,
}

#[derive(Clone)]
pub struct Resources {
    pub cwd: String,
    pub agent_dir: String,
    pub project_trusted: bool,
    pub no_skills: bool,
    pub skill_paths: Vec<String>,
}

fn unknown_options(names: &[&str]) -> Option<String> {
    (!names.is_empty()).then(|| {
        format!(
            "Unknown option{}: {}",
            if names.len() == 1 { "" } else { "s" },
            names
                .iter()
                .map(|name| format!("--{name}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub source: String,
    pub scope: String,
    pub base_dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedExtension {
    pub path: String,
    pub source_info: Option<SourceInfo>,
}

fn within(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

fn source_info(resolved: &str, cli: &[String], cwd: &str, dirs: &[String]) -> Option<SourceInfo> {
    if cli.iter().any(|path| within(resolved, &resolve(path, cwd))) {
        return Some(SourceInfo {
            source: "cli".to_owned(),
            scope: "temporary".to_owned(),
            base_dir: None,
        });
    }
    dirs.iter()
        .zip(["project", "user"])
        .find(|(dir, _)| within(resolved, dir))
        .map(|(dir, scope)| SourceInfo {
            source: "local".to_owned(),
            scope: scope.to_owned(),
            base_dir: Some(dir.clone()),
        })
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
pub fn mode_name(mode: Option<crate::args::Mode>, tui: bool) -> &'static str {
    match mode {
        Some(crate::args::Mode::Rpc) => "rpc",
        Some(crate::args::Mode::Json) => "json",
        Some(crate::args::Mode::Text) => "print",
        None if tui => "tui",
        None => "print",
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
#[allow(clippy::too_many_arguments)]
pub fn load(
    cwd: &str,
    agent_dir: &str,
    args: &crate::args::Args,
    settings: &SettingsManager,
    registry: &Arc<ModelRegistry>,
    mode: &str,
    has_ui: bool,
) -> Extensions {
    let cli = args.extensions.as_deref().unwrap_or(&[]);
    let no_extensions = args.no_extensions;
    let flags = &args.unknown_flags;
    let mut extensions = Extensions::default();
    let names: Vec<&str> = flags.iter().map(|(name, _)| name.as_str()).collect();
    let unknown = unknown_options(&names);
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
    extensions.errors.extend(unknown.clone());
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
    host.update("providers", builtin_providers());
    host.update("models", json!({ "list": registry.models().as_ref() }));
    let _ = host.0.sink.registry.set(Arc::clone(registry));
    let sources = pi_extensions::Sources {
        dirs: &dirs,
        paths: &paths,
        cli,
        flags,
    };
    let load = match host.load(agent_dir, &sources) {
        Ok(load) => load,
        Err(error) => {
            extensions.notices.push(format!("Warning: {error}"));
            return extensions;
        }
    };
    extensions.errors = load.flag_errors.clone();
    extensions.loaded = load
        .extensions
        .iter()
        .map(|extension| LoadedExtension {
            path: extension.path.clone(),
            source_info: source_info(&extension.resolved_path, cli, cwd, &dirs),
        })
        .collect();
    for extension in &load.extensions {
        for command in &extension.commands {
            lock(&host.0.commands).push(command.name.clone());
        }
        lock(&host.0.handled).extend(extension.events.iter().cloned());
    }
    host.0.sink.tools(load.tools.clone());
    host.0.sink.providers(load.providers.clone());
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

/// `builtinProviders()`: each provider of the built-in catalog, with the login it offers.
fn builtin_providers() -> Value {
    let mut providers: Vec<(String, String)> = Vec::new();
    for model in pi_durable::ai::catalog::all() {
        if !providers.iter().any(|(id, _)| *id == model.provider) {
            providers.push((model.provider, model.base_url));
        }
    }
    let logins = crate::login::providers_for(Some(crate::login::AuthType::Oauth));
    providers
        .into_iter()
        .map(|(id, base_url)| {
            let oauth = logins
                .iter()
                .find(|login| login.id == id)
                .map(|login| json!({ "id": id, "name": login.name }));
            json!({
                "id": id,
                "name": crate::login::display_name(&id),
                "baseUrl": base_url,
                "oauth": oauth,
            })
        })
        .collect()
}

#[derive(Clone)]
pub struct Live(Arc<Inner>);

struct Inner {
    host: pi_extensions::Host,
    sink: Arc<Sink>,
    state: Mutex<pi_extensions::Ctx>,
    updates: Mutex<Map<String, Value>>,
    shown: Mutex<Map<String, Value>>,
    dead: AtomicBool,
    settled: AtomicBool,
    started: AtomicBool,
    commands: Mutex<Vec<String>>,
    handled: Mutex<HashSet<String>>,
    resources: Mutex<Option<Resources>>,
    run: Mutex<RunState>,
    session: Mutex<Option<SessionSource>>,
    announced: Mutex<Option<EntryId>>,
    thinking: Mutex<Option<String>>,
    round: Mutex<calls::Round>,
    ahead: Mutex<Option<calls::Ahead>>,
}

#[derive(Default)]
struct RunState {
    started: bool,
    turn: Option<usize>,
    next: usize,
    forced: Option<String>,
    messages: Vec<Value>,
}

#[derive(Clone)]
struct SessionSource {
    root: Conversation,
    path: Option<String>,
    cwd: String,
    name: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    messages: Vec<IgnoredAny>,
    system_prompt_options: RunPrompt,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunPrompt {
    #[serde(flatten)]
    input: PromptInput,
    #[serde(default)]
    force_system_prompt: Option<String>,
}

struct Prepared {
    options: PromptInput,
    forced: Option<String>,
    selected_before: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Discovered {
    #[serde(default)]
    skill_paths: Vec<DiscoveredPath>,
}

#[derive(serde::Deserialize)]
struct DiscoveredPath {
    path: String,
}

#[derive(serde::Deserialize)]
struct Transformed {
    messages: Vec<Message>,
}

#[derive(serde::Deserialize)]
struct Boundary {
    entries: Vec<IgnoredAny>,
    #[serde(rename = "continue")]
    proceed: bool,
}

#[derive(Default)]
struct Sink {
    calls: Mutex<Vec<Call>>,
    models: Mutex<Option<Arc<ExtensionModels>>>,
    tools: Mutex<Vec<ToolInfo>>,
    providers: Mutex<Vec<ProviderInfo>>,
    loadouts: Mutex<Vec<(Weak<Registry>, Arc<Loadout>)>>,
    registry: OnceLock<Arc<ModelRegistry>>,
    failed: Mutex<Option<AssistantMessage>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Peer for Sink {
    fn ask(
        &self,
        call: &str,
        args: &Value,
        update: &mut dyn FnMut(Value),
        cancel: &CancelToken,
    ) -> std::result::Result<Value, String> {
        if call == "compact" {
            return live()
                .ok_or("no extension host is running")?
                .compact(args, cancel);
        }
        if call == "activeTools" {
            return Ok(Value::Object(self.activate(args)));
        }
        let models = lock(&self.models).clone();
        match models {
            Some(models) if call == "refreshModels" => {
                let (answer, next) = models.refresh(args, cancel);
                *lock(&self.models) = Some(Arc::new(next));
                Ok(answer)
            }
            Some(models) => models.call(call, args, update, cancel),
            None => Err(format!("pi-rs has no model to answer {call} with yet")),
        }
    }

    fn providers(&self, providers: Vec<ProviderInfo>) {
        *lock(&self.providers) = providers;
    }

    fn virtual_models(&self, models: Vec<Value>) {
        let Some(registry) = self.registry.get() else {
            return;
        };
        let mut parsed = Vec::new();
        for model in models {
            match serde_json::from_value::<Model>(model) {
                Ok(model) => parsed.push(model),
                Err(e) => self.call(Call::Notify {
                    text: format!("Warning: a virtual model pi-rs cannot read: {e}"),
                    level: "warning".to_owned(),
                }),
            }
        }
        registry.set_virtual(parsed);
        let handed = lock(&self.models).clone();
        if let (Some(live), Some(handed)) = (live(), handed) {
            live.hand_models(&handed.registry, &handed.current);
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
        let entries = self.entries();
        if let Err(error) = self.each(|loadout| loadout.set_extension_tools(entries.clone())) {
            self.warn(format!("the extensions' tools were not installed: {error}"));
        }
    }
}

impl Sink {
    fn activate(&self, args: &Value) -> Map<String, Value> {
        let names = args.get("names").cloned().unwrap_or(Value::Null);
        match serde_json::from_value::<Vec<String>>(names) {
            Ok(names) => {
                if let Err(error) = self.each(|loadout| loadout.set_active(names.clone())) {
                    self.warn(format!("the active tools were not installed: {error}"));
                }
            }
            Err(e) => self.failed(Failed {
                path: "setActiveTools".to_owned(),
                event: String::new(),
                error: format!("the extension host sent tool names pi-rs cannot read: {e}"),
            }),
        }
        live().map(Live::take_shown).unwrap_or_default()
    }

    fn warn(&self, text: String) {
        self.call(Call::Notify {
            text: format!("Warning: {text}"),
            level: "warning".to_owned(),
        });
    }

    fn loadouts(&self) -> Vec<Arc<Loadout>> {
        let mut loadouts = lock(&self.loadouts);
        loadouts.retain(|(registry, _)| registry.strong_count() > 0);
        loadouts
            .iter()
            .map(|(_, loadout)| Arc::clone(loadout))
            .collect()
    }

    fn prompt(&self) -> Option<PromptInput> {
        self.loadouts()
            .iter()
            .rev()
            .find_map(|loadout| loadout.prompt())
    }

    fn each(
        &self,
        change: impl Fn(&Loadout) -> std::result::Result<(), Error>,
    ) -> std::result::Result<(), Error> {
        let mut outcome = Ok(());
        for loadout in self.loadouts() {
            if let Err(error) = change(&loadout) {
                outcome = Err(error);
            }
        }
        outcome
    }

    fn entries(&self) -> Vec<Entry> {
        lock(&self.tools).iter().map(entry).collect()
    }
}

fn entry(info: &ToolInfo) -> Entry {
    let mut entry = Entry::new(
        registration(info),
        Source::Extension {
            path: info.path.clone(),
        },
    )
    .with_prompt(info.prompt_snippet.as_deref(), &info.prompt_guidelines);
    entry.default_active = info.default_active != Some(false);
    entry
}

fn registration(info: &ToolInfo) -> ToolRegistration {
    let name = info.name.clone();
    let exposure = info
        .exposure
        .as_deref()
        .and_then(|exposure| serde_json::from_value::<ToolExposure>(json!(exposure)).ok())
        .unwrap_or_default();
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
        hidden: false,
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
            shown: Mutex::new(Map::new()),
            dead: AtomicBool::new(false),
            settled: AtomicBool::new(false),
            started: AtomicBool::new(false),
            commands: Mutex::new(Vec::new()),
            handled: Mutex::new(HashSet::new()),
            resources: Mutex::new(None),
            run: Mutex::new(RunState::default()),
            session: Mutex::new(None),
            announced: Mutex::new(None),
            thinking: Mutex::new(None),
            round: Mutex::new(calls::Round::default()),
            ahead: Mutex::new(None),
        })))
    }

    pub fn set_resources(&self, resources: Resources) {
        *lock(&self.0.resources) = Some(resources);
    }

    pub fn extension_entries(&self) -> Vec<Entry> {
        self.0.sink.entries()
    }

    pub fn show_tools(&self, view: &ToolsView<'_>) {
        if let Ok(value) = serde_json::to_value(view) {
            lock(&self.0.shown).insert("tools".to_owned(), value);
        }
    }

    pub fn show_prompt(&self, view: &PromptView) {
        if let Ok(value) = serde_json::to_value(view) {
            lock(&self.0.shown).insert("prompt".to_owned(), value);
        }
    }

    fn take_shown(&self) -> Map<String, Value> {
        std::mem::take(&mut *lock(&self.0.shown))
    }

    fn pending_updates(&self, updates: &mut Map<String, Value>) -> Map<String, Value> {
        let mut pending = std::mem::take(updates);
        pending.extend(self.take_shown());
        pending
    }

    fn discover(&self, reason: &str) {
        if !self.handles("resources_discover") {
            return;
        }
        let cwd = lock(&self.0.state).cwd.clone();
        let reason = if reason == "reload" {
            "reload"
        } else {
            "startup"
        };
        let Ok(answer) = self.ask(
            "resources_discover",
            json!({ "cwd": cwd, "reason": reason }),
        ) else {
            return;
        };
        let discovered: Discovered = match serde_json::from_value(answer) {
            Ok(discovered) => discovered,
            Err(e) => {
                self.0.sink.warn(format!(
                    "the extension host answered resources_discover with {e}"
                ));
                return;
            }
        };
        if discovered.skill_paths.is_empty() {
            return;
        }
        let skills = {
            let mut resources = lock(&self.0.resources);
            let Some(resources) = resources.as_mut() else {
                return;
            };
            for found in discovered.skill_paths {
                if !resources.skill_paths.contains(&found.path) {
                    resources.skill_paths.push(found.path);
                }
            }
            crate::skills::load_skills(&crate::skills::SkillsInput {
                cwd: &resources.cwd,
                agent_dir: &resources.agent_dir,
                project_trusted: resources.project_trusted,
                no_skills: resources.no_skills,
                cli_paths: &resources.skill_paths,
            })
            .skills
        };
        if let Err(error) = self
            .0
            .sink
            .each(|loadout| loadout.set_skills(skills.clone()))
        {
            self.0
                .sink
                .warn(format!("the discovered skills were not installed: {error}"));
        }
    }

    fn handles(&self, event: &str) -> bool {
        lock(&self.0.handled).contains(event)
    }

    fn unported(&self, what: &str) {
        self.0.sink.call(Call::Notify {
            text: format!("Warning: {what}, which pi-rs does not implement yet"),
            level: "warning".to_owned(),
        });
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
        let moved = lock(&self.0.session)
            .as_ref()
            .is_none_or(|source| source.root.id() != root.id());
        if moved {
            self.skip_announced(root, cancel);
        }
        *lock(&self.0.session) = Some(SessionSource {
            root: root.clone(),
            path: path.map(str::to_owned),
            cwd: cwd.to_owned(),
            name: crate::session_info::name(harness, cancel),
        });
        self.refresh_session(cancel);
    }

    fn refresh_session(&self, cancel: &CancelToken) {
        if let Some(session) = self.session_update(cancel) {
            self.update("session", session);
        }
    }

    fn session_update(&self, cancel: &CancelToken) -> Option<Value> {
        let source = lock(&self.0.session).clone()?;
        let path = source.path.as_deref();
        let session_id = crate::sessions::export::session_id(path);
        let mut entries =
            crate::sessions::export::lines(&source.root, cancel, &session_id, &source.cwd).ok()?;
        let header = entries.remove(0);
        self.unstored(&mut entries);
        Some(json!({
            "header": header,
            "entries": entries,
            "file": path,
            "dir": path.map(crate::paths::dirname),
            "name": source.name,
        }))
    }

    pub fn before_agent_start(&self, content: &UserContent) -> std::result::Result<(), String> {
        if let Some(root) = self.root() {
            self.skip_announced(&root, &CancelToken::new());
        }
        let Some(base) = self.0.sink.prompt() else {
            return Ok(());
        };
        lock(&self.0.run).forced = None;
        let installed = |e: Error| format!("the run's system prompt was not installed: {e}");
        self.0.sink.each(Loadout::end_run).map_err(installed)?;
        let prepared = if self.handles("before_agent_start") {
            self.run_prompt(base.clone(), content)?
        } else {
            None
        };
        let prompt = prepared.unwrap_or_else(|| Prepared {
            selected_before: base.selected_tools.clone(),
            options: base,
            forced: None,
        });
        lock(&self.0.run).forced = prompt.forced.clone();
        self.0
            .sink
            .each(|loadout| {
                loadout.start_run(
                    prompt.options.clone(),
                    prompt.forced.clone(),
                    &prompt.selected_before,
                )
            })
            .map_err(installed)?;
        Ok(())
    }

    fn run_prompt(
        &self,
        base: PromptInput,
        content: &UserContent,
    ) -> std::result::Result<Option<Prepared>, String> {
        let images: Vec<&UserBlock> = match content {
            UserContent::Blocks(blocks) => blocks
                .iter()
                .filter(|block| matches!(block, UserBlock::Image(_)))
                .collect(),
            UserContent::Text(_) => Vec::new(),
        };
        let mut event = json!({ "prompt": text_of(content).unwrap_or_default() });
        if !images.is_empty() {
            event["images"] = json!(images);
        }
        event["systemPromptOptions"] = serde_json::to_value(&base).map_err(|e| e.to_string())?;
        let Ok(answer) = self.ask("before_agent_start", event) else {
            return Ok(None);
        };
        let started: Started = serde_json::from_value(answer)
            .map_err(|e| format!("the extension host answered before_agent_start with {e}"))?;
        let RunPrompt {
            input: mut options,
            force_system_prompt,
        } = started.system_prompt_options;
        options.check_sections()?;
        if !started.messages.is_empty() {
            self.unported("a before_agent_start handler returned a message");
        }
        options.package_dir = base.package_dir;
        Ok(Some(Prepared {
            options,
            forced: force_system_prompt,
            selected_before: base.selected_tools,
        }))
    }

    fn start_run(&self) {
        let mut run = lock(&self.0.run);
        if run.started {
            return;
        }
        run.started = true;
        *lock(&self.0.sink.failed) = None;
        self.event("agent_start", json!({}));
    }

    fn turn_start(&self) {
        self.start_run();
        let index = {
            let mut run = lock(&self.0.run);
            if run.turn.is_some() {
                return;
            }
            run.turn = Some(run.next);
            run.next
        };
        if index > 0 {
            for loadout in self.0.sink.loadouts() {
                loadout.next_turn();
            }
        }
        self.event(
            "turn_start",
            json!({ "turnIndex": index, "timestamp": now_ms() }),
        );
    }

    fn turn_end(&self) {
        let index = {
            let mut run = lock(&self.0.run);
            let Some(index) = run.turn.take() else {
                return;
            };
            run.next = index + 1;
            index
        };
        if !self.handles("turn_end") {
            return;
        }
        self.refresh_session(&CancelToken::new());
        let Ok(answer) = self.ask("turn_end", json!({ "turnIndex": index })) else {
            return;
        };
        let boundary = match serde_json::from_value::<Option<Boundary>>(answer) {
            Ok(Some(boundary)) => boundary,
            Ok(None) => return,
            Err(e) => {
                self.0.sink.call(Call::Notify {
                    text: format!("Warning: the extension host answered turn_end with {e}"),
                    level: "warning".to_owned(),
                });
                return;
            }
        };
        if !boundary.entries.is_empty() {
            self.unported("a turn_end handler returned session entries");
        }
        if boundary.proceed {
            self.unported("a turn_end handler asked the run to continue");
        }
    }

    pub fn end_run(&self) {
        self.close_round();
        if let Some(root) = self.root() {
            self.announce(&root);
        }
        self.turn_end();
        let messages = std::mem::take(&mut *lock(&self.0.run)).messages;
        self.event("agent_end", json!({ "messages": messages }));
    }

    fn end_prompt_run(&self) {
        if let Err(error) = self.0.sink.each(Loadout::end_run) {
            self.0
                .sink
                .warn(format!("the base system prompt was not restored: {error}"));
        }
    }

    fn forced(&self, messages: &[Message]) -> Option<Vec<Message>> {
        let forced = lock(&self.0.run).forced.clone()?;
        let current = get_current_system_message(messages);
        let head = SystemMessage {
            role: SystemRole,
            content: SystemContent::Text(forced),
            sections: None,
            tools_added: current
                .as_ref()
                .and_then(|current| current.tools_added.clone()),
            tools_removed: None,
            timestamp: current.map_or_else(now_ms, |current| current.timestamp),
        };
        Some(
            std::iter::once(Message::System(head))
                .chain(
                    messages
                        .iter()
                        .filter(|message| !matches!(message, Message::System(_)))
                        .cloned(),
                )
                .collect(),
        )
    }

    /// Hands the extensions the models they see: the session's own, the catalog `find` searches,
    /// and the providers whose credentials are configured, logins without models included.
    pub fn hand_models(&self, registry: &Arc<ModelRegistry>, model: &Model) {
        self.set_models(ExtensionModels::of(registry, model));
    }

    /// The API requests to `model` go through when an extension registered it as a virtual model,
    /// or registered its provider with its own `streamSimple`.
    pub fn provider_api(&self, model: &Model, origin: Origin) -> Option<Arc<dyn Api>> {
        if crate::models::is_virtual(model) {
            return Some(Arc::new(runtime::Virtual { origin }));
        }
        lock(&self.0.sink.providers)
            .iter()
            .any(|info| info.id == model.provider && info.streams)
            .then(|| {
                Arc::new(Routed {
                    provider: model.provider.clone(),
                }) as Arc<dyn Api>
            })
    }

    /// The built-in provider whose login a provider an extension registered runs, as an extra
    /// login for one provider does.
    pub fn oauth_flow(&self, provider: &str) -> Option<String> {
        lock(&self.0.sink.providers)
            .iter()
            .find(|info| info.id == provider)
            .and_then(|info| info.oauth.as_ref())
            .and_then(|oauth| oauth.flow.clone())
    }

    /// One request through a provider's `streamSimple` in the child, its events pushed to `out`.
    fn stream(
        &self,
        provider: &str,
        model: &Model,
        request: Streamed,
        signal: Option<&CancelToken>,
        out: &AssistantMessageEventStream,
    ) {
        let mut unreadable: Option<String> = None;
        let answer = self
            .0
            .host
            .stream(
                provider,
                request.model,
                request.context,
                request.options,
                request.hooks,
            )
            .and_then(|pending| {
                pending.wait(signal, &mut |event| match serde_json::from_value::<
                    AssistantMessageEvent,
                >(event)
                {
                    Ok(event) => {
                        let last = matches!(
                            event,
                            AssistantMessageEvent::Done { .. }
                                | AssistantMessageEvent::Error { .. }
                        );
                        out.push(event);
                        if last {
                            out.end(None);
                        }
                    }
                    Err(e) => {
                        unreadable.get_or_insert_with(|| {
                            format!("the {provider} provider sent an event pi-rs cannot read: {e}")
                        });
                    }
                })
            });
        if out.is_done() {
            return;
        }
        let message = match (answer, unreadable) {
            (Err(error), _) => {
                self.died();
                error
            }
            (Ok(_), Some(unreadable)) => unreadable,
            (Ok(_), None) => format!("the {provider} provider's stream ended without a result"),
        };
        fail(out, new_output(model), signal, message);
    }

    fn set_models(&self, models: ExtensionModels) {
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

    pub fn attach(
        &self,
        registry: &Arc<Registry>,
        loadout: &Arc<Loadout>,
    ) -> std::result::Result<(), Error> {
        lock(&self.0.sink.loadouts).push((Arc::downgrade(registry), Arc::clone(loadout)));
        loadout.attach(registry)
    }

    fn update(&self, key: &str, value: Value) {
        lock(&self.0.updates).insert(key.to_owned(), value);
    }

    /// Loads what the settings and the flags named, in the child that goes on to serve events.
    fn load(
        &self,
        agent_dir: &str,
        sources: &pi_extensions::Sources<'_>,
    ) -> std::result::Result<Load, String> {
        let ctx = self.ctx();
        let cwd = ctx.cwd.clone();
        self.0.host.load(&cwd, agent_dir, sources, &ctx)
    }

    fn ctx(&self) -> pi_extensions::Ctx {
        let mut ctx = lock(&self.0.state).clone();
        ctx.updates = self.pending_updates(&mut lock(&self.0.updates));
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
        ctx.updates = self.pending_updates(&mut updates);
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
        let announced = self.relays(api.conversation_id(), &call_id, name);
        let pending = self
            .begin(|host, ctx| host.tool(name, &call_id, args, announced, ctx))
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
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let answer = self.ask(name, payload).ok();
        match name {
            "agent_end" => {
                self.0.settled.store(true, Ordering::SeqCst);
                self.end_prompt_run();
            }
            "session_shutdown" => *lock(&self.0.session) = None,
            "session_start" => self.discover(reason.as_deref().unwrap_or_default()),
            _ => {}
        }
        answer
    }

    pub fn observe(&self, event: &AgentEvent) {
        if let AgentEvent::RunEnd { .. } = event {
            self.end_run();
        }
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

    pub fn hooks(&self) -> Vec<HookRegistration> {
        let streamed = self.clone();
        let ended = self.clone();
        let requested = self.clone();
        let finished = self.clone();
        let router = self.clone();
        let answered = self.clone();
        let ahead = self.clone();
        vec![
            HookRegistration::new(
                &GENERATION_HOOK,
                GenerationHooks {
                    before_request: Some(Box::new(move |api: &HookApi, messages: &[Message]| {
                        requested.turn_start();
                        let Some(root) = requested.root_of(api) else {
                            return Ok(requested.forced(messages));
                        };
                        requested.remember_thinking(&root);
                        requested.announce(&root);
                        let context = requested.context(messages)?;
                        Ok(requested
                            .forced(context.as_deref().unwrap_or(messages))
                            .or(context))
                    })),
                    routes: Some(Box::new(crate::models::is_virtual)),
                    route: Some(Box::new(
                        move |_api: &HookApi, model: &Model, messages: &[Message], level, signal| {
                            router.routed(model, messages, level, signal)
                        },
                    )),
                    after_response: Some(Box::new(
                        move |_api: &HookApi, message: &AssistantMessage| {
                            answered.responded(message);
                            Ok(())
                        },
                    )),
                    after_tools: Some(Box::new(
                        move |api: &HookApi, _assistant: EntryId, _results: &[EntryId]| {
                            if let Some(root) = finished.root_of(api) {
                                finished.tools_done(&root);
                            }
                            finished.turn_end();
                            Ok(())
                        },
                    )),
                    on_stream: Some(Arc::new(
                        move |_api: &HookApi,
                              event: &AssistantMessageEvent,
                              current: &dyn Fn() -> Option<AssistantMessage>| {
                            streamed.streaming(event, current)
                        },
                    )),
                    message_end: Some(Arc::new(move |_api: &HookApi, message: &Message| {
                        ended.replaced(message)
                    })),
                    before_tools: Some(Box::new(move |api: &HookApi, round: &ToolRound<'_>| {
                        ahead.tools_ahead(api, round);
                        Ok(())
                    })),
                    ..GenerationHooks::default()
                },
            ),
            self.tool_hooks(),
        ]
    }

    fn root(&self) -> Option<Conversation> {
        lock(&self.0.session)
            .as_ref()
            .map(|source| source.root.clone())
    }

    fn root_of(&self, api: &HookApi) -> Option<Conversation> {
        self.root().filter(|root| root.id() == api.conversation_id)
    }

    fn skip_announced(&self, root: &Conversation, cancel: &CancelToken) {
        if let Ok(last) = crate::print::last_entry(root, cancel) {
            *lock(&self.0.announced) = last;
        }
    }

    fn remember_thinking(&self, root: &Conversation) {
        if let Ok(agent) = root.agent(&CancelToken::new()) {
            *lock(&self.0.thinking) = Some(crate::run_config::level_name(agent.thinking_level));
        }
    }

    fn upstream(&self, message: &Message) -> std::result::Result<Value, String> {
        let mut value = serde_json::to_value(message).map_err(|e| e.to_string())?;
        if let Some(thinking) = lock(&self.0.thinking).as_deref() {
            crate::sessions::export::upstream_message(&mut value, thinking);
        }
        Ok(value)
    }

    fn announce(&self, root: &Conversation) {
        let messages = match self.unannounced(root) {
            Ok(messages) => messages,
            Err(error) => {
                self.0.sink.call(Call::Notify {
                    text: format!("Warning: the extensions were not told of new messages: {error}"),
                    level: "warning".to_owned(),
                });
                return;
            }
        };
        self.tell(messages);
    }

    fn tell(&self, messages: Vec<Value>) {
        for message in messages {
            self.event("message_start", json!({ "message": message }));
            if let Some(Value::Object(_)) = self.event("message_end", json!({ "message": message }))
            {
                self.unported("a message_end handler replaced a message the session already holds");
            }
            lock(&self.0.run).messages.push(message);
        }
    }

    fn unannounced(&self, root: &Conversation) -> std::result::Result<Vec<Value>, String> {
        let records = {
            let mut announced = lock(&self.0.announced);
            let records =
                crate::sessions::export::entries_after(root, *announced, &CancelToken::new())?;
            if let Some(last) = records.last() {
                *announced = Some(last.id);
            }
            records
        };
        let assistant = pi_durable::entries::assistant_entry();
        let mut inputs: Vec<EntryRecord> = records
            .into_iter()
            .filter(|record| {
                record.kind != assistant.kind() && record.kind != crate::bash_exec::KIND
            })
            .collect();
        self.in_call_order(&mut inputs);
        let thinking = lock(&self.0.thinking).clone().unwrap_or_default();
        crate::sessions::export::run_messages(&inputs, &thinking)
    }

    fn context(&self, messages: &[Message]) -> std::result::Result<Option<Vec<Message>>, Error> {
        if !self.handles("context") && !self.handles("context_with_system") {
            return Ok(None);
        }
        let payload = messages
            .iter()
            .map(|message| self.upstream(message))
            .collect::<std::result::Result<Vec<Value>, String>>()
            .map_err(Error::Invalid)?;
        let Ok(answer) = self.ask("context", json!({ "messages": payload })) else {
            return Ok(None);
        };
        match serde_json::from_value::<Option<Transformed>>(answer) {
            Ok(transformed) => Ok(transformed.map(|transformed| transformed.messages)),
            Err(e) => {
                self.0.sink.call(Call::Notify {
                    text: format!(
                        "Warning: a context handler returned {e}; the request keeps its messages"
                    ),
                    level: "warning".to_owned(),
                });
                Ok(None)
            }
        }
    }

    fn streaming(
        &self,
        event: &AssistantMessageEvent,
        current: &dyn Fn() -> Option<AssistantMessage>,
    ) {
        match event {
            AssistantMessageEvent::Start { partial } => {
                self.0.started.store(true, Ordering::SeqCst);
                if let Ok(message) = serde_json::to_value(partial) {
                    self.event("message_start", json!({ "message": message }));
                }
            }
            AssistantMessageEvent::Done { .. } | AssistantMessageEvent::Error { .. } => {}
            _ if !self.0.started.load(Ordering::SeqCst) || !self.handles("message_update") => {}
            _ => {
                let Ok(mut update) = serde_json::to_value(event) else {
                    return;
                };
                let message = current()
                    .and_then(|current| serde_json::to_value(current).ok())
                    .unwrap_or_else(|| update["partial"].clone());
                update["partial"] = message.clone();
                self.event(
                    "message_update",
                    json!({ "message": message, "assistantMessageEvent": update }),
                );
            }
        }
    }

    fn replaced(&self, message: &Message) -> std::result::Result<Option<Message>, Error> {
        let started = self.0.started.swap(false, Ordering::SeqCst);
        let payload = self.upstream(message).map_err(Error::Invalid)?;
        if !started {
            self.event("message_start", json!({ "message": payload }));
        }
        let answer = self
            .ask("message_end", json!({ "message": payload }))
            .map_err(Error::Invalid)?;
        let Some(next) = answer.get("message").cloned() else {
            lock(&self.0.run).messages.push(payload);
            return Ok(None);
        };
        let parsed = serde_json::from_value(next.clone())
            .map_err(|e| Error::Invalid(format!("a message_end handler returned {e}")))?;
        lock(&self.0.run).messages.push(next);
        Ok(Some(parsed))
    }

    fn ask(&self, name: &str, payload: Value) -> std::result::Result<Value, String> {
        self.finish(self.begin(|host, ctx| host.event(name, payload, ctx)))
    }
}
