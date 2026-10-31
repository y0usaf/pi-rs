use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use pi_durable::cancel::CancelToken;
use pi_extensions::Failed;
use pi_tui::components::editor::Editor;
use pi_tui::components::loader::LoaderIndicatorOptions;
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::terminal::Terminal;
use pi_tui::tui::{Component, ComponentRef, Container, component, same};
use serde::Deserialize;
use serde_json::{Value, json};

use super::chat::{Chat, DEFAULT_THINKING_LABEL};
use super::footer::Footer;
use super::screen::Screen;
use super::selectors::extension::{ExtensionDialog, ExtensionInput, ExtensionSelector, Outcome};
use super::theme::fg;

const MAX_WIDGET_LINES: usize = 10;
const PUMP_MS: u64 = 16;
const SETTLE_MS: u64 = 2;
const ABANDON_MS: u64 = 1000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Indicator {
    #[serde(default)]
    frames: Option<Vec<String>>,
    #[serde(default)]
    interval_ms: Option<f64>,
}

impl Indicator {
    fn options(self) -> LoaderIndicatorOptions {
        LoaderIndicatorOptions {
            frames: self.frames,
            interval_ms: self
                .interval_ms
                .filter(|ms| ms.is_finite() && *ms >= 1.0)
                .map(|ms| ms as u64),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "method")]
enum Shown {
    #[serde(rename = "notify")]
    Notify {
        message: String,
        #[serde(default, rename = "notifyType")]
        kind: Option<String>,
    },
    #[serde(rename = "setStatus")]
    SetStatus {
        #[serde(rename = "statusKey")]
        key: String,
        #[serde(default, rename = "statusText")]
        text: Option<String>,
    },
    #[serde(rename = "setWorkingMessage")]
    SetWorkingMessage {
        #[serde(default)]
        message: Option<String>,
    },
    #[serde(rename = "setWorkingVisible")]
    SetWorkingVisible { visible: bool },
    #[serde(rename = "setWorkingIndicator")]
    SetWorkingIndicator {
        #[serde(default)]
        indicator: Option<Indicator>,
    },
    #[serde(rename = "setHiddenThinkingLabel")]
    SetHiddenThinkingLabel {
        #[serde(default)]
        label: Option<String>,
    },
    #[serde(rename = "setWidget")]
    SetWidget {
        #[serde(rename = "widgetKey")]
        key: String,
        #[serde(default, rename = "widgetLines")]
        lines: Option<Vec<String>>,
        #[serde(default, rename = "widgetPlacement")]
        placement: Option<String>,
    },
    #[serde(rename = "setTitle")]
    SetTitle { title: String },
    #[serde(rename = "set_editor_text")]
    SetEditorText { text: String },
    #[serde(rename = "paste_to_editor")]
    PasteToEditor { text: String },
    #[serde(rename = "setToolsExpanded")]
    SetToolsExpanded { expanded: bool },
}

#[derive(Deserialize)]
#[serde(tag = "method")]
enum Asked {
    #[serde(rename = "select")]
    Select {
        title: String,
        options: Vec<String>,
        #[serde(default)]
        timeout: Option<f64>,
    },
    #[serde(rename = "confirm")]
    Confirm {
        title: String,
        message: String,
        #[serde(default)]
        timeout: Option<f64>,
    },
    #[serde(rename = "input")]
    Input {
        title: String,
        #[serde(default)]
        timeout: Option<f64>,
    },
    #[serde(rename = "get_editor_text")]
    GetEditorText,
    #[serde(rename = "get_tools_expanded")]
    GetToolsExpanded,
}

enum Message {
    Show(Shown),
    Ask {
        asked: Asked,
        cancel: CancelToken,
        answer: Sender<Value>,
    },
    Failed(Failed),
    Unreadable(String),
}

pub struct Requests(Sender<Message>);

impl crate::extensions::Ui for Requests {
    fn show(&self, request: Value) {
        let message = match serde_json::from_value(request) {
            Ok(shown) => Message::Show(shown),
            Err(e) => Message::Unreadable(format!(
                "Warning: the extension host asked for UI pi-rs cannot read: {e}"
            )),
        };
        let _ = self.0.send(message);
    }

