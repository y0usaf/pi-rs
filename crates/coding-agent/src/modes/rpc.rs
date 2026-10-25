//! `--mode rpc`: commands as JSON lines on stdin, responses and the session's events as JSON
//! lines on stdout (`modes/rpc/rpc-mode.ts`). Events are the same lines `--mode json` prints;
//! responses answer a command by its `id`. A command whose type pi-rs does not carry answers
//! `"<type> is not implemented"`; a type no mode has answers `"Unknown command: <type>"`, as
//! upstream does.

use std::cell::RefCell;
use std::io::BufRead;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use pi_durable::ai::thinking::supported_thinking_levels;
use pi_durable::ai::types::{
    AssistantBlock, ImageContent, Message, Model, ModelThinkingLevel, TextContent, TextType,
    UserBlock, UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::compaction::estimate_context;
use pi_durable::harness::events::{AgentEvent, AgentEventStream};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::inbox::{InboxItem, InboxState, inbox_doc, withdraw_queued_inputs};
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::{ErrorReport, ModelRef};
use pi_durable::harness::util::read_draft;
use pi_durable::types::{EntryQuery, ScanOrder};
use serde_json::{Value, json};

use super::{Output, json_event, value};
use crate::args::is_valid_thinking_level;
use crate::models::ModelRegistry;
use crate::print;
use crate::run_config::{self, level_name};
use crate::session_fork;
use crate::session_info;
use crate::session_list;
use crate::session_open::{Open, Sink, Source};
use crate::session_tree::{self, Tree};
use crate::settings::SettingsManager;

pub struct Options {
    pub model: pi_durable::ai::types::Model,
    pub thinking: pi_durable::ai::types::ModelThinkingLevel,
    pub registry: Arc<ModelRegistry>,
    /// The SQLite file, or `None` under `--no-session`.
    pub session_path: Option<String>,
    /// Where a new session file goes (`/new`).
    pub session_dir: String,
    /// How a session is opened, for the commands that replace it.
    pub source: Source,
    pub cwd: String,
    /// The settings the queue modes, the compaction and retry toggles are saved to.
    pub settings: Rc<RefCell<SettingsManager>>,
    /// The commands a `prompt` may invoke, as `get_commands` reports them.
    pub commands: Vec<Value>,
    /// The prompt templates a `prompt` may invoke by name.
    pub templates: Vec<crate::prompt_templates::PromptTemplate>,
}

/// A `bash` command running on its own thread, kept so the loop carries on reading commands
/// (upstream's `_bashAbortControllers`).
struct BashRun {
    id: Option<Value>,
    command: String,
    exclude_from_context: bool,
    cancel: CancelToken,
    rx: std::sync::mpsc::Receiver<crate::bash_exec::Update>,
}

/// The `BashResult` a `bash` command answers with, absent fields left out as upstream leaves
/// them undefined.
fn bash_result(outcome: &crate::bash_exec::Outcome) -> Value {
    let mut result = serde_json::Map::new();
    result.insert("output".into(), json!(outcome.output));
    if let Some(code) = outcome.exit_code {
        result.insert("exitCode".into(), json!(code));
    }
    result.insert("cancelled".into(), json!(outcome.cancelled));
    result.insert("truncated".into(), json!(outcome.truncated));
    if let Some(path) = &outcome.full_output_path {
        result.insert("fullOutputPath".into(), json!(path));
    }
    Value::Object(result)
}

/// `bash_execution_update` for each chunk, the `bash` response and the recorded message when a
/// run ends.
fn drain_bash(runs: &mut Vec<BashRun>, root: &Conversation, cancel: &CancelToken, output: &Output) {
    let mut index = 0;
    while index < runs.len() {
        let finished = {
            let run = &mut runs[index];
            let mut finished = false;
            loop {
                match run.rx.try_recv() {
                    Ok(crate::bash_exec::Update::Chunk(delta)) => {
                        let mut line = serde_json::Map::new();
                        if let Some(id) = &run.id {
                            line.insert("id".into(), id.clone());
                        }
                        line.insert("type".into(), json!("bash_execution_update"));
                        line.insert("delta".into(), json!(delta));
                        output.line(&Value::Object(line));
                    }
                    Ok(crate::bash_exec::Update::Done(result)) => {
                        let id = run.id.clone();
                        let response = match result {
                            Ok(outcome) => {
                                if let Err(e) = crate::bash_exec::record(
                                    root,
                                    cancel,
                                    &run.command,
                                    &outcome,
                                    run.exclude_from_context,
                                ) {
                                    failure(id.as_ref(), "bash", e)
                                } else {
                                    success(id.as_ref(), "bash", Some(bash_result(&outcome)))
                                }
                            }
                            Err(e) => failure(id.as_ref(), "bash", e),
                        };
                        output.line(&response);
                        finished = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
            finished
        };
        if finished {
            runs.remove(index);
        } else {
            index += 1;
        }
    }
}

/// What the session state is, kept from the events as they arrive.
struct State {
    model: pi_durable::ai::types::Model,
    thinking: pi_durable::ai::types::ModelThinkingLevel,
    streaming: bool,
    compacting: u32,
    pending: usize,
    retrying: bool,
    steering_mode: String,
    follow_up_mode: String,
    auto_compaction: bool,
    auto_retry: bool,
}

impl State {
    fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::RunStart { .. } => self.streaming = true,
            AgentEvent::RunEnd { .. } => self.streaming = false,
            AgentEvent::CompactionStart { .. } => self.compacting += 1,
            AgentEvent::CompactionEnd { .. } => self.compacting = self.compacting.saturating_sub(1),
            AgentEvent::InboxUpdate { items } => self.pending = items.len(),
            AgentEvent::AutoRetryStart { .. } => self.retrying = true,
            AgentEvent::AutoRetryEnd { .. } => self.retrying = false,
            _ => {}
        }
    }
}

fn success(id: Option<&Value>, command: &str, data: Option<Value>) -> Value {
    let mut line = response(id, command, true);
    if let Some(data) = data {
        line["data"] = data;
    }
    line
}

fn failure(id: Option<&Value>, command: &str, message: impl Into<String>) -> Value {
    let mut line = response(id, command, false);
    line["error"] = json!(message.into());
    line
}

/// Upstream writes a response's `id` first, then `type`, `command` and `success`.
fn response(id: Option<&Value>, command: &str, ok: bool) -> Value {
    let mut line = serde_json::Map::new();
    if let Some(id) = id {
        line.insert("id".into(), id.clone());
    }
    line.insert("type".into(), json!("response"));
    line.insert("command".into(), json!(command));
    line.insert("success".into(), json!(ok));
    Value::Object(line)
}

fn user_content(
    message: &Value,
    templates: &[crate::prompt_templates::PromptTemplate],
) -> UserContent {
    let mut blocks: Vec<UserBlock> = Vec::new();
    if let Some(text) = message["message"].as_str() {
        blocks.push(UserBlock::Text(TextContent {
            kind: TextType,
            text: crate::prompt_templates::expand(text, templates),
            text_signature: None,
        }));
    }
    for image in message["images"].as_array().into_iter().flatten() {
        if let Ok(image) = serde_json::from_value::<ImageContent>(image.clone()) {
            blocks.push(UserBlock::Image(image));
        }
    }
    UserContent::Blocks(blocks)
}

fn transcript(root: &Conversation, cancel: &CancelToken) -> Result<Vec<Message>, String> {
    print::transcript(root, cancel).map_err(|e| e.to_string())
}

/// The answers `get_commands` gives: what a typed message may invoke.
fn commands(options: &Options) -> Value {
    json!({ "commands": options.commands })
}

/// The text of a queued message, which `clear_queue` hands back to the host.
fn queued_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserBlock::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<String>>()
            .join(""),
    }
}

