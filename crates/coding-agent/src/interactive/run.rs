//! The interactive loop: the editor submits to the durable conversation, its agent events come
//! back over a channel and redraw the chat. Escape aborts the run, Ctrl+C clears the editor (twice
//! quits), Ctrl+D quits on an empty editor, Ctrl+O expands tool output.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pi_durable::ai::thinking::supported_thinking_levels;
use pi_durable::ai::types::{
    AssistantBlock, Message, Model, ModelThinkingLevel, TextContent, TextType, UserBlock,
    UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::events::AgentEvent;
use pi_durable::harness::harness::Conversation;
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::ErrorReport;
use pi_durable::ids::EntryId;
use pi_extensions::Call;
use pi_tui::autocomplete::CombinedAutocompleteProvider;
use pi_tui::components::editor::{Editor, EditorOptions};
use pi_tui::components::text::Text;
use pi_tui::keys::is_key_release;
use pi_tui::terminal::{ProcessTerminal, Terminal};
use pi_tui::tui::{ComponentRef, InputListenerResult, component};
use pi_tui::tui_main_screen::{TuiMainScreen, as_ref};
use serde_json::json;

use super::bash_mode::BashMode;
use super::chat::Chat;
use super::external_editor;
use super::footer::{Footer, FooterState, Totals, git_branch};
use super::keybindings;
use super::pending::PendingMessages;
use super::selectors::{
    ActionSlot, OverlayAction, action_slot,
    confirm::Confirm,
    fork::ForkSelector,
    login as login_selector,
    login::AuthSelector,
    login::LoginDialog,
    login::MethodSelector,
    model::ModelSelector,
    scoped_models::ScopedModelsSelector,
    session::SessionSelector,
    settings::SettingsCallbacks,
    settings::SettingsConfig,
    settings::SettingsSelector,
    thinking::ThinkingSelector,
    tree::{Filter, TreeSelector},
    trust::TrustSelector,
};
use super::slash_commands::{self, Command, Counts, SessionInfo};
use super::status::{StatusArea, WORKING, header};
use super::theme;
use crate::model_resolver;
use crate::models::ModelRegistry;
use crate::print::ToolSettings;
use crate::run_config::{self, level_name, parse_level};
use crate::session_info;
use crate::session_list;
use crate::session_open::{Open, Sink, Source};
use crate::session_tree::{self, Tree};
use crate::settings::{self as settings_mod, SettingsManager};

/// What the loop hears from other threads.
pub enum Incoming {
    Events(Vec<AgentEvent>),
    Report(String),
}

/// Opens a session's harness and root conversation. The loop owns what an open returns, and
/// opens another one when it switches session, so nothing here borrows the session it replaces.
#[derive(Clone)]
pub struct Opener {
    pub source: Source,
    pub sender: mpsc::Sender<Incoming>,
    pub reports: Arc<Mutex<mpsc::Sender<Incoming>>>,
}

impl Opener {
    /// Where a session's scheduler errors go: the chat, under the TUI.
    fn report(&self) -> ErrorReport {
        let reports = Arc::clone(&self.reports);
        Arc::new(move |error| {
            if let Ok(sender) = reports.lock() {
                let _ = sender.send(Incoming::Report(error.to_string()));
            }
        })
    }

    /// The loop's event feed, for a session opened now.
    fn sink(&self) -> Sink {
        let feed = self.sender.clone();
        Box::new(move |events, _| {
            let _ = feed.send(Incoming::Events(events.to_vec()));
            Ok(())
        })
    }

    /// Open the session at path, or a fresh in-memory one for None, with its events arriving as
    /// Incoming::Events.
    pub fn open(&self, path: Option<&str>) -> Result<Open, String> {
        self.source
            .open(path, self.report(), self.sink(), &CancelToken::new())
    }
}

pub struct Session {
    pub opener: Opener,
    /// Where /new puts its file; persists is false under --no-session.
    pub session_dir: String,
    pub persists: bool,
    pub model: Model,
    pub thinking: ModelThinkingLevel,
    pub cwd: String,
    /// The settings the selectors read and write.
    pub settings: Rc<RefCell<SettingsManager>>,
    /// The models a selector offers.
    pub registry: Arc<ModelRegistry>,
    /// Where `trust.json` lives.
    pub agent_dir: String,
    /// `--theme`: extra theme files or directories.
    pub theme_paths: Vec<String>,
    /// `--use-theme`: the theme to start with instead of the setting's.
    pub use_theme: Option<String>,
    /// `--no-themes`: the built-in themes only.
    pub no_themes: bool,
    /// The first message from the command line, sent once the screen is up.
    pub initial: Option<(String, Vec<UserBlock>)>,
    pub messages: Vec<String>,
    pub provider_count: usize,
    /// The SQLite file the loop starts in; `None` under `--no-session`.
    pub session_path: Option<String>,
    /// `--name`: the name the session starts with.
    pub initial_name: Option<String>,
    /// MCP servers that failed to connect or are configured wrongly, shown once at startup.
    pub mcp_problems: Vec<String>,
    /// The commands the extensions registered, and what loading them had to say.
    pub extensions: crate::extensions::Extensions,
    /// The prompt templates a typed line may invoke by name.
    pub templates: Vec<crate::prompt_templates::PromptTemplate>,
    pub incoming: mpsc::Receiver<Incoming>,
}

/// Keys the loop acts on, read by an input listener before the editor sees them.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Interrupt,
    Clear,
    Exit,
    Expand,
    FollowUp,
    Dequeue,
    ThinkingToggle,
    ThinkingCycle,
    /// Forward or backward through the models in scope.
    ModelCycle(bool),
    ModelSelect,
    ExternalEditor,
    Suspend,
    /// A session key, run as the slash command it names.
    Session(&'static str),
}

/// The components the loop swaps a selector in and out of.
struct Parts {
    /// None under quietStartup: true, which hides the startup header.
    header: Option<ComponentRef>,
    chat: ComponentRef,
    pending: ComponentRef,
    status: ComponentRef,
    editor: ComponentRef,
    footer: ComponentRef,
}

impl Parts {
    /// Draw the header, chat, the queued messages, status, footer and either the editor or
    /// `selector` in its place, focused.
    fn mount(&self, tui: &mut TuiMainScreen<ProcessTerminal>, selector: Option<&ComponentRef>) {
        tui.clear();
        if let Some(header) = &self.header {
            tui.add_child(header.clone());
        }
        tui.add_child(self.chat.clone());
        tui.add_child(self.pending.clone());
        tui.add_child(self.status.clone());
        let focus = match selector {
            Some(selector) => selector.clone(),
            None => self.editor.clone(),
        };
        tui.add_child(focus.clone());
        tui.add_child(self.footer.clone());
        tui.set_focus(Some(focus));
        tui.request_render(true);
    }
}

/// Set the session's model, clamping the thinking level to what the model accepts, and save it as
/// the default when asked.
fn apply_model(
    root: &Conversation,
    cancel: &CancelToken,
    settings: &mut SettingsManager,
    footer: &Rc<RefCell<Footer>>,
    model: &Model,
    persist: bool,
    thinking: &mut ModelThinkingLevel,
) -> Result<String, String> {
    let level = run_config::set_model(root, cancel, settings, model, *thinking)?;
    *thinking = level;
    {
        let mut footer = footer.borrow_mut();
        footer.state.model_id = Some(model.id.clone());
        footer.state.provider = Some(model.provider.clone());
        footer.state.context_window = model.context_window;
        footer.state.reasoning = model.reasoning;
        footer.state.thinking = level_name(level);
    }
    if persist {
        settings.set_default_model_and_provider(&model.provider, &model.id);
    }
    Ok(if persist {
        format!("Default model: {}/{}", model.provider, model.id)
    } else {
        format!("Model: {}", model.id)
    })
}

/// Set the thinking level for the session, and as the default when asked.
fn apply_thinking(
    root: &Conversation,
    cancel: &CancelToken,
    settings: &mut SettingsManager,
    footer: &Rc<RefCell<Footer>>,
    level: ModelThinkingLevel,
    persist: bool,
) -> Result<String, String> {
    run_config::set_thinking(root, cancel, level)?;
    footer.borrow_mut().state.thinking = level_name(level);
    if persist {
        settings.set_default_thinking_level(level);
    }
    Ok(format!(
        "{} thinking level: {}",
        if persist { "Default " } else { "Thinking" },
        level_name(level)
    ))
}

fn default_model_ref(settings: &SettingsManager) -> Option<(String, String)> {
    match (settings.default_provider(), settings.default_model()) {
        (Some(provider), Some(model)) => Some((provider.to_string(), model.to_string())),
        _ => None,
    }
}

fn settings_config(
    settings: &SettingsManager,
    registry: &ModelRegistry,
    model: &Model,
    thinking: ModelThinkingLevel,
    themes: &[String],
) -> SettingsConfig {
    SettingsConfig {
        hide_thinking_block: settings.hide_thinking_block(),
        output_pad: settings.output_pad(),
        editor_padding_x: settings.editor_padding_x() as usize,
        autocomplete_max_visible: settings.autocomplete_max_visible() as usize,
        show_hardware_cursor: settings.show_hardware_cursor(),
        clear_on_shrink: settings.clear_on_shrink(),
        default_project_trust: settings.default_project_trust().to_string(),
        thinking_level: settings
            .default_thinking_level()
            .and_then(parse_level)
            .unwrap_or(thinking),
        model_thinking_levels: settings
            .all_model_thinking_levels()
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|level| (key, level.to_string())))
            .collect::<BTreeMap<String, String>>(),
        available_models: run_config::available(registry),
        current_model: Some((model.provider.clone(), model.id.clone())),
        default_model: default_model_ref(settings),
        theme_setting: settings
            .theme_setting()
            .unwrap_or(theme::SYSTEM_THEME)
            .to_owned(),
        themes: themes.to_vec(),
        quiet_startup: match settings.quiet_startup() {
            crate::settings::QuietStartup::On => "true",
            crate::settings::QuietStartup::Header => "header",
            crate::settings::QuietStartup::Off => "false",
        }
        .to_owned(),
        tree_filter_mode: settings.tree_filter_mode().to_owned(),
    }
}

