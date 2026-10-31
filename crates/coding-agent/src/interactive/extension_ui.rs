use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use pi_durable::cancel::CancelToken;
use pi_extensions::Failed;
use pi_tui::autocomplete::CombinedAutocompleteProvider;
use pi_tui::components::editor::Editor;
use pi_tui::components::loader::LoaderIndicatorOptions;
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::terminal::Terminal;
use pi_tui::terminal_colors::TerminalColors;
use pi_tui::tui::{Component, ComponentRef, Container, OverlayId, component, overlay_width, same};
use serde::Deserialize;
use serde_json::{Value, json};

use super::chat::{Chat, DEFAULT_THINKING_LABEL};
use super::completions::{Completing, CompletionAsked, CompletionShown, Completions, Served};
use super::footer::Footer;
use super::hosted::{Hosted, HostedWire, OverlayWire, ToHost};
use super::hosted_editor::{EditorAction, HostedEditor, Step};
use super::screen::Screen;
use super::selectors::extension::{
    ExtensionDialog, ExtensionEditor, ExtensionInput, ExtensionSelector, Outcome,
};
use super::terminal_input::{TerminalInput, Verdict};
use super::theme::fg;
use crate::extensions::Live;

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
    #[serde(rename = "terminalInput")]
    TerminalInput { enabled: bool },
    #[serde(rename = "terminalInputResult")]
    TerminalInputResult { sequence: u64, verdict: Verdict },
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
    #[serde(rename = "componentLines")]
    ComponentLines {
        component: u64,
        width: usize,
        lines: Vec<String>,
    },
    #[serde(rename = "customClose")]
    CustomClose { component: u64 },
    #[serde(rename = "showOverlay")]
    ShowOverlay {
        hosted: HostedWire,
        #[serde(default)]
        options: Option<OverlayWire>,
        visible: bool,
    },
    #[serde(rename = "overlay")]
    Overlay {
        component: u64,
        #[serde(flatten)]
        action: OverlayAction,
    },
    #[serde(rename = "hideTopOverlay")]
    HideTopOverlay,
    #[serde(rename = "setWidgetComponent")]
    SetWidgetComponent {
        #[serde(rename = "widgetKey")]
        key: String,
        hosted: HostedWire,
        #[serde(default, rename = "widgetPlacement")]
        placement: Option<String>,
    },
    #[serde(rename = "setEditorComponent")]
    SetEditorComponent {
        #[serde(default)]
        hosted: Option<HostedWire>,
    },
    #[serde(rename = "editorChange")]
    EditorChange { component: u64, text: String },
    #[serde(rename = "editorSubmit")]
    EditorSubmit { component: u64, text: String },
    #[serde(rename = "editorAction")]
    EditorAction {
        component: u64,
        action: EditorAction,
    },
    #[serde(rename = "editorInputDone")]
    EditorInputDone { component: u64, sequence: u64 },
    #[serde(rename = "setFooter")]
    SetFooter { hosted: Option<HostedWire> },
    #[serde(rename = "setHeader")]
    SetHeader { hosted: Option<HostedWire> },
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum OverlayAction {
    Hide,
    SetHidden { hidden: bool },
    Focus,
    Unfocus,
    Visible { visible: bool },
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
    #[serde(rename = "editor")]
    Editor {
        title: String,
        #[serde(default)]
        prefill: Option<String>,
    },
    #[serde(rename = "get_editor_text")]
    GetEditorText,
    #[serde(rename = "get_editor_raw_text")]
    GetEditorRawText,
    #[serde(rename = "setTheme")]
    SetTheme { name: String },
    #[serde(rename = "get_tools_expanded")]
    GetToolsExpanded,
    #[serde(rename = "custom")]
    Custom { hosted: HostedWire },
    #[serde(rename = "showOverlay")]
    ShowOverlay {
        hosted: HostedWire,
        #[serde(default)]
        options: Option<OverlayWire>,
        visible: bool,
    },
    #[serde(rename = "overlayFocused")]
    OverlayFocused { component: u64 },
    #[serde(rename = "overlayBounds")]
    OverlayBounds { component: u64 },
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

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Line<C, U> {
    #[serde(rename = "extension_completion")]
    Completion(C),
    #[serde(rename = "extension_ui_request")]
    Ui(U),
}

pub struct Requests {
    messages: Sender<Message>,
    completing: Sender<Completing>,
}

impl crate::extensions::Ui for Requests {
    fn show(&self, request: Value) {
        match serde_json::from_value::<Line<CompletionShown, Shown>>(request) {
            Ok(Line::Completion(shown)) => {
                let _ = self.completing.send(shown.into());
            }
            Ok(Line::Ui(shown)) => {
                let _ = self.messages.send(Message::Show(shown));
            }
            Err(e) => {
                let _ = self.messages.send(Message::Unreadable(format!(
                    "Warning: the extension host asked for UI pi-rs cannot read: {e}"
                )));
            }
        }
    }

    fn ask(&self, request: Value, cancel: &CancelToken) -> Result<Value, String> {
        let line = serde_json::from_value::<Line<CompletionAsked, Asked>>(request)
            .map_err(|e| format!("the extension host asked for UI pi-rs cannot read: {e}"))?;
        let (answer, answered) = channel();
        let sent = match line {
            Line::Completion(asked) => self.completing.send(asked.answered_by(answer)).is_ok(),
            Line::Ui(asked) => self
                .messages
                .send(Message::Ask {
                    asked,
                    cancel: cancel.clone(),
                    answer,
                })
                .is_ok(),
        };
        if !sent {
            return Err("the screen is closed".to_owned());
        }
        answered
            .recv()
            .map_err(|_| "the dialog closed without an answer".to_owned())
    }

    fn failed(&self, failed: &Failed) {
        let _ = self.messages.send(Message::Failed(failed.clone()));
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Select,
    Confirm,
    Input,
    Editor,
}

impl Kind {
    fn response(self, outcome: Outcome) -> Value {
        match (self, outcome) {
            (_, Outcome::Cancelled) => json!({ "cancelled": true }),
            (Kind::Confirm, Outcome::Chosen(option)) => json!({ "confirmed": option == "Yes" }),
            (Kind::Select | Kind::Input | Kind::Editor, Outcome::Chosen(value)) => {
                json!({ "value": value })
            }
        }
    }
}

enum Waiting {
    Answer {
        dialog: Rc<RefCell<dyn ExtensionDialog>>,
        kind: Kind,
        cancel: CancelToken,
        answer: Sender<Value>,
    },
    Hosted {
        component: u64,
        saved: String,
        answer: Sender<Value>,
    },
}

struct Overlay {
    shown: OverlayId,
    visible: Rc<Cell<bool>>,
    answer: Option<Sender<Value>>,
}

impl Overlay {
    fn closed(self) {
        if let Some(answer) = self.answer {
            let _ = answer.send(closed());
        }
    }
}

fn closed() -> Value {
    json!({ "closed": true })
}

struct Open {
    shown: ComponentRef,
    waiting: Waiting,
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
        waiting: Waiting::Answer {
            dialog,
            kind,
            cancel,
            answer,
        },
    }
}

struct Widget {
    key: String,
    shown: ComponentRef,
    hosted: Option<u64>,
}

#[derive(Default)]
struct Told {
    size: Option<(usize, usize)>,
    kitty: Option<bool>,
    expanded: Option<bool>,
    providers: Option<usize>,
    style: Option<(usize, usize, bool, String)>,
    theme: Option<String>,
    colors: Option<TerminalColors>,
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
    pub terminal_input: TerminalInput,
    pub themes: super::theme::Sources,
    pub settings: Rc<RefCell<crate::settings::SettingsManager>>,
    pub active_editor: Rc<RefCell<ComponentRef>>,
    pub busy: Box<dyn Fn() -> bool>,
    pub autocomplete: Rc<CombinedAutocompleteProvider>,
    pub editor: Rc<RefCell<Editor>>,
    pub editor_slot: Rc<RefCell<Container>>,
    pub widgets_above: Rc<RefCell<Container>>,
    pub widgets_below: Rc<RefCell<Container>>,
    pub footer: Rc<RefCell<Footer>>,
    pub footer_slot: Rc<RefCell<Container>>,
    pub header_slot: Rc<RefCell<Container>>,
    pub built_in_header: ComponentRef,
    pub dialog_open: Rc<Cell<bool>>,
    pub external_editor: Box<dyn Fn() -> String>,
}

#[derive(Default)]
pub struct Effects {
    pub opened: bool,
    pub tools_expanded: Option<bool>,
    pub editor_actions: Vec<EditorAction>,
    pub submitted: Vec<String>,
}

pub struct ExtensionUi {
    receiver: Receiver<Message>,
    completions: Completions,
    surface: Surface,
    start: Instant,
    live: Option<&'static Live>,
    closing: bool,
    dialogs: Vec<Open>,
    above: Vec<Widget>,
    below: Vec<Widget>,
    hosted: HashMap<u64, Rc<RefCell<Hosted>>>,
    overlays: HashMap<u64, Overlay>,
    footer: Option<u64>,
    header: Option<(u64, ComponentRef)>,
    editor: Option<HostedEditor>,
    border: (bool, String),
    told: Told,
    effects: Effects,
    pub working_message: Option<String>,
    pub working_visible: bool,
    pub working_indicator: Option<LoaderIndicatorOptions>,
}

impl ExtensionUi {
    pub fn new(surface: Surface, start: Instant) -> (ExtensionUi, Requests) {
        let (messages, receiver) = channel();
        let (completing, completed) = channel();
        let ui = ExtensionUi {
            receiver,
            completions: Completions::new(completed, surface.autocomplete.clone()),
            surface,
            start,
            live: None,
            closing: false,
            dialogs: Vec::new(),
            above: Vec::new(),
            below: Vec::new(),
            hosted: HashMap::new(),
            overlays: HashMap::new(),
            footer: None,
            header: None,
            editor: None,
            border: (false, String::new()),
            told: Told::default(),
            effects: Effects::default(),
            working_message: None,
            working_visible: true,
            working_indicator: None,
        };
        (
            ui,
            Requests {
                messages,
                completing,
            },
        )
    }

    pub fn set_autocomplete_base(&mut self, base: Rc<CombinedAutocompleteProvider>) {
        self.completions.set_base(&self.surface.editor, base);
    }

    pub fn set_border(&mut self, bash: bool, thinking: &str) {
        if self.border.0 != bash || self.border.1 != thinking {
            self.border = (bash, thinking.to_owned());
        }
    }

    pub fn add_to_history(&mut self, text: &str) {
        match (&self.editor, self.live) {
            (Some(editor), Some(live)) => live.ui(&ToHost::EditorAddHistory {
                component: editor.component,
                text,
            }),
            _ => self.surface.editor.borrow_mut().add_to_history(text),
        }
    }

    pub fn hosts_editor(&self) -> bool {
        self.editor.is_some()
    }

    pub fn hurry(&self) -> bool {
        self.editor.as_ref().is_some_and(HostedEditor::in_flight)
            || self.surface.terminal_input.waiting()
    }

    fn swap_editor(&mut self, tui: &mut Screen, hosted: Option<HostedWire>) {
        let leftover = match self.editor.take() {
            Some(old) => {
                self.hosted.remove(&old.component);
                old.leftover()
            }
            None => Vec::new(),
        };
        let text = self.surface.editor.borrow().get_text();
        let shown: ComponentRef = match hosted.and_then(|wire| {
            let component = wire.component;
            self.host(wire).map(|proxy| (component, proxy))
        }) {
            Some((component, proxy)) => {
                let editor = HostedEditor::new(component, proxy, text);
                let shown = editor.shown.clone();
                self.editor = Some(editor);
                shown
            }
            None => self.surface.editor.clone(),
        };
        *self.surface.active_editor.borrow_mut() = shown;
        self.effects.opened = true;
        if self.dialogs.is_empty() {
            self.show_top(tui);
        }
        for data in leftover {
            tui.handle_terminal_input(&data);
        }
    }

    fn step_editor(&mut self, tui: &mut Screen, chat: &mut Chat) -> bool {
        let held = (self.surface.busy)()
            || !self.effects.editor_actions.is_empty()
            || !self.effects.submitted.is_empty();
        let (Some(editor), Some(live)) = (self.editor.as_mut(), self.live) else {
            return false;
        };
        editor.sync(live, &self.surface.editor.borrow().get_text());
        let component = editor.component;
        match editor.step(live, held) {
            Step::Idle | Step::Sent => false,
            Step::TimedOut => {
                live.ui(&ToHost::EditorDropped { component });
                chat.notice(
                    "warning",
                    "Extension editor did not answer; restored the built-in editor",
                );
                self.swap_editor(tui, None);
                true
            }
        }
    }

    pub fn install(&mut self, live: &'static Live, requests: Requests) {
        live.set_ui(std::sync::Arc::new(requests));
        self.live = Some(live);
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
            .filter_map(|open| match &open.waiting {
                Waiting::Answer { dialog, .. } => dialog.borrow().due(),
                Waiting::Hosted { .. } => None,
            })
            .min()
    }

    pub fn serve(&mut self, tui: &mut Screen, chat: &mut Chat) -> bool {
        let now = self.now();
        self.tell(tui, chat);
        let mut drew = false;
        while let Ok(message) = self.receiver.try_recv() {
            drew |= self.apply(message, tui, chat, now);
        }
        match self.completions.serve(&self.surface.editor, self.live) {
            Served::Quiet => {}
            Served::Delivered => drew = true,
            Served::TimedOut => {
                chat.notice(
                    "warning",
                    "Extension autocomplete provider timed out; extension autocomplete disabled",
                );
                drew = true;
            }
        }
        drew |= self.step_editor(tui, chat);
        let (input_drew, timed_out) = self.surface.terminal_input.dispatch(tui, self.live);
        if timed_out {
            chat.notice(
                "warning",
                "Extension terminal-input callback timed out; input callbacks disabled",
            );
        }
        drew | input_drew | self.settle(tui, chat, now)
    }

    fn tell(&mut self, tui: &mut Screen, chat: &Chat) {
        let Some(live) = self.live else {
            return;
        };
        let terminal = tui.terminal();
        let size = (terminal.columns(), terminal.rows());
        if self.told.size != Some(size) {
            self.told.size = Some(size);
            live.ui(&ToHost::Size {
                columns: size.0,
                rows: size.1,
            });
        }
        let kitty = pi_tui::keys::is_kitty_protocol_active();
        if self.told.kitty != Some(kitty) {
            self.told.kitty = Some(kitty);
            live.ui(&ToHost::Kitty { active: kitty });
        }
        if self.told.expanded != Some(chat.expanded) {
            self.told.expanded = Some(chat.expanded);
            live.ui(&ToHost::Expanded {
                expanded: chat.expanded,
            });
        }
        let providers = self.surface.footer.borrow().state.provider_count;
        if self.told.providers != Some(providers) {
            self.told.providers = Some(providers);
            live.ui(&ToHost::Providers { count: providers });
        }
        let style = {
            let editor = self.surface.editor.borrow();
            (
                editor.padding_x(),
                editor.autocomplete_max_visible(),
                self.border.0,
                self.border.1.clone(),
            )
        };
        if let Some(colors) = super::theme::reported_colors()
            && self.told.colors.as_ref() != Some(&colors)
        {
            live.ui(&ToHost::TerminalColors {
                foreground: colors.foreground.map(Into::into),
                background: colors.background.map(Into::into),
                palette: colors.palette.map(|palette| palette.map(Into::into).into()),
            });
            self.told.colors = Some(colors);
        }
        let theme = super::theme::current().name.clone();
        if self.told.theme.as_ref() != Some(&theme) {
            live.ui(&ToHost::Theme { name: &theme });
            self.told.theme = Some(theme);
        }
        if self.told.style.as_ref() != Some(&style) {
            live.ui(&ToHost::EditorStyle {
                padding_x: style.0,
                autocomplete_max_visible: style.1,
                bash: style.2,
                thinking: &style.3,
            });
            self.told.style = Some(style);
        }
    }

    fn host(&mut self, wire: HostedWire) -> Option<Rc<RefCell<Hosted>>> {
        let id = wire.component;
        let proxy = component(Hosted::new(self.live?, wire));
        self.hosted.insert(id, proxy.clone());
        Some(proxy)
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
                    Asked::GetEditorRawText => {
                        let _ = answer.send(json!(self.surface.editor.borrow().get_text()));
                        return false;
                    }
                    Asked::SetTheme { name } => {
                        let result = match super::theme::set_named(&name, &self.surface.themes) {
                            Ok(()) => {
                                let mut settings = self.surface.settings.borrow_mut();
                                if settings.theme() != Some(name.as_str()) {
                                    settings.set_theme(&name);
                                }
                                json!({ "success": true })
                            }
                            Err(error) => json!({ "success": false, "error": error }),
                        };
                        let _ = answer.send(result);
                        tui.invalidate();
                        return true;
                    }
                    Asked::GetToolsExpanded => {
                        let expanded = self.effects.tools_expanded.unwrap_or(chat.expanded);
                        let _ = answer.send(json!(expanded));
                        return false;
                    }
                    Asked::ShowOverlay {
                        hosted,
                        options,
                        visible,
                    } => {
                        if self.closing {
                            let _ = answer.send(closed());
                            return false;
                        }
                        self.show_overlay(tui, hosted, options, visible, Some(answer));
                        return false;
                    }
                    Asked::OverlayFocused { component } => {
                        let focused = self
                            .overlays
                            .get(&component)
                            .is_some_and(|overlay| tui.is_overlay_focused(overlay.shown));
                        let _ = answer.send(json!(focused));
                        return false;
                    }
                    Asked::OverlayBounds { component } => {
                        let bounds = self
                            .overlays
                            .get(&component)
                            .and_then(|overlay| tui.overlay_bounds(overlay.shown))
                            .map_or(Value::Null, |bounds| {
                                json!({
                                    "row": bounds.row,
                                    "col": bounds.col,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                })
                            });
                        let _ = answer.send(bounds);
                        return false;
                    }
                    Asked::Custom { .. } if self.closing => {
                        let _ = answer.send(closed());
                        return false;
                    }
                    _ if self.closing => {
                        let _ = answer.send(dismissed);
                        return false;
                    }
                    Asked::Custom { hosted } => {
                        let component = hosted.component;
                        let Some(proxy) = self.host(hosted) else {
                            let _ = answer.send(closed());
                            return false;
                        };
                        let saved = self.surface.editor.borrow().get_text();
                        Open {
                            shown: proxy,
                            waiting: Waiting::Hosted {
                                component,
                                saved,
                                answer,
                            },
                        }
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
                    Asked::Editor { title, prefill } => open(
                        ExtensionEditor::new(&title, prefill.as_deref()),
                        Kind::Editor,
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
            Shown::TerminalInput { enabled } => self.surface.terminal_input.enable(enabled),
            Shown::TerminalInputResult { sequence, verdict } => {
                return self.surface.terminal_input.answer(tui, sequence, verdict);
            }
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
                let shown = lines.map(|lines| Widget {
                    key: key.clone(),
                    shown: widget(&lines),
                    hosted: None,
                });
                self.place_widget(&key, shown, placement.as_deref());
            }
            Shown::SetWidgetComponent {
                key,
                hosted,
                placement,
            } => {
                let component = hosted.component;
                let Some(proxy) = self.host(hosted) else {
                    return false;
                };
                let shown = Widget {
                    key: key.clone(),
                    shown: proxy,
                    hosted: Some(component),
                };
                self.place_widget(&key, Some(shown), placement.as_deref());
            }
            Shown::ComponentLines {
                component,
                width,
                lines,
            } => {
                return self
                    .hosted
                    .get(&component)
                    .is_some_and(|proxy| proxy.borrow_mut().draw(width, lines));
            }
            Shown::CustomClose { component } => {
                self.hosted.remove(&component);
                let index = self.dialogs.iter().position(|open| {
                    matches!(open.waiting, Waiting::Hosted { component: shown, .. } if shown == component)
                });
                let Some(index) = index else {
                    return false;
                };
                if let Waiting::Hosted { saved, answer, .. } = self.dialogs.remove(index).waiting {
                    self.surface.editor.borrow_mut().set_text(&saved);
                    let _ = answer.send(closed());
                }
                self.show_top(tui);
            }
            Shown::ShowOverlay {
                hosted,
                options,
                visible,
            } => self.show_overlay(tui, hosted, options, visible, None),
            Shown::Overlay { component, action } => {
                let Some(overlay) = self.overlays.get_mut(&component) else {
                    return false;
                };
                match action {
                    OverlayAction::Hide => {
                        tui.hide_overlay_id(overlay.shown);
                        self.hosted.remove(&component);
                        if let Some(overlay) = self.overlays.remove(&component) {
                            overlay.closed();
                        }
                    }
                    OverlayAction::SetHidden { hidden } => {
                        tui.set_overlay_hidden(overlay.shown, hidden);
                    }
                    OverlayAction::Focus => tui.focus_overlay(overlay.shown),
                    OverlayAction::Unfocus => tui.unfocus_overlay(overlay.shown, None),
                    OverlayAction::Visible { visible } => {
                        overlay.visible.set(visible);
                        tui.request_render(false);
                    }
                }
            }
            Shown::HideTopOverlay => {
                tui.hide_overlay();
                let gone: Vec<u64> = self
                    .overlays
                    .iter()
                    .filter(|(_, overlay)| !tui.is_overlay_shown(overlay.shown))
                    .map(|(component, _)| *component)
                    .collect();
                for component in gone {
                    self.removed(component);
                }
            }
            Shown::SetFooter { hosted } => {
                if let Some(old) = self.footer.take() {
                    self.hosted.remove(&old);
                }
                let component = hosted.as_ref().map(|wire| wire.component);
                let shown: ComponentRef = match hosted.and_then(|wire| self.host(wire)) {
                    Some(proxy) => {
                        self.footer = component;
                        proxy
                    }
                    None => self.surface.footer.clone(),
                };
                let mut slot = self.surface.footer_slot.borrow_mut();
                slot.clear();
                slot.add_child(shown);
            }
            Shown::SetHeader { hosted } => {
                let current = match self.header.take() {
                    Some((old, proxy)) => {
                        self.hosted.remove(&old);
                        proxy
                    }
                    None => self.surface.built_in_header.clone(),
                };
                let component = hosted.as_ref().map(|wire| wire.component);
                let next: Option<ComponentRef> =
                    match (component, hosted.and_then(|wire| self.host(wire))) {
                        (Some(component), Some(proxy)) => {
                            let proxy: ComponentRef = proxy;
                            self.header = Some((component, proxy.clone()));
                            Some(proxy)
                        }
                        _ => None,
                    };
                let mut slot = self.surface.header_slot.borrow_mut();
                let index = slot.children.iter().position(|shown| same(shown, &current));
                match (index, next) {
                    (Some(index), Some(next)) => slot.children[index] = next,
                    (None, Some(next)) => slot.children.insert(0, next),
                    (Some(index), None) => {
                        slot.children[index] = self.surface.built_in_header.clone()
                    }
                    (None, None) => {}
                }
            }
            Shown::SetTitle { title } => tui.set_title(&title),
            Shown::SetEditorComponent { hosted } => self.swap_editor(tui, hosted),
            Shown::EditorChange { component, text } => {
                if let Some(editor) = self.editor.as_mut().filter(|e| e.component == component) {
                    let local = self.surface.editor.borrow().get_text();
                    if editor.accept(&local, &text) {
                        self.surface.editor.borrow_mut().set_text(&text);
                    }
                }
            }
            Shown::EditorSubmit { component, text } => {
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.component == component)
                {
                    self.effects.submitted.push(text);
                }
            }
            Shown::EditorAction { component, action } => {
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.component == component)
                {
                    self.effects.editor_actions.push(action);
                }
            }
            Shown::EditorInputDone {
                component,
                sequence,
            } => {
                if let Some(editor) = self.editor.as_mut().filter(|e| e.component == component) {
                    editor.done(sequence);
                }
            }
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

    fn removed(&mut self, component: u64) {
        self.hosted.remove(&component);
        if let Some(overlay) = self.overlays.remove(&component) {
            overlay.closed();
        }
        if let Some(live) = self.live {
            live.ui(&ToHost::Removed { component });
        }
    }

    fn show_overlay(
        &mut self,
        tui: &mut Screen,
        hosted: HostedWire,
        options: Option<OverlayWire>,
        visible: bool,
        answer: Option<Sender<Value>>,
    ) {
        let component = hosted.component;
        let Some(proxy) = self.host(hosted) else {
            if let Some(answer) = answer {
                let _ = answer.send(closed());
            }
            return;
        };
        let visible = Rc::new(Cell::new(visible));
        let options = options.unwrap_or_default().options(visible.clone());
        let terminal = tui.terminal();
        let width = overlay_width(&options, terminal.columns(), terminal.rows());
        proxy.borrow_mut().request(width);
        let shown = tui.show_overlay(proxy, Some(options));
        self.overlays.insert(
            component,
            Overlay {
                shown,
                visible,
                answer,
            },
        );
    }

    fn place_widget(&mut self, key: &str, shown: Option<Widget>, placement: Option<&str>) {
        for placed in [&mut self.above, &mut self.below] {
            placed.retain(|widget| {
                let kept = widget.key != key;
                if let (false, Some(hosted)) = (kept, widget.hosted) {
                    self.hosted.remove(&hosted);
                }
                kept
            });
        }
        if let Some(shown) = shown {
            match placement {
                Some("belowEditor") => self.below.push(shown),
                _ => self.above.push(shown),
            }
        }
        self.render_widgets();
    }

    fn render_widgets(&self) {
        let mut above = self.surface.widgets_above.borrow_mut();
        above.clear();
        above.add_child(component(Spacer::new(1)));
        for widget in &self.above {
            above.add_child(widget.shown.clone());
        }
        let mut below = self.surface.widgets_below.borrow_mut();
        below.clear();
        for widget in &self.below {
            below.add_child(widget.shown.clone());
        }
    }

    fn settle(&mut self, tui: &mut Screen, chat: &Chat, now: u64) -> bool {
        let top = self.top();
        let mut drew = false;
        let mut index = 0;
        while index < self.dialogs.len() {
            let Waiting::Answer {
                dialog,
                kind,
                cancel,
                answer,
            } = &self.dialogs[index].waiting
            else {
                index += 1;
                continue;
            };
            let outcome = {
                let mut dialog = dialog.borrow_mut();
                if dialog.take_tools_toggle() {
                    let expanded = self.effects.tools_expanded.unwrap_or(chat.expanded);
                    self.effects.tools_expanded = Some(!expanded);
                }
                drew |= dialog.tick(now);
                if let Some(content) = dialog.take_external() {
                    let command = (self.surface.external_editor)();
                    tui.stop(false);
                    let edited = super::external_editor::edit(&command, &content);
                    tui.start();
                    tui.request_render(true);
                    if let Some(text) = edited {
                        dialog.edited(&text);
                    }
                    drew = true;
                }
                dialog.take_outcome()
            };
            if cancel.is_cancelled() || outcome.is_some() {
                if let Some(outcome) = outcome.filter(|_| !cancel.is_cancelled()) {
                    let _ = answer.send(kind.response(outcome));
                }
                self.dialogs.remove(index);
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
        let shown: Vec<u64> = self.overlays.keys().copied().collect();
        for component in shown {
            self.removed(component);
        }
        if self.dialogs.is_empty() {
            return;
        }
        for open in self.dialogs.drain(..) {
            match open.waiting {
                Waiting::Answer { kind, answer, .. } => {
                    let _ = answer.send(kind.response(Outcome::Cancelled));
                }
                Waiting::Hosted {
                    component, answer, ..
                } => {
                    self.hosted.remove(&component);
                    let _ = answer.send(closed());
                }
            }
        }
        self.show_top(tui);
    }

    fn show_top(&self, tui: &mut Screen) {
        let top: ComponentRef = match self.dialogs.last() {
            Some(open) => open.shown.clone(),
            None => self.surface.active_editor.borrow().clone(),
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
        if self.live.is_none() {
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