/// Empty the inbox of the messages waiting behind the run and hand back their text: upstream's
/// `clearQueue`. A queued write stays, as it is not user input.
fn clear_queue(
    root: &Conversation,
    cancel: &CancelToken,
) -> Result<(Vec<String>, Vec<String>), String> {
    let items: Vec<InboxItem> = root
        .commit(
            |tx| {
                let token = inbox_doc()?;
                let state: InboxState = read_draft(tx, &token, root.id())?;
                withdraw_queued_inputs(tx, root.id())?;
                Ok(state.items)
            },
            cancel,
        )
        .map_err(|e| e.to_string())?;
    let mut steering = Vec::new();
    let mut follow_up = Vec::new();
    for item in items {
        match item {
            InboxItem::Steer { content, .. } => steering.push(queued_text(&content)),
            InboxItem::FollowUp { content, .. } => follow_up.push(queued_text(&content)),
            InboxItem::Write { .. } => {}
        }
    }
    Ok((steering, follow_up))
}

/// Upstream's `getTree`: every entry of the session, nested by the branch it sits on.
fn tree_node(tree: &Tree, index: usize) -> Value {
    json!({
        "entry": value(&tree.nodes[index].entry),
        "children": tree.nodes[index]
            .children
            .iter()
            .map(|child| tree_node(tree, *child))
            .collect::<Vec<Value>>(),
    })
}