fn overrides_reader(
    settings: Rc<RefCell<SettingsManager>>,
) -> crate::interactive::selectors::settings::OverridesReader {
    Rc::new(move || {
        settings
            .borrow()
            .all_model_thinking_levels()
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|level| (key, level.to_string())))
            .collect()
    })
}

fn settings_callbacks(settings: Rc<RefCell<SettingsManager>>) -> Rc<RefCell<SettingsCallbacks>> {
    let for_set = settings.clone();
    let for_remove = settings;
    Rc::new(RefCell::new(SettingsCallbacks {
        model_thinking_level: Box::new(move |provider: &str, id: &str, level| {
            for_set
                .borrow_mut()
                .set_model_thinking_level(provider, id, level);
        }),
        model_thinking_level_remove: Box::new(move |provider: &str, id: &str| {
            for_remove
                .borrow_mut()
                .remove_model_thinking_level(provider, id);
        }),
    }))
}

/// The text of a queued input, as `restoreQueuedMessagesToEditor` puts it back.
fn input_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserBlock::Text(text) => Some(text.text.clone()),
                UserBlock::Image(_) => None,
            })
            .collect::<Vec<String>>()
            .join("\n"),
    }
}

/// `getSteeringMessages` and `getFollowUpMessages`: what the inbox holds, in order.
fn queued(conversation: &Conversation, cancel: &CancelToken) -> (Vec<String>, Vec<String>) {
    use pi_durable::harness::inbox::{InboxItem, InboxState, inbox_doc};
    let id = conversation.id();
    let state = conversation.commit(
        |tx| {
            let token = inbox_doc()?;
            pi_durable::harness::util::read_draft::<InboxState>(tx, &token, id)
        },
        cancel,
    );
    let mut steering = Vec::new();
    let mut follow_up = Vec::new();
    if let Ok(state) = state {
        for item in &state.items {
            match item {
                InboxItem::Steer { content, .. } => steering.push(input_text(content)),
                InboxItem::FollowUp { content, .. } => follow_up.push(input_text(content)),
                InboxItem::Write { .. } => {}
            }
        }
    }
    (steering, follow_up)
}

/// `clearQueue`: the queued messages, taken out of the inbox so they can go back in the editor.
fn take_queued(
    conversation: &Conversation,
    cancel: &CancelToken,
) -> Result<(Vec<String>, Vec<String>), String> {
    use pi_durable::harness::inbox::{InboxItem, InboxState, inbox_doc, withdraw_queued_inputs};
    let id = conversation.id();
    conversation
        .commit(
            |tx| {
                let token = inbox_doc()?;
                let state = pi_durable::harness::util::read_draft::<InboxState>(tx, &token, id)?;
                let mut steering = Vec::new();
                let mut follow_up = Vec::new();
                for item in &state.items {
                    match item {
                        InboxItem::Steer { content, .. } => steering.push(input_text(content)),
                        InboxItem::FollowUp { content, .. } => follow_up.push(input_text(content)),
                        InboxItem::Write { .. } => {}
                    }
                }
                withdraw_queued_inputs(tx, id)?;
                Ok((steering, follow_up))
            },
            cancel,
        )
        .map_err(|e| e.to_string())
}

fn text_input(text: &str, images: &[UserBlock]) -> UserContent {
    let mut blocks = vec![UserBlock::Text(TextContent {
        kind: TextType,
        text: text.to_string(),
        text_signature: None,
    })];
    blocks.extend(images.iter().cloned());
    UserContent::Blocks(blocks)
}

/// What the extensions made of the calls they made while an event was served: the same actions
/// the keys beside them take.
struct Calls<'a> {
    root: &'a Conversation,
    chat: &'a mut Chat,
    pending: &'a mut Vec<(UserContent, &'static str)>,
    quit: &'a mut bool,
    cwd: &'a str,
    interrupted: &'a mut bool,
    cancel: &'a CancelToken,
}

/// Serves what the extensions asked for; true when one of the calls put a line in the chat.
fn serve_extension_calls(live: &crate::extensions::Live, calls: Calls<'_>) -> bool {
    let Calls {
        root,
        chat,
        pending,
        quit,
        cwd,
        interrupted,
        cancel,
    } = calls;
    live.set_state(cwd, chat.working, pending.len() as u64);
    let mut drew = false;
    for call in live.take_calls() {
        match call {
            Call::Abort => {
                *interrupted = true;
                if let Err(e) = root.abort(false, cancel) {
                    chat.notice("error", &format!("Error: {e}"));
                    drew = true;
                }
            }
            Call::Notify { text, level } => {
                let token = match level.as_str() {
                    "error" => "error",
                    "warning" => "warning",
                    _ => "dim",
                };
                chat.notice(token, &text);
                drew = true;
            }
            Call::SendUserMessage { content, .. } => {
                pending.push((crate::extensions::content_of(&content), "extension"));
            }
            Call::Shutdown => *quit = true,
            Call::Unsupported { member } => {
                chat.notice("dim", &format!("{member} is not available in pi-rs yet"));
                drew = true;
            }
        }
    }
    live.settle(*interrupted);
    *interrupted = *interrupted && chat.working;
    drew
}