    fn ask(&self, request: Value, cancel: &CancelToken) -> Result<Value, String> {
        let asked = serde_json::from_value(request)
            .map_err(|e| format!("the extension host asked for UI pi-rs cannot read: {e}"))?;
        let (answer, answered) = channel();
        self.0
            .send(Message::Ask {
                asked,
                cancel: cancel.clone(),
                answer,
            })
            .map_err(|_| "the screen is closed".to_owned())?;
        answered
            .recv()
            .map_err(|_| "the dialog closed without an answer".to_owned())
    }

    fn failed(&self, failed: &Failed) {
        let _ = self.0.send(Message::Failed(failed.clone()));
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Select,
    Confirm,
    Input,
}

impl Kind {
    fn response(self, outcome: Outcome) -> Value {
        match (self, outcome) {
            (_, Outcome::Cancelled) => json!({ "cancelled": true }),
            (Kind::Confirm, Outcome::Chosen(option)) => json!({ "confirmed": option == "Yes" }),
            (Kind::Select | Kind::Input, Outcome::Chosen(value)) => json!({ "value": value }),
        }
    }
}

struct Open {
    dialog: Rc<RefCell<dyn ExtensionDialog>>,
    shown: ComponentRef,
    kind: Kind,
    cancel: CancelToken,
    answer: Sender<Value>,
}

fn open<D: ExtensionDialog + 'static>(
    dialog: D,
    kind: Kind,
    cancel: CancelToken,
    answer: Sender<Value>,
) -> Open {
    let dialog = component(dialog);
    Open {
        shown: dialog.clone(),
        dialog,
        kind,
        cancel,
        answer,
    }
}

fn widget(lines: &[String]) -> ComponentRef {
    let mut container = Container::new();
    for line in lines.iter().take(MAX_WIDGET_LINES) {
        container.add_child(component(Text::new(line, 1, 0)));
    }
    if lines.len() > MAX_WIDGET_LINES {
        container.add_child(component(Text::new(
            &fg("muted", "... (widget truncated)"),
            1,
            0,
        )));
    }
    component(container)
}

pub struct Surface {
    pub editor: Rc<RefCell<Editor>>,
    pub editor_slot: Rc<RefCell<Container>>,
    pub widgets_above: Rc<RefCell<Container>>,
    pub widgets_below: Rc<RefCell<Container>>,
    pub footer: Rc<RefCell<Footer>>,
    pub dialog_open: Rc<Cell<bool>>,
}

#[derive(Default)]
pub struct Effects {
    pub opened: bool,
    pub tools_expanded: Option<bool>,
}

pub struct ExtensionUi {
    receiver: Receiver<Message>,
    surface: Surface,
    start: Instant,
    installed: bool,
    closing: bool,
    dialogs: Vec<Open>,
    above: Vec<(String, ComponentRef)>,
    below: Vec<(String, ComponentRef)>,
    effects: Effects,
    pub working_message: Option<String>,
    pub working_visible: bool,
    pub working_indicator: Option<LoaderIndicatorOptions>,
}

impl ExtensionUi {
    pub fn new(surface: Surface, start: Instant) -> (ExtensionUi, Requests) {
        let (sender, receiver) = channel();
        let ui = ExtensionUi {
            receiver,
            surface,
            start,
            installed: false,
            closing: false,
            dialogs: Vec::new(),
            above: Vec::new(),
            below: Vec::new(),
            effects: Effects::default(),
            working_message: None,
            working_visible: true,
            working_indicator: None,
        };
        (ui, Requests(sender))
    }