/// Upstream's `getSessionStats`: counts over every entry the session holds, the usage recorded for
/// it, and what its context costs now.
fn session_stats(
    harness: &Harness,
    root: &Conversation,
    cancel: &CancelToken,
    model: &Model,
    session_path: Option<&str>,
) -> Result<Value, String> {
    let tree = Tree::build(root, cancel).map_err(|e| e.to_string())?;
    let (mut user, mut assistant, mut tool_calls, mut tool_results, mut system) = (0, 0, 0, 0, 0);
    for node in &tree.nodes {
        let entry = &node.entry;
        let kind = entry.kind.as_str();
        if kind == pi_durable::entries::user_entry().kind() {
            user += 1;
        } else if kind == pi_durable::entries::assistant_entry().kind() {
            assistant += 1;
            for message in entry.model.iter().flatten() {
                if let Message::Assistant(message) = message {
                    tool_calls += message
                        .content
                        .iter()
                        .filter(|block| matches!(block, AssistantBlock::ToolCall(_)))
                        .count();
                }
            }
        } else if kind == pi_durable::entries::tool_result_entry().kind() {
            tool_results += 1;
        } else if kind == pi_durable::entries::system_entry().kind() {
            system += 1;
        }
    }
    let usage = harness.usage(cancel).map_err(|e| e.to_string())?;
    let (mut input, mut output, mut cache_read, mut cache_write) = (0u64, 0u64, 0u64, 0u64);
    let mut cost = 0.0f64;
    for recorded in usage.models.values().chain(usage.tools.values()) {
        input += recorded.input;
        output += recorded.output;
        cache_read += recorded.cache_read;
        cache_write += recorded.cache_write;
        cost += recorded.cost.total;
    }
    let mut stats = json!({
        "sessionId": root.id().0,
        "userMessages": user,
        "assistantMessages": assistant,
        "toolCalls": tool_calls,
        "toolResults": tool_results,
        "totalMessages": user + assistant + tool_results + system,
        "tokens": {
            "input": input,
            "output": output,
            "cacheRead": cache_read,
            "cacheWrite": cache_write,
            "total": input + output + cache_read + cache_write,
        },
        "cost": cost,
    });
    if let Some(path) = session_path {
        stats["sessionFile"] = json!(path);
    }
    if model.context_window > 0 {
        let view = root.context(None, cancel).map_err(|e| e.to_string())?;
        let tokens = estimate_context(&view, &[]);
        stats["contextUsage"] = json!({
            "tokens": tokens,
            "contextWindow": model.context_window,
            "percent": tokens as f64 * 100.0 / model.context_window as f64,
        });
    }
    Ok(stats)
}