fn submit(
    root: &Conversation,
    model: &Model,
    content: UserContent,
    source: &str,
    busy: bool,
    cancel: &CancelToken,
    live: Option<&crate::extensions::Live>,
) -> Result<(), String> {
    let content = match (live, crate::extensions::text_of(&content)) {
        (Some(live), Some(text)) => match live.input(&text, source) {
            None => return Ok(()),
            Some(replaced) if replaced != text => crate::extensions::with_text(content, replaced),
            Some(_) => content,
        },
        _ => content,
    };
    if run_config::is_unknown_model(model) {
        return Err(crate::list_models::no_api_key_found(&model.provider));
    }
    root.submit(
        SubmissionDraft::Input {
            request_id: None,
            content,
            when_busy: if busy {
                WhenBusy::Steer
            } else {
                WhenBusy::FollowUp
            },
        },
        cancel,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// `updatePendingMessagesDisplay`: redraw the queued lines; true when they changed.
fn refresh_pending(
    pending: &Rc<RefCell<PendingMessages>>,
    root: &Conversation,
    cancel: &CancelToken,
) -> bool {
    let (steering, follow_up) = queued(root, cancel);
    pending.borrow_mut().set(&steering, &follow_up)
}

/// `JSON.stringify` of a string, as upstream quotes a name it warns about.
fn quoted(text: &str) -> String {
    pi_durable::types::JsonValue::String(text.to_owned()).to_string()
}

/// `/session`: the file, the id, message counts over the stored conversation, and the totals the
/// footer shows.
fn session_info(
    root: &Conversation,
    cancel: &CancelToken,
    name: Option<String>,
    path: Option<&str>,
    totals: Totals,
    context_tokens: Option<u64>,
) -> Result<String, String> {
    let messages = crate::print::transcript(root, cancel).map_err(|e| e.to_string())?;
    let mut counts = Counts::default();
    for message in &messages {
        counts.total += 1;
        match message {
            Message::User(_) => counts.user += 1,
            Message::Assistant(assistant) => {
                counts.assistant += 1;
                counts.tool_calls += assistant
                    .content
                    .iter()
                    .filter(|block| matches!(block, AssistantBlock::ToolCall(_)))
                    .count();
            }
            Message::ToolResult(_) => counts.tool_results += 1,
            Message::System(_) => {}
        }
    }
    Ok(slash_commands::session(&SessionInfo {
        name,
        file: path.map(str::to_owned),
        id: path
            .and_then(|path| path.rsplit('/').next())
            .and_then(|name| name.strip_suffix(".db").map(str::to_owned))
            .unwrap_or_else(|| root.id().0.to_string()),
        counts,
        totals,
        context_tokens,
    }))
}

/// Take the model and thinking level a session recorded for itself, so switching to it does not
/// rewrite what it stored. False when it recorded none, and the caller configures it instead.
fn adopt_session_agent(
    root: &Conversation,
    cancel: &CancelToken,
    registry: &ModelRegistry,
    footer: &Rc<RefCell<Footer>>,
    model: &mut Model,
    thinking: &mut ModelThinkingLevel,
) -> Result<bool, String> {
    let Some((found, level)) = run_config::recorded_agent(root, cancel, registry)? else {
        return Ok(false);
    };
    *thinking = level;
    *model = found.clone();
    {
        let mut footer = footer.borrow_mut();
        footer.state.model_id = Some(found.id.clone());
        footer.state.provider = Some(found.provider.clone());
        footer.state.context_window = found.context_window;
        footer.state.reasoning = found.reasoning;
        footer.state.thinking = level_name(*thinking);
    }
    Ok(true)
}

/// Upstream's `switchSession` and `newSession` (`core/agent-session-runtime.ts`): settle the run
/// in flight into the session it belongs to, close it, open the target, and redraw the chat from
/// the target's transcript.
fn replace_session(
    open: &mut Open,
    opener: &Opener,
    chat: &mut Chat,
    target: Option<&str>,
    cancel: &CancelToken,
) -> Result<(), String> {
    open.swap(
        &opener.source,
        target,
        opener.report(),
        opener.sink(),
        cancel,
    )?;
    chat.clear();
    chat.load(&open.stream.snapshot);
    Ok(())
}

/// Upstream's `fork` (`core/agent-session-runtime.ts`): write the session's active path up to
/// `point` as a new file and switch the loop to it, as `/resume` does. Hands back the path and
/// the text of the entry the fork stopped before, which upstream re-opens in the editor.
fn branch_and_switch(
    open: &mut Open,
    opener: &Opener,
    chat: &mut Chat,
    point: Option<EntryId>,
    cancel: &CancelToken,
) -> Result<String, String> {
    let tree = Tree::build(&open.root, cancel).map_err(|e| e.to_string())?;
    let path = crate::session_fork::branch_from(open.path.as_deref(), &tree, point)?;
    replace_session(open, opener, chat, Some(&path), cancel)?;
    Ok(path)
}

/// `/fork`: the branch stops before the chosen user message, whose text the editor re-opens.
fn fork_session(
    open: &mut Open,
    opener: &Opener,
    chat: &mut Chat,
    target: EntryId,
    cancel: &CancelToken,
) -> Result<(String, Option<String>), String> {
    let tree = Tree::build(&open.root, cancel).map_err(|e| e.to_string())?;
    let point = session_tree::fork_point(&tree, target);
    let text = tree.entry(target).and_then(session_tree::user_message_text);
    let path = branch_and_switch(open, opener, chat, point, cancel)?;
    Ok((path, text))
}

/// Point a session the loop switched to at its own recorded model and thinking level, or at the
/// loop's when it recorded none, as upstream's `switchSession` does.
fn adopt_or_apply(
    root: &Conversation,
    cancel: &CancelToken,
    registry: &ModelRegistry,
    footer: &Rc<RefCell<Footer>>,
    model: &mut Model,
    thinking: &mut ModelThinkingLevel,
    cwd: &str,
) -> Result<(), String> {
    match adopt_session_agent(root, cancel, registry, footer, model, thinking)? {
        true => Ok(()),
        false => run_config::apply_agent(root, cancel, model, *thinking, cwd),
    }
}

/// Upstream's `navigateTree`: move the session's leaf to a chosen entry, redraw the chat from
/// the branch that forks, and hand back the text to re-open in the editor.
fn navigate_session(
    open: &mut Open,
    chat: &mut Chat,
    opener: &Opener,
    target: EntryId,
    cancel: &CancelToken,
) -> Result<(bool, Option<String>), String> {
    let tree = Tree::build(&open.root, cancel).map_err(|e| e.to_string())?;
    if tree.leaf == Some(target) {
        return Ok((false, None));
    }
    let point = session_tree::fork_point(&tree, target);
    let editor_text = tree.entry(target).and_then(session_tree::user_message_text);
    if point == tree.leaf {
        return Ok((true, editor_text));
    }
    let branch = session_tree::navigate(&open.harness, &open.root, point, cancel)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "The session did not branch".to_owned())?;
    open.stream.stop();
    open.root = branch;
    let stream = open.root.events(cancel).map_err(|e| e.to_string())?;
    let feed = opener.sender.clone();
    stream
        .start(move |events, _| {
            let _ = feed.send(Incoming::Events(events.to_vec()));
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    open.stream = stream;
    chat.clear();
    chat.load(&open.stream.snapshot);
    Ok((true, editor_text))
}

/// `MCP servers need attention`, with one line per server, as upstream reports them once at startup.
pub fn mcp_report(problems: &[String]) -> String {
    let lines: Vec<String> = problems
        .iter()
        .map(|problem| format!("  {problem}"))
        .collect();
    format!("MCP servers need attention:\n{}", lines.join("\n"))
}

/// Opens the login dialog for one provider, keeping the handle the loop polls.
fn open_login(
    dialog: &mut Option<Rc<RefCell<LoginDialog>>>,
    agent_dir: &str,
    provider: &str,
    auth: crate::login::AuthType,
    slot: &ActionSlot,
) -> ComponentRef {
    let name = crate::login::display_name(provider);
    let (built, handle) = login_selector::start(agent_dir, provider, &name, auth, slot);
    *dialog = Some(handle);
    built
}

fn login_default(registry: &ModelRegistry, provider: &str, label: &str) -> Result<Model, String> {
    let Some(default_id) = model_resolver::default_model_for(provider) else {
        return Err(format!(
            "{label}, but no default model is configured for provider \"{provider}\". Use /model to select a model."
        ));
    };
    let provider_models: Vec<Model> = registry
        .available()
        .into_iter()
        .filter(|found| found.provider == provider)
        .collect();
    if provider_models.is_empty() {
        return Err(format!(
            "{label}, but no models are available for that provider. Use /model to select a model."
        ));
    }
    provider_models
        .into_iter()
        .find(|found| found.id == default_id)
        .ok_or_else(|| {
            format!(
                "{label}, but its default model \"{default_id}\" is not available. Use /model to select a model."
            )
        })
}

pub fn run(session: Session) -> Result<Option<String>, String> {
    let cancel = CancelToken::new();
    let Session {
        opener,
        session_dir,
        persists,
        model,
        thinking,
        cwd,
        settings,
        registry,
        agent_dir,
        theme_paths,
        use_theme,
        no_themes,
        initial,
        messages,
        provider_count,
        session_path,
        initial_name,
        mcp_problems,
        extensions,
        templates,
        incoming,
    } = session;
    let (mut model, mut thinking, mut cwd) = (model, thinking, cwd);
    keybindings::install(&agent_dir);
    let sources = theme::Sources {
        agent_dir: agent_dir.clone(),
        cwd: cwd.clone(),
        trusted: settings.borrow().is_project_trusted(),
        paths: {
            let mut paths = settings.borrow().theme_paths();
            paths.extend(theme_paths);
            paths
        },
        no_themes,
    };
    let setting = use_theme.or_else(|| settings.borrow().theme_setting().map(str::to_owned));
    if let Some(message) = theme::init(&sources, setting.as_deref()) {
        eprintln!("Warning: {message}");
    }
    let theme_names: Vec<String> = theme::available(&sources)
        .into_iter()
        .map(|info| info.name)
        .collect();
    let mut open = opener.open(session_path.as_deref())?;
    run_config::apply_agent(&open.root, &cancel, &model, thinking, &cwd)?;
    if let Some(name) = &initial_name {
        session_info::set_name(&open.root, &cancel, name).map_err(|e| e.to_string())?;
    }

    let mut chat = Chat::new(&cwd);
    for notice in &extensions.notices {
        chat.notice("warning", notice);
    }
    let mut bash = BashMode::new();
    chat.hide_thinking = settings.borrow().hide_thinking_block();
    chat.output_pad = settings.borrow().output_pad() as usize;
    let mut scope = run_config::scope(&settings.borrow(), &run_config::available(&registry));
    chat.load(&open.stream.snapshot);
    if !mcp_problems.is_empty() {
        chat.notice("warning", &mcp_report(&mcp_problems));
    }
    if run_config::is_unknown_model(&model) {
        chat.notice(
            "warning",
            &format!("Warning: {}", crate::list_models::no_models_available()),
        );
    }

    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut tui = TuiMainScreen::new(ProcessTerminal::new());
    tui.set_show_hardware_cursor(settings.borrow().show_hardware_cursor());
    tui.set_clear_on_shrink(settings.borrow().clear_on_shrink());
    let header =
        header(settings.borrow().quiet_startup()).map(|text| component(Text::new(&text, 1, 1)));
    let status = component(StatusArea::new());
    let pending_area = component(PendingMessages::new());
    let editor = component(Editor::new(
        theme::editor(),
        EditorOptions {
            padding_x: Some(settings.borrow().editor_padding_x() as usize),
            autocomplete_max_visible: Some(settings.borrow().autocomplete_max_visible() as usize),
        },
    ));
    let footer = component(Footer {
        state: FooterState {
            cwd: cwd.clone(),
            branch: git_branch(&cwd),
            context_window: model.context_window,
            model_id: Some(model.id.clone()),
            provider: Some(model.provider.clone()),
            reasoning: model.reasoning,
            thinking: level_name(thinking),
            provider_count,
            auto_compact: true,
            ..FooterState::default()
        },
    });
    let submitted: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let bash_border = Rc::new(Cell::new(false));
    let dirty = Rc::new(Cell::new(true));
    {
        let mut e = editor.borrow_mut();
        let (s, d) = (submitted.clone(), dirty.clone());
        e.on_submit = Some(Box::new(move |t: &str| s.borrow_mut().push(t.to_string())));
        e.request_render = Some(Rc::new(move || d.set(true)));
        e.set_autocomplete_provider(Rc::new(CombinedAutocompleteProvider::new(
            slash_commands::with_templates(
                slash_commands::autocomplete_commands(&extensions.commands),
                &templates,
            ),
            &cwd,
            crate::config::tool_path("fd").as_deref(),
        )));
    }
    let actions: Rc<RefCell<Vec<Action>>> = Rc::new(RefCell::new(Vec::new()));
    let selector_open = Rc::new(Cell::new(false));
    let (acts, ed, sel_open) = (actions.clone(), editor.clone(), selector_open.clone());
    tui.add_input_listener(Box::new(move |data: &str| {
        if is_key_release(data) {
            return None;
        }
        if sel_open.get() {
            return None;
        }
        let kb = pi_tui::keybindings::get_keybindings();
        let action = if kb.matches(data, "app.interrupt") && !ed.borrow().is_showing_autocomplete()
        {
            Action::Interrupt
        } else if kb.matches(data, "app.clear") {
            Action::Clear
        } else if kb.matches(data, "app.exit") && ed.borrow().get_text().is_empty() {
            Action::Exit
        } else if kb.matches(data, "app.tools.expand") {
            Action::Expand
        } else if kb.matches(data, "app.message.followUp") {
            Action::FollowUp
        } else if kb.matches(data, "app.message.dequeue") {
            Action::Dequeue
        } else if kb.matches(data, "app.thinking.toggle") {
            Action::ThinkingToggle
        } else if kb.matches(data, "app.thinking.cycle") {
            Action::ThinkingCycle
        } else if kb.matches(data, "app.model.cycleForward") {
            Action::ModelCycle(true)
        } else if kb.matches(data, "app.model.cycleBackward") {
            Action::ModelCycle(false)
        } else if kb.matches(data, "app.model.select") {
            Action::ModelSelect
        } else if kb.matches(data, "app.editor.external") {
            Action::ExternalEditor
        } else if kb.matches(data, "app.suspend") {
            Action::Suspend
        } else if kb.matches(data, "app.session.new") {
            Action::Session("/new")
        } else if kb.matches(data, "app.session.tree") {
            Action::Session("/tree")
        } else if kb.matches(data, "app.session.fork") {
            Action::Session("/fork")
        } else if kb.matches(data, "app.session.resume") {
            Action::Session("/resume")
        } else {
            return None;
        };
        acts.borrow_mut().push(action);
        Some(InputListenerResult {
            consume: true,
            data: None,
        })
    }));
    let parts = Parts {
        header: header.as_ref().map(as_ref),
        chat: chat.container.clone(),
        pending: as_ref(&pending_area),
        status: as_ref(&status),
        editor: as_ref(&editor),
        footer: as_ref(&footer),
    };
    let mut selector: Option<(ComponentRef, ActionSlot)> = None;
    let mut login_dialog: Option<Rc<RefCell<LoginDialog>>> = None;
    // The `/import` a confirmation dialog is asking about.
    let mut import_request: Option<String> = None;
    parts.mount(&mut tui, None);
    tui.start();

    let mut pending: Vec<(UserContent, &'static str)> = Vec::new();
    if let Some((text, images)) = &initial {
        pending.push((text_input(text, images), "interactive"));
    }
    pending.extend(messages.iter().map(|m| (text_input(m, &[]), "interactive")));
    let (catalogs, refreshed) = std::sync::mpsc::channel();
    if registry.network {
        let registry = Arc::clone(&registry);
        std::thread::spawn(move || {
            let cancel = CancelToken::new();
            let timer = cancel.clone();
            std::thread::spawn(move || {
                timer.wait_timeout(Duration::from_secs(15));
                timer.cancel();
            });
            registry.refresh(true, false, None, &cancel);
            let _ = catalogs.send(());
        });
    }
    let live = crate::extensions::live();
    let mut handed: Option<(String, String)> = None;
    if let Some(live) = live {
        live.hand_models(&registry, &model);
        handed = Some((model.provider.clone(), model.id.clone()));
        live.set_session(
            &open.harness,
            &open.root,
            open.path.as_deref(),
            &cwd,
            &cancel,
        );
        live.event("session_start", json!({ "reason": "startup" }));
    }
    let mut interrupted = false;
    let mut last_clear: Option<Instant> = None;
    let mut quit = false;
    let mut next_timer = Instant::now();
    while !quit {
        // Queued command-line messages go one per idle turn, as print mode sends them.
        if !chat.working && !pending.is_empty() {
            let (content, source) = pending.remove(0);
            match submit(&open.root, &model, content, source, false, &cancel, live) {
                Ok(()) => chat.working = true,
                Err(e) => chat.notice("error", &format!("Error: {e}")),
            }
            dirty.set(true);
        }
        if refreshed.try_recv().is_ok() {
            if let Some(live) = live {
                live.hand_models(&registry, &model);
            }
            footer.borrow_mut().state.provider_count = registry.provider_count();
            dirty.set(true);
        }
        if let Some(live) = live {
            let reference = (model.provider.clone(), model.id.clone());
            if handed.as_ref() != Some(&reference) {
                live.hand_models(&registry, &model);
                handed = Some(reference);
            }
        }
        let mut due = tui.tick(now());
        let (spun, spin_due) = status.borrow_mut().tick(now());
        if spun {
            dirty.set(true);
        }
        let (moved, bash_due) = bash.tick(now());
        if moved {
            dirty.set(true);
        }
        due = match (due, spin_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        due = [due, bash_due].into_iter().flatten().min();
        let timeout = due.map_or(50, |d| d.saturating_sub(now()).min(50));
        let events = tui.terminal.poll(Some(timeout));
        tui.handle_events(events);

        if let Some(dialog) = &login_dialog
            && dialog.borrow_mut().poll()
        {
            dirty.set(true);
        }
        while let Ok(message) = incoming.try_recv() {
            match message {
                Incoming::Events(events) => {
                    let mut queue_changed = false;
                    let mut entries_changed = false;
                    for event in &events {
                        if let Some(live) = live {
                            live.observe(event);
                        }
                        if matches!(event, AgentEvent::InboxUpdate { .. }) {
                            queue_changed = true;
                        }
                        if matches!(
                            event,
                            AgentEvent::Snapshot(_)
                                | AgentEvent::EntryAppended { .. }
                                | AgentEvent::MessageEnd { .. }
                                | AgentEvent::ToolExecutionEnd { entry: Some(_), .. }
                        ) {
                            entries_changed = true;
                        }
                        if chat.apply(event) {
                            dirty.set(true);
                        }
                    }
                    if entries_changed && let Some(live) = live {
                        live.set_session(
                            &open.harness,
                            &open.root,
                            open.path.as_deref(),
                            &cwd,
                            &cancel,
                        );
                    }
                    if queue_changed && refresh_pending(&pending_area, &open.root, &cancel) {
                        dirty.set(true);
                    }
                }
                Incoming::Report(text) => {
                    chat.notice("error", &format!("Error: {text}"));
                    dirty.set(true);
                }
            }
        }
        if let Some(live) = live
            && serve_extension_calls(
                live,
                Calls {
                    root: &open.root,
                    chat: &mut chat,
                    pending: &mut pending,
                    quit: &mut quit,
                    cwd: &cwd,
                    interrupted: &mut interrupted,
                    cancel: &cancel,
                },
            )
        {
            dirty.set(true);
        }
        for action in actions.borrow_mut().drain(..) {
            match action {
                Action::Interrupt => {
                    if !chat.working && bash.cancel() {
                        // `abortBash`: the block ends as cancelled.
                    } else if !chat.working
                        && editor.borrow().get_text().trim_start().starts_with('!')
                    {
                        editor.borrow_mut().set_text("");
                    } else if chat.working {
                        // `restoreQueuedMessagesToEditor({abort: true})`: the queued lines go
                        // back in the editor before the run is aborted.
                        match take_queued(&open.root, &cancel) {
                            Ok((steering, follow_up)) => {
                                let queued: Vec<String> =
                                    steering.into_iter().chain(follow_up).collect();
                                if !queued.is_empty() {
                                    let current = editor.borrow().get_text();
                                    let combined = [queued.join("\n\n"), current]
                                        .into_iter()
                                        .filter(|text| !text.trim().is_empty())
                                        .collect::<Vec<String>>()
                                        .join("\n\n");
                                    editor.borrow_mut().set_text(&combined);
                                }
                            }
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        }
                        refresh_pending(&pending_area, &open.root, &cancel);
                        if let Err(e) = open.root.abort(false, &cancel) {
                            chat.notice("error", &format!("Error: {e}"));
                        }
                    }
                }
                Action::FollowUp => {
                    let text = editor.borrow().get_text().trim().to_owned();
                    if !text.is_empty() && chat.working {
                        editor.borrow_mut().add_to_history(&text);
                        editor.borrow_mut().set_text("");
                        let expanded = crate::prompt_templates::expand(&text, &templates);
                        if let Err(e) = submit(
                            &open.root,
                            &model,
                            text_input(&expanded, &[]),
                            "interactive",
                            false,
                            &cancel,
                            live,
                        ) {
                            chat.notice("error", &format!("Error: {e}"));
                        }
                        refresh_pending(&pending_area, &open.root, &cancel);
                    } else if !text.is_empty() {
                        // Idle, it acts like Enter: the line is submitted as a prompt.
                        editor.borrow_mut().set_text("");
                        submitted.borrow_mut().push(text);
                    }
                }
                Action::Dequeue => match take_queued(&open.root, &cancel) {
                    Ok((steering, follow_up)) => {
                        let queued: Vec<String> = steering.into_iter().chain(follow_up).collect();
                        if queued.is_empty() {
                            chat.notice("dim", "No queued messages to restore");
                        } else {
                            let restored = queued.len();
                            let current = editor.borrow().get_text();
                            let combined = [queued.join("\n\n"), current]
                                .into_iter()
                                .filter(|text| !text.trim().is_empty())
                                .collect::<Vec<String>>()
                                .join("\n\n");
                            editor.borrow_mut().set_text(&combined);
                            chat.notice(
                                "dim",
                                &format!(
                                    "Restored {restored} queued message{} to editor",
                                    if restored > 1 { "s" } else { "" }
                                ),
                            );
                        }
                        refresh_pending(&pending_area, &open.root, &cancel);
                    }
                    Err(e) => chat.notice("error", &format!("Error: {e}")),
                },
                Action::Clear => {
                    if last_clear.is_some_and(|t| t.elapsed().as_millis() < 500) {
                        quit = true;
                    } else {
                        editor.borrow_mut().set_text("");
                        last_clear = Some(Instant::now());
                    }
                }
                Action::Exit => quit = true,
                Action::Expand => {
                    let expanded = !chat.expanded;
                    chat.set_expanded(expanded);
                }
                Action::ModelCycle(forward) => {
                    let available = run_config::available(&registry);
                    let scoped = !scope.is_empty();
                    let models: Vec<Model> = if scoped {
                        scope
                            .iter()
                            .filter(|scoped_model| {
                                available.iter().any(|model| {
                                    model.provider == scoped_model.provider
                                        && model.id == scoped_model.id
                                })
                            })
                            .cloned()
                            .collect()
                    } else {
                        available
                    };
                    if models.len() <= 1 {
                        chat.show_status(if scoped {
                            "Only one model in scope"
                        } else {
                            "Only one model available"
                        });
                    } else {
                        let count = models.len();
                        let current = models
                            .iter()
                            .position(|m| m.provider == model.provider && m.id == model.id)
                            .unwrap_or(0);
                        let index = if forward {
                            (current + 1) % count
                        } else {
                            (current + count - 1) % count
                        };
                        let next = models[index].clone();
                        match apply_model(
                            &open.root,
                            &cancel,
                            &mut settings.borrow_mut(),
                            &footer,
                            &next,
                            false,
                            &mut thinking,
                        ) {
                            Ok(_) => {
                                let name = if next.name.is_empty() {
                                    next.id.clone()
                                } else {
                                    next.name.clone()
                                };
                                let thinking_text =
                                    if next.reasoning && thinking != ModelThinkingLevel::Off {
                                        format!(" (thinking: {})", level_name(thinking))
                                    } else {
                                        String::new()
                                    };
                                model = next;
                                chat.show_status(&format!("Switched to {name}{thinking_text}"));
                            }
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        }
                    }
                }
                Action::ModelSelect => submitted.borrow_mut().push("/model".to_owned()),
                Action::ExternalEditor => {
                    let command = settings.borrow().external_editor_command();
                    let content = editor.borrow().get_text();
                    tui.stop(false);
                    println!(
                        "Launching external editor: {command}\nPi will resume when the editor exits."
                    );
                    if let Some(text) = external_editor::edit(&command, &content) {
                        editor.borrow_mut().set_text(&text);
                    }
                    tui.start();
                    tui.request_render(true);
                }
                Action::Session(command) => submitted.borrow_mut().push(command.to_owned()),
                Action::Suspend => {
                    tui.stop(false);
                    pi_tui::terminal::suspend();
                    tui.start();
                    tui.request_render(true);
                }
                Action::ThinkingCycle => {
                    let levels = supported_thinking_levels(&model);
                    if !model.reasoning {
                        chat.show_status("Current model does not support thinking");
                    } else {
                        let next = match levels.iter().position(|level| *level == thinking) {
                            Some(index) => levels[(index + 1) % levels.len()],
                            None => levels[0],
                        };
                        match apply_thinking(
                            &open.root,
                            &cancel,
                            &mut settings.borrow_mut(),
                            &footer,
                            next,
                            false,
                        ) {
                            Ok(_) => {
                                thinking = next;
                                chat.show_status(&format!("Thinking level: {}", level_name(next)));
                            }
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        }
                    }
                }
                Action::ThinkingToggle => {
                    let hidden = !chat.hide_thinking;
                    chat.set_hide_thinking(hidden);
                    settings.borrow_mut().set_hide_thinking_block(hidden);
                    chat.show_status(&format!(
                        "Thinking blocks: {}",
                        if hidden { "hidden" } else { "visible" }
                    ));
                }
            }
            dirty.set(true);
        }
        let pending = selector
            .as_ref()
            .and_then(|(_, slot)| slot.borrow_mut().take());
        if let Some(action) = pending {
            let mut close = false;
            let mut next: Option<(ComponentRef, ActionSlot)> = None;
            match action {
                OverlayAction::Close => close = true,
                OverlayAction::SetLoginMethod { auth, provider } => {
                    let slot = action_slot();
                    let built: ComponentRef = match provider {
                        Some(provider) => {
                            open_login(&mut login_dialog, &agent_dir, &provider, auth, &slot)
                        }
                        None => component(AuthSelector::new(
                            Some(auth),
                            &agent_dir,
                            None,
                            slot.clone(),
                        )),
                    };
                    next = Some((built, slot));
                }
                OverlayAction::LoginProvider { provider, auth } => {
                    let slot = action_slot();
                    let built = open_login(&mut login_dialog, &agent_dir, &provider, auth, &slot);
                    next = Some((built, slot));
                }
                OverlayAction::LogoutProvider {
                    provider,
                    name,
                    auth,
                } => {
                    close = true;
                    match crate::login::logout(&agent_dir, &provider) {
                        Ok(()) => {
                            registry.reload_auth();
                            let count = registry.provider_count();
                            footer.borrow_mut().state.provider_count = count;
                            match auth {
                                crate::login::AuthType::Oauth => {
                                    chat.notice("accent", &format!("Logged out of {name}"))
                                }
                                crate::login::AuthType::ApiKey => chat.notice(
                                    "accent",
                                    &format!(
                                        "Removed stored API key for {name}. Environment variables and models.json config are unchanged."
                                    ),
                                ),
                            }
                        }
                        Err(e) => chat.notice("error", &format!("Logout failed: {e}")),
                    }
                }
                OverlayAction::LoggedIn(outcome) => {
                    close = true;
                    registry.reload_auth();
                    let count = registry.provider_count();
                    footer.borrow_mut().state.provider_count = count;
                    let action = match outcome.auth {
                        crate::login::AuthType::Oauth => "Logged in to",
                        crate::login::AuthType::ApiKey => "Saved API key for",
                    };
                    let label = format!("{action} {}", outcome.name);
                    let picked = run_config::is_unknown_model(&model).then(|| {
                        login_default(&registry, &outcome.provider, &label).and_then(|found| {
                            apply_model(
                                &open.root,
                                &cancel,
                                &mut settings.borrow_mut(),
                                &footer,
                                &found,
                                true,
                                &mut thinking,
                            )
                            .map(|_| found)
                            .map_err(|e| {
                                format!(
                                    "{label}, but selecting its default model failed: {e}. Use /model to select a model."
                                )
                            })
                        })
                    });
                    match picked {
                        Some(Ok(found)) => {
                            chat.notice(
                                "accent",
                                &format!(
                                    "{label}. Selected {}. Credentials saved to {}",
                                    found.id, outcome.path
                                ),
                            );
                            model = found;
                        }
                        picked => {
                            chat.notice(
                                "accent",
                                &format!("{label}. Credentials saved to {}", outcome.path),
                            );
                            if let Some(Err(error)) = picked {
                                chat.notice("error", &format!("Error: {error}"));
                            }
                        }
                    }
                }
                OverlayAction::LoginFailed {
                    name,
                    auth,
                    message,
                } => {
                    close = true;
                    if message == "Login cancelled" {
                        chat.notice("dim", "Login cancelled");
                    } else {
                        let action = match auth {
                            crate::login::AuthType::Oauth => "Failed to login to",
                            crate::login::AuthType::ApiKey => "Failed to save API key for",
                        };
                        chat.notice("error", &format!("{action} {name}: {message}"));
                    }
                }
                OverlayAction::Confirmed(confirmed) => {
                    close = true;
                    if let Some(argument) = import_request.take() {
                        if !confirmed {
                            chat.notice("accent", "Import cancelled");
                        } else {
                            match crate::sessions::import::run(
                                &argument,
                                &session_dir,
                                &cwd,
                                &cancel,
                            ) {
                                Err(e) => {
                                    chat.notice("error", &format!("Failed to import session: {e}"))
                                }
                                Ok(imported) => {
                                    let opener = Opener {
                                        source: Source {
                                            tool_settings: ToolSettings {
                                                cwd: imported.cwd.clone(),
                                                ..opener.source.tool_settings.clone()
                                            },
                                            ..opener.source.clone()
                                        },
                                        ..opener.clone()
                                    };
                                    match replace_session(
                                        &mut open,
                                        &opener,
                                        &mut chat,
                                        Some(&imported.path),
                                        &cancel,
                                    ) {
                                        Err(e) => chat.notice(
                                            "error",
                                            &format!("Failed to import session: {e}"),
                                        ),
                                        Ok(()) => {
                                            cwd = imported.cwd.clone();
                                            {
                                                let mut footer = footer.borrow_mut();
                                                footer.state.cwd = cwd.clone();
                                                footer.state.branch = git_branch(&cwd);
                                            }
                                            chat.cwd = cwd.clone();
                                            editor.borrow_mut().set_autocomplete_provider(Rc::new(
                                                CombinedAutocompleteProvider::new(
                                                    slash_commands::autocomplete_commands(
                                                        &extensions.commands,
                                                    ),
                                                    &cwd,
                                                    crate::config::tool_path("fd").as_deref(),
                                                ),
                                            ));
                                            match adopt_or_apply(
                                                &open.root,
                                                &cancel,
                                                &registry,
                                                &footer,
                                                &mut model,
                                                &mut thinking,
                                                &cwd,
                                            ) {
                                                Ok(()) => chat.notice(
                                                    "accent",
                                                    &format!("Session imported from: {argument}"),
                                                ),
                                                Err(e) => chat.notice(
                                                    "error",
                                                    &format!("Failed to import session: {e}"),
                                                ),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                OverlayAction::SetModel {
                    provider,
                    id,
                    persist,
                } => {
                    close = true;
                    match registry.get_model(&provider, &id) {
                        Some(found) => match apply_model(
                            &open.root,
                            &cancel,
                            &mut settings.borrow_mut(),
                            &footer,
                            &found,
                            persist,
                            &mut thinking,
                        ) {
                            Ok(message) => {
                                model = found;
                                chat.notice("accent", &message)
                            }
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        },
                        None => chat
                            .notice("error", &format!("Error: {provider}/{id} is not available")),
                    }
                }
                OverlayAction::SetThinking { level, persist } => {
                    close = true;
                    match apply_thinking(
                        &open.root,
                        &cancel,
                        &mut settings.borrow_mut(),
                        &footer,
                        level,
                        persist,
                    ) {
                        Ok(message) => chat.notice("accent", &message),
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    }
                }
                OverlayAction::SetEnabledModels { ids, persist } => match ids {
                    Some(ids) => {
                        let available = run_config::available(&registry);
                        scope = model_resolver::resolve_model_scope(&ids, &available)
                            .0
                            .into_iter()
                            .map(|scoped| scoped.model)
                            .collect();
                        if persist {
                            settings.borrow_mut().set_enabled_models(Some(&ids));
                            chat.notice("accent", "Model selection saved to settings");
                        }
                    }
                    None => {
                        scope.clear();
                        if persist {
                            settings.borrow_mut().set_enabled_models(None);
                            chat.notice("accent", "Model selection saved to settings");
                        }
                    }
                },
                OverlayAction::SetTrust { updates, trusted } => {
                    close = true;
                    let writes: Vec<(String, Option<bool>)> = updates
                        .iter()
                        .map(|update| (update.path.clone(), update.decision))
                        .collect();
                    match settings_mod::write_trust(&agent_dir, &writes) {
                        Ok(()) => chat.notice(
                            "accent",
                            &format!(
                                "Saved trust decision: {}. Restart pi-rs for this to take effect.",
                                if trusted { "trusted" } else { "untrusted" }
                            ),
                        ),
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    }
                }
                OverlayAction::Resume { path } => {
                    close = true;
                    match replace_session(&mut open, &opener, &mut chat, Some(&path), &cancel) {
                        Ok(()) => {
                            let adopted = adopt_session_agent(
                                &open.root,
                                &cancel,
                                &registry,
                                &footer,
                                &mut model,
                                &mut thinking,
                            );
                            let configured = match adopted {
                                Ok(true) => Ok(()),
                                Ok(false) => run_config::apply_agent(
                                    &open.root, &cancel, &model, thinking, &cwd,
                                ),
                                Err(e) => Err(e),
                            };
                            match configured {
                                Ok(()) => chat.notice("dim", "Resumed session"),
                                Err(e) => chat.notice("error", &format!("Error: {e}")),
                            }
                        }
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    }
                }
                OverlayAction::Navigate(target) => {
                    close = true;
                    if chat.working {
                        chat.notice(
                            "error",
                            "Wait for the current response to finish before navigating the session tree.",
                        );
                    } else {
                        match navigate_session(&mut open, &mut chat, &opener, target, &cancel) {
                            Ok((false, _)) => chat.notice("dim", "Already at this point"),
                            Ok((true, text)) => {
                                if let Some(text) = &text
                                    && editor.borrow().get_text().trim().is_empty()
                                {
                                    editor.borrow_mut().set_text(text);
                                }
                                chat.notice("accent", "Navigated to selected point");
                            }
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        }
                    }
                }
                OverlayAction::Fork(target) => {
                    close = true;
                    match fork_session(&mut open, &opener, &mut chat, target, &cancel) {
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                        Ok((path, text)) => {
                            let switched = adopt_or_apply(
                                &open.root,
                                &cancel,
                                &registry,
                                &footer,
                                &mut model,
                                &mut thinking,
                                &cwd,
                            );
                            match switched {
                                Err(e) => chat.notice("error", &format!("Error: {e}")),
                                Ok(()) => {
                                    editor.borrow_mut().set_text(text.as_deref().unwrap_or(""));
                                    chat.notice(
                                        "accent",
                                        &format!("Forked to new session: {path}"),
                                    );
                                }
                            }
                        }
                    }
                }
                OverlayAction::PreviewTheme(setting) => {
                    let _ = theme::set_setting(&setting, &sources);
                }
                OverlayAction::Setting { id, value } => match id.as_str() {
                    "theme" => match theme::set_setting(&value, &sources) {
                        Ok(()) => settings.borrow_mut().set_theme(&value),
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    },
                    "quiet-startup" => {
                        let quiet = match value.as_str() {
                            "true" => crate::settings::QuietStartup::On,
                            "header" => crate::settings::QuietStartup::Header,
                            _ => crate::settings::QuietStartup::Off,
                        };
                        settings.borrow_mut().set_quiet_startup(quiet);
                    }
                    "tree-filter-mode" => {
                        settings.borrow_mut().set_tree_filter_mode(&value);
                    }
                    "hide-thinking" => {
                        let hidden = value == "true";
                        chat.hide_thinking = hidden;
                        settings.borrow_mut().set_hide_thinking_block(hidden);
                    }
                    "default-project-trust" => {
                        settings.borrow_mut().set_default_project_trust(&value);
                    }
                    "show-hardware-cursor" => {
                        let shown = value == "true";
                        tui.set_show_hardware_cursor(shown);
                        settings.borrow_mut().set_show_hardware_cursor(shown);
                    }
                    "clear-on-shrink" => {
                        let cleared = value == "true";
                        tui.set_clear_on_shrink(cleared);
                        settings.borrow_mut().set_clear_on_shrink(cleared);
                    }
                    "editor-padding" => {
                        if let Ok(padding) = value.parse::<usize>() {
                            editor.borrow_mut().set_padding_x(padding);
                            settings.borrow_mut().set_editor_padding_x(padding as f64);
                        }
                    }
                    "output-padding" => {
                        if let Ok(padding) = value.parse::<u8>() {
                            chat.output_pad = padding as usize;
                            settings.borrow_mut().set_output_pad(padding);
                        }
                    }
                    "autocomplete-max-visible" => {
                        if let Ok(max) = value.parse::<usize>() {
                            editor.borrow_mut().set_autocomplete_max_visible(max);
                            settings
                                .borrow_mut()
                                .set_autocomplete_max_visible(max as f64);
                        }
                    }
                    _ => {}
                },
            }
            if let Some((built, slot)) = next {
                selector = Some((built, slot));
                selector_open.set(true);
                parts.mount(&mut tui, selector.as_ref().map(|(built, _)| built));
            } else if close {
                selector = None;
                login_dialog = None;
                selector_open.set(false);
                parts.mount(&mut tui, None);
            }
            dirty.set(true);
        }
        let texts: Vec<String> = submitted.borrow_mut().drain(..).collect();
        for text in texts {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if live.is_some_and(|live| live.run_command(trimmed)) {
                continue;
            }
            if let Some(command) = slash_commands::dispatch(trimmed) {
                match command {
                    Command::Quit => quit = true,
                    Command::New => {
                        let target = persists.then(|| session_list::new_path(&session_dir));
                        match replace_session(
                            &mut open,
                            &opener,
                            &mut chat,
                            target.as_deref(),
                            &cancel,
                        )
                        .and_then(|()| {
                            run_config::apply_agent(&open.root, &cancel, &model, thinking, &cwd)
                        }) {
                            Ok(()) => chat.notice("accent", "✓ New session started"),
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        }
                    }
                    Command::Name(name) => match name {
                        None => match session_info::name(&open.harness, &cancel) {
                            Some(current) => {
                                chat.notice("dim", &format!("Session name: {current}"))
                            }
                            None => chat.notice("warning", "Usage: /name <name>"),
                        },
                        Some(name) => match session_info::set_name(&open.root, &cancel, &name) {
                            Ok(()) => {
                                let stored = session_info::name(&open.harness, &cancel);
                                if let Some(stored) = &stored
                                    && stored != &name
                                {
                                    chat.notice(
                                        "warning",
                                        &format!(
                                            "Session name was normalized from {} to {}",
                                            quoted(&name),
                                            quoted(stored)
                                        ),
                                    );
                                }
                                chat.notice(
                                    "dim",
                                    &format!("Session name set: {}", stored.unwrap_or(name)),
                                );
                            }
                            Err(e) => chat.notice("error", &format!("Error: {e}")),
                        },
                    },
                    Command::Session => match session_info(
                        &open.root,
                        &cancel,
                        session_info::name(&open.harness, &cancel),
                        open.path.as_deref(),
                        chat.totals,
                        chat.context_tokens,
                    ) {
                        Ok(info) => chat.block(&info),
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    },
                    Command::Hotkeys => chat.block(&slash_commands::hotkeys()),
                    Command::Compact(instructions) => {
                        if let Err(e) = open.root.compact(instructions.as_deref(), &cancel) {
                            chat.notice("error", &format!("Error: {e}"));
                        }
                    }
                    Command::Model(term) => {
                        let available = run_config::available(&registry);
                        let exact = term.as_deref().and_then(|term| {
                            model_resolver::find_exact_model_reference_match(term, &available)
                                .cloned()
                        });
                        match exact {
                            Some(found) => match apply_model(
                                &open.root,
                                &cancel,
                                &mut settings.borrow_mut(),
                                &footer,
                                &found,
                                false,
                                &mut thinking,
                            ) {
                                Ok(message) => {
                                    model = found;
                                    chat.notice("accent", &message)
                                }
                                Err(e) => chat.notice("error", &format!("Error: {e}")),
                            },
                            None => {
                                let slot = action_slot();
                                let built: ComponentRef = component(ModelSelector::new(
                                    available,
                                    scope.clone(),
                                    Some(model.clone()),
                                    default_model_ref(&settings.borrow()),
                                    term.as_deref(),
                                    slot.clone(),
                                ));
                                selector = Some((built, slot));
                            }
                        }
                    }
                    Command::Thinking(term) => {
                        let levels = supported_thinking_levels(&model);
                        match term.as_deref().and_then(parse_level) {
                            Some(level) => match apply_thinking(
                                &open.root,
                                &cancel,
                                &mut settings.borrow_mut(),
                                &footer,
                                level,
                                false,
                            ) {
                                Ok(message) => chat.notice("accent", &message),
                                Err(e) => chat.notice("error", &format!("Error: {e}")),
                            },
                            None => {
                                let slot = action_slot();
                                let default = settings
                                    .borrow()
                                    .default_thinking_level()
                                    .and_then(parse_level);
                                let built: ComponentRef = component(ThinkingSelector::new(
                                    thinking,
                                    levels,
                                    default,
                                    slot.clone(),
                                ));
                                selector = Some((built, slot));
                            }
                        }
                    }
                    Command::ScopedModels => {
                        let slot = action_slot();
                        let enabled = settings.borrow().enabled_models();
                        let built: ComponentRef = component(ScopedModelsSelector::new(
                            run_config::available(&registry),
                            enabled,
                            slot.clone(),
                        ));
                        selector = Some((built, slot));
                    }
                    Command::Settings => {
                        let slot = action_slot();
                        let config = settings_config(
                            &settings.borrow(),
                            &registry,
                            &model,
                            thinking,
                            &theme_names,
                        );
                        let callbacks = settings_callbacks(settings.clone());
                        let reader = overrides_reader(settings.clone());
                        let built: ComponentRef = component(SettingsSelector::new(
                            &config,
                            callbacks,
                            reader,
                            slot.clone(),
                        ));
                        selector = Some((built, slot));
                    }
                    Command::Tree => match Tree::build(&open.root, &cancel) {
                        Ok(tree) if tree.is_empty() => chat.notice("dim", "No entries in session"),
                        Ok(tree) => {
                            let slot = action_slot();
                            let visible = (tui.terminal.rows() / 2).max(5);
                            let filter = Filter::from_setting(settings.borrow().tree_filter_mode());
                            let built: ComponentRef =
                                component(TreeSelector::new(tree, visible, slot.clone(), filter));
                            selector = Some((built, slot));
                        }
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    },
                    Command::Fork => match Tree::build(&open.root, &cancel) {
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                        Ok(tree) => {
                            let mut messages: Vec<(pi_durable::ids::EntryId, String)> = tree
                                .nodes
                                .iter()
                                .filter_map(|node| {
                                    session_tree::user_message_text(&node.entry)
                                        .map(|text| (node.entry.id, text))
                                })
                                .filter(|(_, text)| !text.is_empty())
                                .collect();
                            messages.sort_by_key(|(id, _)| *id);
                            if messages.is_empty() {
                                chat.notice("dim", "No messages to fork from");
                            } else {
                                let slot = action_slot();
                                let built: ComponentRef =
                                    component(ForkSelector::new(messages, None, slot.clone()));
                                selector = Some((built, slot));
                            }
                        }
                    },
                    Command::Clone => match Tree::build(&open.root, &cancel) {
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                        Ok(tree) if tree.leaf.is_none() => {
                            chat.notice("dim", "Nothing to clone yet")
                        }
                        Ok(tree) => {
                            let switched = branch_and_switch(
                                &mut open, &opener, &mut chat, tree.leaf, &cancel,
                            )
                            .and_then(|path| {
                                adopt_or_apply(
                                    &open.root,
                                    &cancel,
                                    &registry,
                                    &footer,
                                    &mut model,
                                    &mut thinking,
                                    &cwd,
                                )?;
                                Ok(path)
                            });
                            match switched {
                                Err(e) => chat.notice("error", &format!("Error: {e}")),
                                Ok(path) => {
                                    editor.borrow_mut().set_text("");
                                    chat.notice(
                                        "accent",
                                        &format!("Cloned to new session: {path}"),
                                    );
                                }
                            }
                        }
                    },
                    Command::Trust => {
                        let slot = action_slot();
                        let options = settings_mod::project_trust_options(&cwd, true);
                        let saved = settings_mod::stored_trust_entry(&agent_dir, &cwd)
                            .ok()
                            .flatten();
                        let trusted = settings.borrow().is_project_trusted();
                        let built: ComponentRef = component(TrustSelector::new(
                            &cwd,
                            options,
                            saved,
                            trusted,
                            slot.clone(),
                        ));
                        selector = Some((built, slot));
                    }

                    Command::Resume => {
                        let slot = action_slot();
                        let built: ComponentRef = component(SessionSelector::new(
                            &session_dir,
                            &agent_dir,
                            open.path.as_deref(),
                            slot.clone(),
                        ));
                        selector = Some((built, slot));
                    }
                    Command::Import => {
                        let argument = crate::sessions::export::path_argument(trimmed, "/import");
                        match argument {
                            None => chat.notice("error", "Usage: /import <path.jsonl>"),
                            Some(argument) => {
                                import_request = Some(argument.clone());
                                let slot = action_slot();
                                let built: ComponentRef = component(Confirm::new(
                                    "Import session",
                                    &format!("Replace current session with {argument}?"),
                                    slot.clone(),
                                ));
                                selector = Some((built, slot));
                            }
                        }
                    }
                    Command::Export => {
                        let argument = crate::sessions::export::path_argument(trimmed, "/export");
                        let session_id = crate::sessions::export::session_id(open.path.as_deref());
                        match crate::sessions::export::write(
                            &open.root,
                            &cancel,
                            &session_id,
                            &cwd,
                            argument.as_deref(),
                        ) {
                            Ok(path) => {
                                chat.notice("accent", &format!("Session exported to: {path}"))
                            }
                            Err(e) => {
                                chat.notice("error", &format!("Failed to export session: {e}"))
                            }
                        }
                    }
                    Command::Login(reference) => {
                        let slot = action_slot();
                        let options = reference
                            .as_deref()
                            .map(crate::login::find)
                            .unwrap_or_default();
                        let built: ComponentRef = match reference.as_deref() {
                            None => component(MethodSelector::new(slot.clone())),
                            Some(reference) => match options.as_slice() {
                                [only] => open_login(
                                    &mut login_dialog,
                                    &agent_dir,
                                    only.id,
                                    only.auth,
                                    &slot,
                                ),
                                _ if options.len() > 1
                                    && options.iter().all(|option| option.id == options[0].id) =>
                                {
                                    let auths: Vec<crate::login::AuthType> =
                                        options.iter().map(|option| option.auth).collect();
                                    component(MethodSelector::for_provider(
                                        Some(options[0].id),
                                        auths,
                                        slot.clone(),
                                    ))
                                }
                                _ => component(AuthSelector::new(
                                    None,
                                    &agent_dir,
                                    Some(reference),
                                    slot.clone(),
                                )),
                            },
                        };
                        selector = Some((built, slot));
                    }
                    Command::Logout => {
                        let slot = action_slot();
                        match AuthSelector::logout(&agent_dir, slot.clone()) {
                            Ok(list) if list.is_empty() => chat.notice(
                                "dim",
                                "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.",
                            ),
                            Ok(list) => {
                                let built: ComponentRef = component(list);
                                selector = Some((built, slot));
                            }
                            Err(e) => chat.notice(
                                "error",
                                &format!("Could not read stored credentials: {e}"),
                            ),
                        }
                    }
                    Command::Unavailable(name) => {
                        chat.notice("dim", &format!("/{name} is not available in pi-rs yet"))
                    }
                }
                if selector.is_some() {
                    selector_open.set(true);
                    parts.mount(&mut tui, selector.as_ref().map(|(built, _)| built));
                }
                dirty.set(true);
                continue;
            }
            if let Some((command, exclude)) = BashMode::parse(&text) {
                if bash.is_running() {
                    chat.notice(
                        "warning",
                        "A bash command is already running. Press Esc to cancel it first.",
                    );
                    editor.borrow_mut().set_text(&text);
                } else {
                    editor.borrow_mut().add_to_history(&text);
                    bash.start(&mut chat, command, exclude, &settings.borrow());
                }
                dirty.set(true);
                continue;
            }
            editor.borrow_mut().add_to_history(&text);
            let expanded = crate::prompt_templates::expand(&text, &templates);
            match submit(
                &open.root,
                &model,
                text_input(&expanded, &[]),
                "interactive",
                chat.working,
                &cancel,
                live,
            ) {
                Ok(()) => chat.working = true,
                Err(e) => chat.notice("error", &format!("Error: {e}")),
            }
            if refresh_pending(&pending_area, &open.root, &cancel) {
                dirty.set(true);
            }
            dirty.set(true);
        }
        if next_timer.elapsed().as_millis() >= 1000 {
            next_timer = Instant::now();
            if chat.tick_rows() {
                dirty.set(true);
            }
        }
        status
            .borrow_mut()
            .set(if chat.working || chat.status.is_some() {
                Some(chat.status.as_deref().unwrap_or(WORKING))
            } else {
                None
            });
        {
            let mut f = footer.borrow_mut();
            f.state.totals = chat.totals;
            f.state.context_tokens = chat.context_tokens;
        }
        let (moved, failure) = bash.poll(&mut chat, &open.root, &cancel);
        if let Some(error) = failure {
            chat.notice("error", &format!("Bash command failed: {error}"));
        }
        if moved {
            dirty.set(true);
        }
        let wants_bash_border = editor.borrow().get_text().trim_start().starts_with('!');
        if wants_bash_border != bash_border.get() {
            bash_border.set(wants_bash_border);
            editor.borrow_mut().border_color = if wants_bash_border {
                theme::fg_fn("bashMode")
            } else {
                theme::editor().border_color.clone()
            };
            dirty.set(true);
        }
        editor.borrow_mut().terminal_rows = tui.terminal.rows();
        if dirty.replace(false) {
            tui.request_render(false);
        }
    }
    tui.stop(false);
    tui.terminal.drain_input(1000, 50);
    if chat.working {
        let _ = open.root.abort(false, &cancel);
    }
    if let Some(live) = live {
        live.event("session_shutdown", json!({ "reason": "quit" }));
    }
    open.shutdown(&cancel)?;
    Ok(open.path)
}
