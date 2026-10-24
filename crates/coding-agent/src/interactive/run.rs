//! The interactive loop: the editor submits to the durable conversation, its agent events come
//! back over a channel and redraw the chat. Escape aborts the run, Ctrl+C clears the editor (twice
//! quits), Ctrl+D quits on an empty editor, Ctrl+O expands tool output.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use pi_durable::ai::types::{
    AssistantBlock, Message, Model, ModelThinkingLevel, TextContent, TextType, UserBlock,
    UserContent,
};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::events::AgentEvent;
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::types::ModelRef;
use pi_tui::autocomplete::CombinedAutocompleteProvider;
use pi_tui::components::editor::{Editor, EditorOptions};
use pi_tui::components::text::Text;
use pi_tui::keys::{is_key_release, matches_key};
use pi_tui::terminal::{ProcessTerminal, Terminal};
use pi_tui::tui::{InputListenerResult, component};
use pi_tui::tui_main_screen::{TuiMainScreen, as_ref};

use super::chat::Chat;
use super::footer::{Footer, FooterState, Totals, git_branch};
use super::slash_commands::{self, Command, Counts, SessionInfo};
use super::status::{StatusArea, WORKING, header};
use super::theme;
use crate::run_config::level_name;

/// What the loop hears from other threads.
pub enum Incoming {
    Events(Vec<AgentEvent>),
    Report(String),
}

pub struct Session<'a> {
    pub harness: &'a Harness,
    pub root: &'a Conversation,
    pub model: Model,
    pub thinking: ModelThinkingLevel,
    pub cwd: String,
    /// The first message from the command line, sent once the screen is up.
    pub initial: Option<(String, Vec<UserBlock>)>,
    pub messages: Vec<String>,
    pub provider_count: usize,
    /// The SQLite file the conversation lives in; `None` under `--no-session`.
    pub session_path: Option<String>,
    pub incoming: mpsc::Receiver<Incoming>,
    pub sender: mpsc::Sender<Incoming>,
}

/// Keys the loop acts on, read by an input listener before the editor sees them.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Interrupt,
    Clear,
    Exit,
    Expand,
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

/// `/session`: the file, the id, message counts over the stored conversation, and the totals the
/// footer shows.
fn session_info(
    root: &Conversation,
    cancel: &CancelToken,
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
        file: path.map(str::to_owned),
        id: root.id().0,
        counts,
        totals,
        context_tokens,
    }))
}

pub fn run(session: Session<'_>) -> Result<(), String> {
    let cancel = CancelToken::new();
    let Session {
        harness,
        root,
        model,
        thinking,
        cwd,
        initial,
        messages,
        provider_count,
        session_path,
        incoming,
        sender,
    } = session;
    root.configure(
        &AgentChange {
            model: Change::Set(ModelRef {
                provider: model.provider.clone(),
                model_id: model.id.clone(),
            }),
            thinking_level: Change::Set(thinking),
            cwd: Change::Set(cwd.clone()),
            ..AgentChange::default()
        },
        &cancel,
    )
    .map_err(|e| e.to_string())?;

    let mut chat = Chat::new(&cwd);
    let stream = root.events(&cancel).map_err(|e| e.to_string())?;
    chat.load(&stream.snapshot);
    let feed = sender.clone();
    stream
        .start(move |events, _| {
            let _ = feed.send(Incoming::Events(events.to_vec()));
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    // Picks up a run a killed process left behind.
    harness.resume().map_err(|e| e.to_string())?;

    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut tui = TuiMainScreen::new(ProcessTerminal::new());
    let header = component(Text::new(&header(), 1, 1));
    let status = component(StatusArea::new());
    let editor = component(Editor::new(
        theme::editor(),
        EditorOptions {
            padding_x: Some(0),
            autocomplete_max_visible: None,
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
    let (acts, ed) = (actions.clone(), editor.clone());
    tui.add_input_listener(Box::new(move |data: &str| {
        if is_key_release(data) {
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
    tui.add_child(as_ref(&header));
    tui.add_child(chat.container.clone());
    tui.add_child(as_ref(&status));
    tui.add_child(as_ref(&editor));
    tui.add_child(as_ref(&footer));
    tui.set_focus(Some(as_ref(&editor)));
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
            if let Err(e) = submit(root, content, false, &cancel) {
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
                        && let Err(e) = root.abort(false, &cancel)
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
        let texts: Vec<String> = submitted.borrow_mut().drain(..).collect();
        for text in texts {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(command) = slash_commands::dispatch(trimmed) {
                match command {
                    Command::Quit => quit = true,
                    Command::New => match root.reset(None, &cancel) {
                        Ok(()) => {
                            chat.clear();
                            chat.notice("accent", "✓ New session started");
                        }
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    },
                    Command::Session => match session_info(
                        root,
                        &cancel,
                        session_path.as_deref(),
                        chat.totals,
                        chat.context_tokens,
                    ) {
                        Ok(info) => chat.block(&info),
                        Err(e) => chat.notice("error", &format!("Error: {e}")),
                    },
                    Command::Hotkeys => chat.block(&slash_commands::hotkeys()),
                    Command::Compact(instructions) => {
                        if let Err(e) = root.compact(instructions.as_deref(), &cancel) {
                            chat.notice("error", &format!("Error: {e}"));
                        }
                    }
                    Command::Unavailable(name) => {
                        chat.notice("dim", &format!("/{name} is not available in pi-rs yet"))
                    }
                }
                dirty.set(true);
                continue;
            }
            editor.borrow_mut().add_to_history(&text);
            if let Err(e) = submit(root, text_input(&text, &[]), chat.working, &cancel) {
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
    stream.stop();
    if chat.working {
        let _ = root.abort(false, &cancel);
    }
    Ok(())
}