pub fn run(
    harness: Harness,
    root: Conversation,
    options: Options,
    cancel: &CancelToken,
) -> Result<(), String> {
    let output = Output::default();
    let state = Arc::new(Mutex::new(State {
        model: options.model.clone(),
        thinking: options.thinking,
        streaming: false,
        compacting: 0,
        pending: 0,
        retrying: false,
        steering_mode: options.settings.borrow().steering_mode().to_owned(),
        follow_up_mode: options.settings.borrow().follow_up_mode().to_owned(),
        auto_compaction: options.settings.borrow().compaction_enabled(),
        auto_retry: options.settings.borrow().retry_enabled(),
    }));

    // The conversation's agent starts with no model; the run's settings are the session's.
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: options.model.provider.clone(),
                model_id: options.model.id.clone(),
            }),
            thinking_level: Change::Set(options.thinking),
            cwd: Change::Set(options.cwd.clone()),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;

    let mut open = Open {
        path: options.session_path.clone(),
        stream: attach(&root, &output, &state, cancel)?,
        harness,
        root,
    };

    // A thread reads stdin, so a running `bash` command's output and `abort_bash` reach the
    // client while nothing is typed.
    let (lines, reading) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if lines.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut runs: Vec<BashRun> = Vec::new();
    loop {
        match reading.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(text) => {
                let text = text.trim_end_matches(['\n', '\r']);
                if !text.is_empty() {
                    match serde_json::from_str::<Value>(text) {
                        Err(error) => output.line(&failure(
                            None,
                            "parse",
                            format!("Failed to parse command: {error}"),
                        )),
                        Ok(parsed) => {
                            if parsed["type"] != "extension_ui_response"
                                && let Some(response) = handle(
                                    &mut open, &output, &options, &state, &parsed, cancel,
                                    &mut runs,
                                )
                            {
                                output.line(&response);
                            }
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        drain_bash(&mut runs, &open.root, cancel, &output);
    }

    open.stream.stop();
    let _ = open.root.wait_for_idle(cancel);
    open.shutdown(cancel)
}

/// Where a session's scheduler errors go: RPC has no chat to show them in.
fn report() -> ErrorReport {
    Arc::new(|error| eprintln!("{error}"))
}

/// Where a session's events go: one JSON line each, as --mode json prints them.
fn sink(output: &Output, state: &Arc<Mutex<State>>) -> Sink {
    let printer = output.clone();
    let shared = Arc::clone(state);
    let mut mapper = json_event::Mapper::default();
    Box::new(move |events, _| {
        for event in events {
            if let Ok(mut state) = shared.lock() {
                state.apply(event);
            }
            for line in mapper.lines(event) {
                printer.line(&line);
            }
        }
        Ok(())
    })
}

/// Subscribe to a session's events, printing them as they arrive.
fn attach(
    root: &Conversation,
    output: &Output,
    state: &Arc<Mutex<State>>,
    cancel: &CancelToken,
) -> Result<AgentEventStream, String> {
    let stream = root.events(cancel).map_err(|e| e.to_string())?;
    stream
        .start(sink(output, state))
        .map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Replace the session with the one at target, then point it at the model and thinking level it
/// recorded for itself, or at this run's when it recorded none: upstream's switchSession,
/// newSession, fork and clone.
fn switch(
    open: &mut Open,
    output: &Output,
    options: &Options,
    state: &Arc<Mutex<State>>,
    target: Option<&str>,
    cancel: &CancelToken,
) -> Result<(), String> {
    let next = sink(output, state);
    open.swap(&options.source, target, report(), next, cancel)?;
    let recorded = run_config::recorded_agent(&open.root, cancel, &options.registry)?;
    let (model, thinking) = match recorded {
        Some(recorded) => recorded,
        None => {
            let shared = state
                .lock()
                .map_err(|_| "the session state is poisoned".to_owned())?;
            (shared.model.clone(), shared.thinking)
        }
    };
    let cwd = open
        .root
        .agent(cancel)
        .ok()
        .and_then(|agent| agent.cwd)
        .unwrap_or_else(|| options.cwd.clone());
    run_config::apply_agent(&open.root, cancel, &model, thinking, &cwd)?;
    let mut shared = state
        .lock()
        .map_err(|_| "the session state is poisoned".to_owned())?;
    shared.model = model;
    shared.thinking = thinking;
    Ok(())
}

/// The thinking level the session is at, for the model switches that keep it.
fn thinking_of(state: &Arc<Mutex<State>>) -> ModelThinkingLevel {
    state
        .lock()
        .map(|state| state.thinking)
        .unwrap_or(ModelThinkingLevel::Off)
}

fn handle(
    open: &mut Open,
    output: &Output,
    options: &Options,
    state: &Arc<Mutex<State>>,
    command: &Value,
    cancel: &CancelToken,
    runs: &mut Vec<BashRun>,
) -> Option<Value> {
    let id = command.get("id").cloned();
    let id = id.as_ref();
    let kind = command["type"].as_str().unwrap_or_default();
    let harness = &open.harness;
    let root = &open.root;
    let response = match kind {
        "prompt" | "steer" | "follow_up" => {
            let when_busy = match kind {
                "steer" => WhenBusy::Steer,
                _ => match command["streamingBehavior"].as_str() {
                    Some("steer") => WhenBusy::Steer,
                    _ => WhenBusy::FollowUp,
                },
            };
            let draft = SubmissionDraft::Input {
                request_id: id.map(|id| id.to_string()),
                content: user_content(command, &options.templates),
                when_busy,
            };
            match root.submit(draft, cancel) {
                Ok(handle) => match handle.status(cancel) {
                    Ok(record) => {
                        let queued = matches!(
                            record.body,
                            pi_durable::types::SubmissionBody::Input(
                                pi_durable::types::InputState::Queued
                            )
                        );
                        let disposition = if queued {
                            "queued"
                        } else if kind == "prompt" {
                            "started"
                        } else {
                            "handled"
                        };
                        success(id, kind, Some(json!({ "disposition": disposition })))
                    }
                    Err(error) => failure(id, kind, error.to_string()),
                },
                Err(error) => failure(id, kind, error.to_string()),
            }
        }

        "abort" => match root.abort(false, cancel) {
            Ok(()) => success(id, kind, None),
            Err(error) => failure(id, kind, error.to_string()),
        },

        "clear_queue" => match clear_queue(root, cancel) {
            Ok((steering, follow_up)) => success(
                id,
                kind,
                Some(json!({ "steering": steering, "followUp": follow_up })),
            ),
            Err(error) => failure(id, kind, error),
        },

        "new_session" => {
            let target = open
                .path
                .as_ref()
                .map(|_| session_list::new_path(&options.session_dir));
            match switch(open, output, options, state, target.as_deref(), cancel) {
                Err(error) => failure(id, kind, error),
                Ok(()) => {
                    if let Some(parent) = command["parentSession"].as_str()
                        && let Err(error) =
                            session_info::set_parent_session(&open.root, cancel, parent)
                    {
                        return Some(failure(id, kind, error.to_string()));
                    }
                    success(id, kind, Some(json!({ "cancelled": false })))
                }
            }
        }

        "switch_session" => {
            let session_path = command["sessionPath"].as_str().unwrap_or_default();
            if !std::path::Path::new(session_path).is_file() {
                return Some(failure(
                    id,
                    kind,
                    format!("Session not found: {session_path}"),
                ));
            }
            match switch(open, output, options, state, Some(session_path), cancel) {
                Ok(()) => success(id, kind, Some(json!({ "cancelled": false }))),
                Err(error) => failure(id, kind, error),
            }
        }

        "fork" => {
            let tree = match Tree::build(&open.root, cancel) {
                Err(error) => return Some(failure(id, kind, error.to_string())),
                Ok(tree) => tree,
            };
            let entry = command["entryId"].as_u64().or_else(|| {
                command["entryId"]
                    .as_str()
                    .and_then(|entry| entry.parse().ok())
            });
            let found = entry
                .map(pi_durable::ids::EntryId)
                .and_then(|entry| tree.entry(entry).map(|record| (entry, record)));
            let Some((entry, record)) = found else {
                return Some(failure(id, kind, "Invalid entry ID for forking"));
            };
            let Some(text) = session_tree::user_message_text(record) else {
                return Some(failure(id, kind, "Invalid entry ID for forking"));
            };
            let point = tree
                .node(entry)
                .and_then(|node| node.parent)
                .map(|parent| tree.nodes[parent].entry.id);
            let path = match session_fork::branch_from(open.path.as_deref(), &tree, point) {
                Err(error) => return Some(failure(id, kind, error)),
                Ok(path) => path,
            };
            let (text, path) = (text, path);
            match switch(open, output, options, state, Some(&path), cancel) {
                Err(error) => failure(id, kind, error),
                Ok(()) => success(id, kind, Some(json!({ "text": text, "cancelled": false }))),
            }
        }

        "clone" => {
            let tree = match Tree::build(&open.root, cancel) {
                Err(error) => return Some(failure(id, kind, error.to_string())),
                Ok(tree) => tree,
            };
            let Some(leaf) = tree.leaf else {
                return Some(failure(
                    id,
                    kind,
                    "Cannot clone session: no current entry selected",
                ));
            };
            let path = match session_fork::branch_from(open.path.as_deref(), &tree, Some(leaf)) {
                Err(error) => return Some(failure(id, kind, error)),
                Ok(path) => path,
            };
            match switch(open, output, options, state, Some(&path), cancel) {
                Ok(()) => success(id, kind, Some(json!({ "cancelled": false }))),
                Err(error) => failure(id, kind, error),
            }
        }

        "get_state" => {
            let mut shared = state.lock().map_err(|_| ()).ok();
            let (model, thinking, streaming, compacting, pending, steering, follow_up, compaction) =
                match &mut shared {
                    Some(state) => (
                        Some(value(&state.model)),
                        json!(level_name(state.thinking)),
                        state.streaming,
                        state.compacting > 0,
                        state.pending,
                        state.steering_mode.clone(),
                        state.follow_up_mode.clone(),
                        state.auto_compaction,
                    ),
                    None => (
                        None,
                        json!("off"),
                        false,
                        false,
                        0,
                        String::new(),
                        String::new(),
                        false,
                    ),
                };
            let count = transcript(root, cancel).map(|m| m.len()).unwrap_or(0);
            let mut state = json!({
                "model": model,
                "thinkingLevel": thinking,
                "isStreaming": streaming,
                "isCompacting": compacting,
                "steeringMode": steering,
                "followUpMode": follow_up,
                "sessionId": root.id().0,
                "autoCompactionEnabled": compaction,
                "messageCount": count,
                "pendingMessageCount": pending,
            });
            if let Some(path) = &open.path {
                state["sessionFile"] = json!(path);
            }
            if let Some(name) = session_info::name(harness, cancel) {
                state["sessionName"] = json!(name);
            }
            success(id, kind, Some(state))
        }

        "set_model" => {
            let provider = command["provider"].as_str().unwrap_or_default();
            let model_id = command["modelId"].as_str().unwrap_or_default();
            match options.registry.get_model(provider, model_id).cloned() {
                None => failure(id, kind, format!("Model not found: {provider}/{model_id}")),
                Some(model) => {
                    let thinking = thinking_of(state);
                    let settings = options.settings.borrow();
                    match run_config::set_model(root, cancel, &settings, &model, thinking) {
                        Err(error) => failure(id, kind, error),
                        Ok(level) => {
                            if let Ok(mut state) = state.lock() {
                                state.model = model.clone();
                                state.thinking = level;
                            }
                            success(id, kind, Some(value(&model)))
                        }
                    }
                }
            }
        }

        "cycle_model" => {
            let available = run_config::available(&options.registry);
            let scoped = run_config::scope(&options.settings.borrow(), &available);
            let is_scoped = !scoped.is_empty();
            let models = if is_scoped { scoped } else { available };
            match state.lock().ok().map(|state| state.model.clone()) {
                None => failure(id, kind, "No model selected"),
                Some(_) if models.len() <= 1 => success(id, kind, Some(Value::Null)),
                Some(current) => {
                    let index = models
                        .iter()
                        .position(|model| {
                            model.provider == current.provider && model.id == current.id
                        })
                        .unwrap_or(0);
                    let next = models[(index + 1) % models.len()].clone();
                    let thinking = thinking_of(state);
                    let settings = options.settings.borrow();
                    match run_config::set_model(root, cancel, &settings, &next, thinking) {
                        Err(error) => failure(id, kind, error),
                        Ok(level) => {
                            if let Ok(mut state) = state.lock() {
                                state.model = next.clone();
                                state.thinking = level;
                            }
                            success(
                                id,
                                kind,
                                Some(json!({
                                    "model": value(&next),
                                    "thinkingLevel": level_name(level),
                                    "isScoped": is_scoped,
                                })),
                            )
                        }
                    }
                }
            }
        }

        "get_available_models" => {
            let models: Vec<Value> = options
                .registry
                .available()
                .into_iter()
                .map(value)
                .collect();
            success(id, kind, Some(json!({ "models": models })))
        }

        "set_thinking_level" => match command["level"].as_str() {
            Some(level) if is_valid_thinking_level(level) => {
                match serde_json::from_value::<ModelThinkingLevel>(json!(level)) {
                    Err(error) => failure(id, kind, error.to_string()),
                    Ok(level) => match run_config::set_thinking(root, cancel, level) {
                        Err(error) => failure(id, kind, error),
                        Ok(()) => {
                            if let Ok(mut state) = state.lock() {
                                state.thinking = level;
                            }
                            success(id, kind, None)
                        }
                    },
                }
            }
            _ => failure(
                id,
                kind,
                format!("Invalid thinking level: {}", command["level"]),
            ),
        },

        "cycle_thinking_level" => match state.lock().ok().map(|s| (s.model.clone(), s.thinking)) {
            None => failure(id, kind, "No model selected"),
            Some((model, _)) if !model.reasoning => success(id, kind, Some(Value::Null)),
            Some((model, thinking)) => {
                let levels = supported_thinking_levels(&model);
                let index = levels
                    .iter()
                    .position(|level| *level == thinking)
                    .unwrap_or(0);
                let next = levels[(index + 1) % levels.len()];
                match run_config::set_thinking(root, cancel, next) {
                    Err(error) => failure(id, kind, error),
                    Ok(()) => {
                        if let Ok(mut state) = state.lock() {
                            state.thinking = next;
                        }
                        success(id, kind, Some(json!({ "level": level_name(next) })))
                    }
                }
            }
        },

        "get_available_thinking_levels" => {
            let levels: Vec<String> = state
                .lock()
                .map(|state| {
                    supported_thinking_levels(&state.model)
                        .into_iter()
                        .map(level_name)
                        .collect()
                })
                .unwrap_or_else(|_| vec!["off".to_owned()]);
            success(id, kind, Some(json!({ "levels": levels })))
        }

        "set_steering_mode" => {
            let mode = command["mode"].as_str().unwrap_or_default();
            options.settings.borrow_mut().set_steering_mode(mode);
            if let Ok(mut state) = state.lock() {
                state.steering_mode = mode.to_owned();
            }
            success(id, kind, None)
        }

        "set_follow_up_mode" => {
            let mode = command["mode"].as_str().unwrap_or_default();
            options.settings.borrow_mut().set_follow_up_mode(mode);
            if let Ok(mut state) = state.lock() {
                state.follow_up_mode = mode.to_owned();
            }
            success(id, kind, None)
        }

        "compact" => {
            let instructions = command["customInstructions"].as_str();
            match root.compact(instructions, cancel) {
                Ok(task) => success(id, kind, Some(json!({ "taskId": value(&task) }))),
                Err(error) => failure(id, kind, error.to_string()),
            }
        }

        "set_auto_compaction" => {
            let enabled = command["enabled"].as_bool().unwrap_or(false);
            options
                .settings
                .borrow_mut()
                .set_compaction_enabled(enabled);
            if let Ok(mut state) = state.lock() {
                state.auto_compaction = enabled;
            }
            success(id, kind, None)
        }

        "set_auto_retry" => {
            let enabled = command["enabled"].as_bool().unwrap_or(false);
            options.settings.borrow_mut().set_retry_enabled(enabled);
            if let Ok(mut state) = state.lock() {
                state.auto_retry = enabled;
            }
            success(id, kind, None)
        }

        "abort_retry" => {
            let retrying = state.lock().map(|state| state.retrying).unwrap_or(false);
            if retrying {
                match root.abort(false, cancel) {
                    Ok(()) => success(id, kind, None),
                    Err(error) => failure(id, kind, error.to_string()),
                }
            } else {
                success(id, kind, None)
            }
        }

        "get_messages" => match transcript(root, cancel) {
            Ok(messages) => success(id, kind, Some(json!({ "messages": messages }))),
            Err(error) => failure(id, kind, error),
        },

        "get_last_assistant_text" => match transcript(root, cancel) {
            Ok(messages) => {
                let text = messages.iter().rev().find_map(|message| match message {
                    Message::Assistant(assistant) => Some(
                        assistant
                            .content
                            .iter()
                            .filter_map(|block| match block {
                                AssistantBlock::Text(text) => Some(text.text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<String>>()
                            .join(""),
                    ),
                    _ => None,
                });
                match text {
                    Some(text) => success(id, kind, Some(json!({ "text": text }))),
                    // Upstream answers the key with a null text.
                    None => success(id, kind, Some(json!({}))),
                }
            }
            Err(error) => failure(id, kind, error),
        },

        "get_entries" => {
            let query = EntryQuery {
                conversation_id: root.id(),
                min_entry_id: None,
                max_entry_id: None,
                order: Some(ScanOrder::Ascending),
            };
            match root.entries(query, 10_000, None, cancel) {
                Err(error) => failure(id, kind, error.to_string()),
                Ok(page) => {
                    let mut entries = page.items;
                    let leaf = entries.last().map(|entry| entry.id.0);
                    if let Some(since) = command["since"].as_str() {
                        match since.parse::<u64>() {
                            Err(_) => {
                                return Some(failure(
                                    id,
                                    kind,
                                    format!("Entry not found: {since}"),
                                ));
                            }
                            Ok(since) => match entries.iter().position(|entry| entry.id.0 == since)
                            {
                                None => {
                                    return Some(failure(
                                        id,
                                        kind,
                                        format!("Entry not found: {since}"),
                                    ));
                                }
                                Some(index) => {
                                    entries.drain(..=index);
                                }
                            },
                        }
                    }
                    success(
                        id,
                        kind,
                        Some(json!({ "entries": entries, "leafId": leaf })),
                    )
                }
            }
        }

        "get_tree" => match Tree::build(root, cancel) {
            Err(error) => failure(id, kind, error.to_string()),
            Ok(tree) => {
                let nodes: Vec<Value> = tree
                    .roots
                    .iter()
                    .map(|index| tree_node(&tree, *index))
                    .collect();
                success(
                    id,
                    kind,
                    Some(json!({ "tree": nodes, "leafId": tree.leaf.map(|id| id.0) })),
                )
            }
        },

        "get_fork_messages" => match Tree::build(root, cancel) {
            Err(error) => failure(id, kind, error.to_string()),
            Ok(tree) => {
                let messages: Vec<Value> = tree
                    .nodes
                    .iter()
                    .filter_map(|node| {
                        let text = session_tree::user_message_text(&node.entry)?;
                        (!text.is_empty())
                            .then(|| json!({ "entryId": node.entry.id.0, "text": text }))
                    })
                    .collect();
                success(id, kind, Some(json!({ "messages": messages })))
            }
        },

        "get_session_stats" => match state.lock().ok().map(|state| state.model.clone()) {
            None => failure(id, kind, "No model selected"),
            Some(model) => match session_stats(harness, root, cancel, &model, open.path.as_deref())
            {
                Err(error) => failure(id, kind, error),
                Ok(stats) => success(id, kind, Some(stats)),
            },
        },

        "get_commands" => success(id, kind, Some(commands(options))),

        "bash" => {
            let line = command["command"].as_str().unwrap_or_default().to_owned();
            let exclude_from_context = command["excludeFromContext"].as_bool().unwrap_or_default();
            let (shell, prefix) = {
                let settings = options.settings.borrow();
                (
                    settings.shell_path(),
                    settings.shell_command_prefix().map(str::to_owned),
                )
            };
            let (token, rx) =
                crate::bash_exec::start(&line, &options.cwd, shell.as_deref(), prefix.as_deref());
            runs.push(BashRun {
                id: id.cloned(),
                command: line,
                exclude_from_context,
                cancel: token,
                rx,
            });
            return None;
        }

        "abort_bash" => {
            for run in runs.iter() {
                run.cancel.cancel();
            }
            success(id, kind, None)
        }

        "set_session_name" => {
            let name = command["name"].as_str().unwrap_or_default().trim();
            if name.is_empty() {
                failure(id, kind, "Session name cannot be empty")
            } else {
                match session_info::set_name(root, cancel, name) {
                    Err(error) => failure(id, kind, error.to_string()),
                    Ok(()) => success(id, kind, None),
                }
            }
        }

        "export_html" => failure(
            id,
            kind,
            format!("pi-rs: {kind} is not implemented in RPC mode"),
        ),

        other => failure(id, other, format!("Unknown command: {other}")),
    };
    Some(response)
}