    pub fn install(&mut self, live: &crate::extensions::Live, requests: Requests) {
        live.set_ui(std::sync::Arc::new(requests));
        self.installed = true;
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn take_effects(&mut self) -> Effects {
        std::mem::take(&mut self.effects)
    }

    pub fn due(&self) -> Option<u64> {
        self.dialogs
            .iter()
            .filter_map(|open| open.dialog.borrow().due())
            .min()
    }

    pub fn serve(&mut self, tui: &mut Screen, chat: &mut Chat) -> bool {
        let now = self.now();
        let mut drew = false;
        while let Ok(message) = self.receiver.try_recv() {
            drew |= self.apply(message, tui, chat, now);
        }
        drew | self.settle(tui, chat, now)
    }

    fn apply(&mut self, message: Message, tui: &mut Screen, chat: &mut Chat, now: u64) -> bool {
        match message {
            Message::Show(shown) => self.show(shown, tui, chat),
            Message::Ask {
                asked,
                cancel,
                answer,
            } => {
                if cancel.is_cancelled() {
                    return false;
                }
                let dismissed = json!({ "cancelled": true });
                let opened = match asked {
                    Asked::GetEditorText => {
                        let _ =
                            answer.send(json!(self.surface.editor.borrow().get_expanded_text()));
                        return false;
                    }
                    Asked::GetToolsExpanded => {
                        let expanded = self.effects.tools_expanded.unwrap_or(chat.expanded);
                        let _ = answer.send(json!(expanded));
                        return false;
                    }
                    _ if self.closing => {
                        let _ = answer.send(dismissed);
                        return false;
                    }
                    Asked::Select {
                        title,
                        options,
                        timeout,
                    } => open(
                        ExtensionSelector::new(&title, options, timeout, now),
                        Kind::Select,
                        cancel,
                        answer,
                    ),
                    Asked::Confirm {
                        title,
                        message,
                        timeout,
                    } => open(
                        ExtensionSelector::new(
                            &format!("{title}\n{message}"),
                            vec!["Yes".to_owned(), "No".to_owned()],
                            timeout,
                            now,
                        ),
                        Kind::Confirm,
                        cancel,
                        answer,
                    ),
                    Asked::Input { title, timeout } => open(
                        ExtensionInput::new(&title, timeout, now),
                        Kind::Input,
                        cancel,
                        answer,
                    ),
                };
                self.dialogs.push(opened);
                self.effects.opened = true;
                self.show_top(tui);
                true
            }
            Message::Failed(failed) => {
                chat.extension_error(&failed.path, &failed.error, failed.stack.as_deref());
                true
            }
            Message::Unreadable(text) => {
                chat.notice("warning", &text);
                true
            }
        }
    }

    fn show(&mut self, shown: Shown, tui: &mut Screen, chat: &mut Chat) -> bool {
        match shown {
            Shown::Notify { message, kind } => match kind.as_deref() {
                Some("error") => chat.notice("error", &format!("Error: {message}")),
                Some("warning") => chat.notice("warning", &format!("Warning: {message}")),
                _ => chat.show_status(&message),
            },
            Shown::SetStatus { key, text } => {
                let statuses = &mut self.surface.footer.borrow_mut().state.statuses;
                match text {
                    Some(text) => statuses.insert(key, text),
                    None => statuses.remove(&key),
                };
            }
            Shown::SetWorkingMessage { message } => self.working_message = message,
            Shown::SetWorkingVisible { visible } => self.working_visible = visible,
            Shown::SetWorkingIndicator { indicator } => {
                self.working_indicator = indicator.map(Indicator::options);
            }
            Shown::SetHiddenThinkingLabel { label } => {
                chat.set_thinking_label(label.as_deref().unwrap_or(DEFAULT_THINKING_LABEL));
            }
            Shown::SetWidget {
                key,
                lines,
                placement,
            } => {
                self.above.retain(|(shown, _)| *shown != key);
                self.below.retain(|(shown, _)| *shown != key);
                if let Some(lines) = lines {
                    let placed = match placement.as_deref() {
                        Some("belowEditor") => &mut self.below,
                        _ => &mut self.above,
                    };
                    placed.push((key, widget(&lines)));
                }
                self.render_widgets();
            }
            Shown::SetTitle { title } => tui.set_title(&title),
            Shown::SetEditorText { text } => self.surface.editor.borrow_mut().set_text(&text),
            Shown::PasteToEditor { text } => self
                .surface
                .editor
                .borrow_mut()
                .handle_input(&format!("\x1b[200~{text}\x1b[201~")),
            Shown::SetToolsExpanded { expanded } => self.effects.tools_expanded = Some(expanded),
        }
        true
    }

    fn render_widgets(&self) {
        let mut above = self.surface.widgets_above.borrow_mut();
        above.clear();
        above.add_child(component(Spacer::new(1)));
        for (_, shown) in &self.above {
            above.add_child(shown.clone());
        }
        let mut below = self.surface.widgets_below.borrow_mut();
        below.clear();
        for (_, shown) in &self.below {
            below.add_child(shown.clone());
        }
    }

    fn settle(&mut self, tui: &mut Screen, chat: &Chat, now: u64) -> bool {
        let top = self.top();
        let mut drew = false;
        let mut index = 0;
        while index < self.dialogs.len() {
            let open = &self.dialogs[index];
            let outcome = {
                let mut dialog = open.dialog.borrow_mut();
                if dialog.take_tools_toggle() {
                    let expanded = self.effects.tools_expanded.unwrap_or(chat.expanded);
                    self.effects.tools_expanded = Some(!expanded);
                }
                drew |= dialog.tick(now);
                dialog.take_outcome()
            };
            if open.cancel.is_cancelled() || outcome.is_some() {
                let open = self.dialogs.remove(index);
                if let Some(outcome) = outcome.filter(|_| !open.cancel.is_cancelled()) {
                    let _ = open.answer.send(open.kind.response(outcome));
                }
                drew = true;
                continue;
            }
            index += 1;
        }
        let moved = match (top, self.top()) {
            (Some(before), Some(after)) => !same(&before, &after),
            (before, after) => before.is_some() != after.is_some(),
        };
        if moved {
            self.show_top(tui);
        }
        drew
    }

    fn top(&self) -> Option<ComponentRef> {
        self.dialogs.last().map(|open| open.shown.clone())
    }

    fn close_dialogs(&mut self, tui: &mut Screen) {
        self.closing = true;
        if self.dialogs.is_empty() {
            return;
        }
        for open in self.dialogs.drain(..) {
            let _ = open.answer.send(open.kind.response(Outcome::Cancelled));
        }
        self.show_top(tui);
    }

    fn show_top(&self, tui: &mut Screen) {
        let top: ComponentRef = match self.dialogs.last() {
            Some(open) => open.shown.clone(),
            None => self.surface.editor.clone(),
        };
        {
            let mut slot = self.surface.editor_slot.borrow_mut();
            slot.clear();
            slot.add_child(top.clone());
        }
        tui.set_focus(Some(top));
        tui.request_render(false);
        self.surface.dialog_open.set(!self.dialogs.is_empty());
    }

    pub fn wait<T: Send + 'static>(
        &mut self,
        tui: &mut Screen,
        chat: &mut Chat,
        quit: &dyn Fn() -> bool,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        if !self.installed {
            return Some(work());
        }
        let (done, finished) = channel();
        let worker = std::thread::spawn(move || {
            let _ = done.send(work());
        });
        let mut abandon_at: Option<u64> = None;
        loop {
            match finished.recv_timeout(Duration::from_millis(SETTLE_MS)) {
                Ok(value) => return Some(value),
                Err(RecvTimeoutError::Disconnected) => {
                    if let Err(panic) = worker.join() {
                        std::panic::resume_unwind(panic);
                    }
                    return None;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = self.now();
            if super::signals::requested() {
                self.close_dialogs(tui);
            }
            if abandon_at.is_none() && quit() {
                self.close_dialogs(tui);
                abandon_at = Some(now + ABANDON_MS);
            }
            if abandon_at.is_some_and(|at| now >= at) {
                return None;
            }
            let due = [tui.tick(now), self.due(), abandon_at]
                .into_iter()
                .flatten()
                .min();
            let timeout = due.map_or(PUMP_MS, |due| due.saturating_sub(now).min(PUMP_MS));
            let events = tui.terminal().poll(Some(timeout));
            let typed = !events.is_empty();
            tui.handle_events(events);
            if self.serve(tui, chat) || typed {
                tui.request_render(false);
            }
        }
    }
}
