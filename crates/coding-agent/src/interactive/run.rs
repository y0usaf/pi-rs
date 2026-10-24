//! The interactive loop: the editor submits to the durable conversation, its agent events come
//! back over a channel and redraw the chat. Escape aborts the run, Ctrl+C clears the editor (twice
//! quits), Ctrl+D quits on an empty editor, Ctrl+O expands tool output.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pi_durable::ai::thinking::{clamp_thinking_level, supported_thinking_levels};
use pi_durable::ai::types::{
    AssistantBlock, Message, Model, ModelThinkingLevel, TextContent, TextType, UserBlock,
    UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::events::{AgentEvent, AgentEventStream};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::{CompactionOverrides, ModelRef};
use pi_durable::storage::memory::MemoryStorage;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::Storage;
use pi_tui::autocomplete::CombinedAutocompleteProvider;
use pi_tui::components::editor::{Editor, EditorOptions};
use pi_tui::components::text::Text;
use pi_tui::keys::{is_key_release, matches_key};
use pi_tui::terminal::{ProcessTerminal, Terminal};
use pi_tui::tui::{ComponentRef, InputListenerResult, component};
use pi_tui::tui_main_screen::{TuiMainScreen, as_ref};

use super::chat::Chat;
use super::footer::{Footer, FooterState, Totals, git_branch};
use super::keybindings;
use super::selectors::{
    ActionSlot, OverlayAction, action_slot, model::ModelSelector,
    scoped_models::ScopedModelsSelector, settings::SettingsCallbacks, settings::SettingsConfig,
    settings::SettingsSelector, thinking::ThinkingSelector, trust::TrustSelector,
};
use super::slash_commands::{self, Command, Counts, SessionInfo};
use super::status::{StatusArea, WORKING, header};
use super::theme;
use crate::model_resolver;
use crate::models::ModelRegistry;
use crate::print::{self, ToolSettings};
use crate::run_config::{level_name, parse_level};
use crate::session_info;
use crate::session_list;
use crate::settings::{self as settings_mod, SettingsManager};

/// What the loop hears from other threads.
pub enum Incoming {
    Events(Vec<AgentEvent>),
    Report(String),
}

/// Opens a session's harness and root conversation. The loop owns what an open returns, and
/// opens another one when it switches session, so nothing here borrows the session it replaces.
pub struct Opener {
    pub registry: Arc<ModelRegistry>,
    /// A model the catalog lacks (a --model id) is still callable.
    pub model: Model,
    pub tools: Vec<String>,
    pub sections: Vec<(String, String)>,
    /// The tool environment without PI_SESSION_FILE, which depends on the session.
    pub tool_settings: ToolSettings,
    pub compaction: CompactionOverrides,
    pub sender: mpsc::Sender<Incoming>,
    pub reports: Arc<Mutex<mpsc::Sender<Incoming>>>,
}

/// A session open in the loop: its harness, its root conversation and the subscription that
/// feeds the loop its events.
pub struct Open {
    pub harness: Harness,
    pub root: Conversation,
    pub stream: AgentEventStream,
    /// The sqlite file it lives in; None under --no-session.
    pub path: Option<String>,
}

impl Opener {
    /// Open the session at path, or a fresh in-memory one for None: the harness, its root
    /// conversation, and a subscription whose events arrive as Incoming::Events.
    pub fn open(&self, path: Option<&str>) -> Result<Open, String> {
        let cancel = CancelToken::new();
        let storage: Arc<dyn Storage> = match path {
            Some(path) => {
                std::fs::create_dir_all(crate::paths::dirname(path)).map_err(|e| e.to_string())?;
                Arc::new(
                    SqliteStorage::open(path, &Default::default())
                        .map_err(|e| format!("Error: {e}"))?,
                )
            }
            None => Arc::new(MemoryStorage::new()),
        };
        let mut settings = self.tool_settings.clone();
        settings.env.retain(|(key, _)| key != "PI_SESSION_FILE");
        if let Some(path) = path {
            settings
                .env
                .push(("PI_SESSION_FILE".to_owned(), path.to_owned()));
        }
        let reports = Arc::clone(&self.reports);
        let harness = print::open_harness_with(
            storage,
            print::providers(Arc::clone(&self.registry), &self.model),
            print::extension(&self.tools, self.sections.clone(), settings),
            self.compaction.clone(),
            Arc::new(move |error| {
                if let Ok(sender) = reports.lock() {
                    let _ = sender.send(Incoming::Report(error.to_string()));
                }
            }),
        )
        .map_err(|e| e.to_string())?;
        let root = print::root(&harness, &cancel).map_err(|e| e.to_string())?;
        let stream = root.events(&cancel).map_err(|e| e.to_string())?;
        let feed = self.sender.clone();
        stream
            .start(move |events, _| {
                let _ = feed.send(Incoming::Events(events.to_vec()));
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        // Picks up a run a killed process left behind.
        harness.resume().map_err(|e| e.to_string())?;
        Ok(Open {
            harness,
            root,
            stream,
            path: path.map(str::to_owned),
        })
    }
}

impl Open {
    /// Stop the event feed and close the harness, which settles its transaction and the file.
    pub fn shutdown(&mut self, cancel: &CancelToken) -> Result<(), String> {
        self.stream.stop();
        print::close(&self.harness, cancel).map_err(|e| e.to_string())
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
    /// The first message from the command line, sent once the screen is up.
    pub initial: Option<(String, Vec<UserBlock>)>,
    pub messages: Vec<String>,
    pub provider_count: usize,
    /// The SQLite file the loop starts in; `None` under `--no-session`.
    pub session_path: Option<String>,
    /// `--name`: the name the session starts with.
    pub initial_name: Option<String>,
    pub incoming: mpsc::Receiver<Incoming>,
}

/// Keys the loop acts on, read by an input listener before the editor sees them.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Interrupt,
    Clear,
    Exit,
    Expand,
}

/// The components the loop swaps a selector in and out of.
struct Parts {
    header: ComponentRef,
    chat: ComponentRef,
    status: ComponentRef,
    editor: ComponentRef,
    footer: ComponentRef,
}

impl Parts {
    /// Draw the header, chat, status, footer and either the editor or `selector` in its place,
    /// focused.
    fn mount(&self, tui: &mut TuiMainScreen<ProcessTerminal>, selector: Option<&ComponentRef>) {
        tui.clear();
        tui.add_child(self.header.clone());
        tui.add_child(self.chat.clone());
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

fn available_models(registry: &ModelRegistry) -> Vec<Model> {
    registry.available().into_iter().cloned().collect()
}

/// The session's scoped models: `enabledModels` resolved over the available models, or none when
/// every model is enabled.
fn session_scope(settings: &SettingsManager, available: &[Model]) -> Vec<Model> {
    match settings.enabled_models() {
        Some(patterns) if !patterns.is_empty() => {
            model_resolver::resolve_model_scope(&patterns, available)
                .0
                .into_iter()
                .map(|scoped| scoped.model)
                .collect()
        }
        _ => Vec::new(),
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
    let override_level = settings
        .model_thinking_level(&model.provider, &model.id)
        .and_then(parse_level);
    let level = clamp_thinking_level(model, override_level.unwrap_or(*thinking));
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: model.provider.clone(),
                model_id: model.id.clone(),
            }),
            thinking_level: Change::Set(level),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;
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
    root.configure(
        &AgentChange {
            thinking_level: Change::Set(level),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;
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
        available_models: available_models(registry),
        current_model: Some((model.provider.clone(), model.id.clone())),
        default_model: default_model_ref(settings),
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

fn text_input(text: &str, images: &[UserBlock]) -> UserContent {
    let mut blocks = vec![UserBlock::Text(TextContent {
        kind: TextType,
        text: text.to_string(),
        text_signature: None,
    })];
    blocks.extend(images.iter().cloned());
    UserContent::Blocks(blocks)
}

fn submit(
    root: &Conversation,
    content: UserContent,
    busy: bool,
    cancel: &CancelToken,
) -> Result<(), String> {
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
        id: root.id().0,
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
    let agent = root.agent(cancel).map_err(|e| e.to_string())?;
    let Some(reference) = agent.model else {
        return Ok(false);
    };
    let Some(found) = registry
        .get_model(&reference.provider, &reference.model_id)
        .cloned()
    else {
        return Ok(false);
    };
    *thinking = clamp_thinking_level(&found, agent.thinking_level);
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
    if chat.working {
        let _ = open.root.abort(false, cancel);
    }
    let _ = open.root.wait_for_idle(cancel);
    open.shutdown(cancel)?;
    let next = opener.open(target)?;
    chat.clear();
    chat.load(&next.stream.snapshot);
    *open = next;
    Ok(())
}

/// Point a session's agent at the loop's model, thinking level and cwd, as upstream does for a
/// session it creates or switches to.
fn apply_agent(
    root: &Conversation,
    cancel: &CancelToken,
    model: &Model,
    thinking: ModelThinkingLevel,
    cwd: &str,
) -> Result<(), String> {
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: model.provider.clone(),
                model_id: model.id.clone(),
            }),
            thinking_level: Change::Set(clamp_thinking_level(model, thinking)),
            cwd: Change::Set(cwd.to_owned()),
            ..AgentChange::default()
        },
        cancel,
    )
    .map_err(|e| e.to_string())
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
        initial,
        messages,
        provider_count,
        session_path,
        initial_name,
        incoming,
    } = session;
    let (mut model, mut thinking) = (model, thinking);
    keybindings::install();
    let mut open = opener.open(session_path.as_deref())?;
    apply_agent(&open.root, &cancel, &model, thinking, &cwd)?;
    if let Some(name) = &initial_name {
        session_info::set_name(&open.root, &cancel, name).map_err(|e| e.to_string())?;
    }

    let mut chat = Chat::new(&cwd);
    chat.hide_thinking = settings.borrow().hide_thinking_block();
    chat.output_pad = settings.borrow().output_pad() as usize;
    let mut scope = session_scope(&settings.borrow(), &available_models(&registry));
    chat.load(&open.stream.snapshot);

    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut tui = TuiMainScreen::new(ProcessTerminal::new());
    tui.set_show_hardware_cursor(settings.borrow().show_hardware_cursor());
    tui.set_clear_on_shrink(settings.borrow().clear_on_shrink());
    let header = component(Text::new(&header(), 1, 1));
    let status = component(StatusArea::new());
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
    let dirty = Rc::new(Cell::new(true));
    {
        let mut e = editor.borrow_mut();
        let (s, d) = (submitted.clone(), dirty.clone());
        e.on_submit = Some(Box::new(move |t: &str| s.borrow_mut().push(t.to_string())));
        e.request_render = Some(Rc::new(move || d.set(true)));
        e.set_autocomplete_provider(Rc::new(CombinedAutocompleteProvider::new(
            slash_commands::autocomplete_commands(),
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
        let action = if matches_key(data, slash_commands::INTERRUPT)
            && !ed.borrow().is_showing_autocomplete()
        {
            Action::Interrupt
        } else if matches_key(data, slash_commands::CLEAR) {
            Action::Clear
        } else if matches_key(data, slash_commands::EXIT) && ed.borrow().get_text().is_empty() {
            Action::Exit
        } else if matches_key(data, slash_commands::EXPAND_TOOLS) {
            Action::Expand
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
        header: as_ref(&header),
        chat: chat.container.clone(),
        status: as_ref(&status),
        editor: as_ref(&editor),
        footer: as_ref(&footer),
    };
    let mut selector: Option<(ComponentRef, ActionSlot)> = None;
    parts.mount(&mut tui, None);
    tui.start();

    let mut pending: Vec<UserContent> = Vec::new();
    if let Some((text, images)) = &initial {
        pending.push(text_input(text, images));
    }
    pending.extend(messages.iter().map(|m| text_input(m, &[])));
    let mut last_clear: Option<Instant> = None;
    let mut quit = false;
    let mut next_timer = Instant::now();
    while !quit {
        // Queued command-line messages go one per idle turn, as print mode sends them.
        if !chat.working && !pending.is_empty() {
            let content = pending.remove(0);
            if let Err(e) = submit(&open.root, content, false, &cancel) {
                chat.notice("error", &format!("Error: {e}"));
            }
            chat.working = true;
            dirty.set(true);
        }
        let mut due = tui.tick(now());
        let (spun, spin_due) = status.borrow_mut().tick(now());
        if spun {
            dirty.set(true);
        }
        due = match (due, spin_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let timeout = due.map_or(50, |d| d.saturating_sub(now()).min(50));
        let events = tui.terminal.poll(Some(timeout));
        tui.handle_events(events);

        while let Ok(message) = incoming.try_recv() {
            match message {
                Incoming::Events(events) => {
                    for event in &events {
                        if chat.apply(event) {
                            dirty.set(true);
                        }
                    }
                }
                Incoming::Report(text) => {
                    chat.notice("error", &format!("Error: {text}"));
                    dirty.set(true);
                }
            }
        }
        for action in actions.borrow_mut().drain(..) {
            match action {
                Action::Interrupt => {
                    if chat.working
                        && let Err(e) = open.root.abort(false, &cancel)
                    {
                        chat.notice("error", &format!("Error: {e}"));
                    }
                }
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
            }
            dirty.set(true);
        }
        let pending = selector
            .as_ref()
            .and_then(|(_, slot)| slot.borrow_mut().take());
        if let Some(action) = pending {
            let mut close = false;
            match action {
                OverlayAction::Close => close = true,
                OverlayAction::SetModel {
                    provider,
                    id,
                    persist,
                } => {
                    close = true;
                    match registry.get_model(&provider, &id).cloned() {
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
                        let available = available_models(&registry);
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
                                Ok(false) => {
                                    apply_agent(&open.root, &cancel, &model, thinking, &cwd)
                                }
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
                OverlayAction::Setting { id, value } => match id.as_str() {
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
            if close {
                selector = None;
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
                        .and_then(|()| apply_agent(&open.root, &cancel, &model, thinking, &cwd))
                        {
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
                        let available = available_models(&registry);
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
                            available_models(&registry),
                            enabled,
                            slot.clone(),
                        ));
                        selector = Some((built, slot));
                    }
                    Command::Settings => {
                        let slot = action_slot();
                        let config =
                            settings_config(&settings.borrow(), &registry, &model, thinking);
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
            editor.borrow_mut().add_to_history(&text);
            if let Err(e) = submit(&open.root, text_input(&text, &[]), chat.working, &cancel) {
                chat.notice("error", &format!("Error: {e}"));
            }
            chat.working = true;
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
    open.shutdown(&cancel)?;
    Ok(open.path)
}
