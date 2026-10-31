use std::cell::RefCell;
use std::rc::Rc;

use pi_tui::components::editor::{Editor, EditorOptions};
use pi_tui::components::input::Input;
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::keybindings::get_keybindings;
use pi_tui::tui::{Component, ComponentRef, Container, component};

use super::{border, key_hint, raw_key_hint};
use crate::interactive::theme::{self, bold, fg};

struct Rule;

impl Component for Rule {
    fn render(&mut self, width: usize) -> Vec<String> {
        vec![border(width)]
    }
}

fn dialog_layout(title: ComponentRef, body: ComponentRef, hint: &str) -> Container {
    let mut layout = Container::new();
    layout.add_child(component(Rule));
    layout.add_child(component(Spacer::new(1)));
    layout.add_child(title);
    layout.add_child(component(Spacer::new(1)));
    layout.add_child(body);
    layout.add_child(component(Spacer::new(1)));
    layout.add_child(component(Text::new(hint, 1, 0)));
    layout.add_child(component(Spacer::new(1)));
    layout.add_child(component(Rule));
    layout
}

struct Countdown {
    remaining: u64,
    next: u64,
}

impl Countdown {
    fn start(timeout_ms: Option<f64>, now: u64) -> Option<Countdown> {
        let seconds = timeout_ms.filter(|ms| ms.is_finite() && *ms > 0.0)? / 1000.0;
        Some(Countdown {
            remaining: seconds.ceil() as u64,
            next: now + 1000,
        })
    }

    fn tick(&mut self, now: u64) -> bool {
        if now < self.next {
            return false;
        }
        self.remaining = self.remaining.saturating_sub(1);
        self.next += 1000;
        true
    }
}

fn counted(title: &str, countdown: Option<&Countdown>) -> String {
    match countdown {
        Some(countdown) => format!("{title} ({}s)", countdown.remaining),
        None => title.to_owned(),
    }
}

pub enum Outcome {
    Chosen(String),
    Cancelled,
}

pub trait ExtensionDialog: Component {
    fn take_outcome(&mut self) -> Option<Outcome>;

    fn take_tools_toggle(&mut self) -> bool {
        false
    }

    fn tick(&mut self, now: u64) -> bool;

    fn due(&self) -> Option<u64>;

    fn take_external(&mut self) -> Option<String> {
        None
    }

    fn edited(&mut self, _text: &str) {}
}

struct Timed {
    title: Rc<RefCell<Text>>,
    base: String,
    style: fn(&str) -> String,
    countdown: Option<Countdown>,
}

impl Timed {
    fn new(title: &str, style: fn(&str) -> String, timeout_ms: Option<f64>, now: u64) -> Timed {
        let countdown = Countdown::start(timeout_ms, now);
        Timed {
            title: component(Text::new(&style(&counted(title, countdown.as_ref())), 1, 0)),
            base: title.to_owned(),
            style,
            countdown,
        }
    }

    fn tick(&mut self, now: u64, outcome: &mut Option<Outcome>) -> bool {
        let Some(countdown) = self.countdown.as_mut() else {
            return false;
        };
        if !countdown.tick(now) {
            return false;
        }
        let shown = (self.style)(&counted(&self.base, Some(countdown)));
        self.title.borrow_mut().set_text(&shown);
        if countdown.remaining == 0 {
            self.countdown = None;
            outcome.get_or_insert(Outcome::Cancelled);
        }
        true
    }

    fn due(&self) -> Option<u64> {
        self.countdown.as_ref().map(|countdown| countdown.next)
    }
}

fn selector_title(title: &str) -> String {
    fg("accent", &bold(title))
}

fn input_title(title: &str) -> String {
    fg("accent", title)
}

pub struct ExtensionSelector {
    options: Vec<String>,
    selected: usize,
    list: Rc<RefCell<Container>>,
    layout: Container,
    timed: Timed,
    outcome: Option<Outcome>,
    toggles_tools: bool,
}

impl ExtensionSelector {
    pub fn new(title: &str, options: Vec<String>, timeout_ms: Option<f64>, now: u64) -> Self {
        let timed = Timed::new(title, selector_title, timeout_ms, now);
        let list = component(Container::new());
        let hint = format!(
            "{}  {}  {}",
            raw_key_hint("↑↓", "navigate"),
            key_hint("tui.select.confirm", "select"),
            key_hint("tui.select.cancel", "cancel")
        );
        let layout = dialog_layout(timed.title.clone(), list.clone(), &hint);
        let mut selector = ExtensionSelector {
            options,
            selected: 0,
            list,
            layout,
            timed,
            outcome: None,
            toggles_tools: false,
        };
        selector.update_list();
        selector
    }

    fn update_list(&mut self) {
        let mut list = self.list.borrow_mut();
        list.clear();
        for (index, option) in self.options.iter().enumerate() {
            let text = if index == self.selected {
                fg("accent", "→ ") + &fg("accent", option)
            } else {
                format!("  {}", fg("text", option))
            };
            list.add_child(component(Text::new(&text, 1, 0)));
        }
    }
}

impl Component for ExtensionSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.layout.render(width)
    }

    fn invalidate(&mut self) {
        self.layout.invalidate();
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "app.tools.expand") {
            self.toggles_tools = true;
        } else if kb.matches(data, "tui.select.up") || data == "k" {
            self.selected = self.selected.saturating_sub(1);
            self.update_list();
        } else if kb.matches(data, "tui.select.down") || data == "j" {
            self.selected = (self.selected + 1).min(self.options.len().saturating_sub(1));
            self.update_list();
        } else if kb.matches(data, "tui.select.confirm") || data == "\n" {
            if let Some(option) = self.options.get(self.selected).filter(|o| !o.is_empty()) {
                self.outcome = Some(Outcome::Chosen(option.clone()));
            }
        } else if kb.matches(data, "tui.select.cancel") {
            self.outcome = Some(Outcome::Cancelled);
        }
    }
}

impl ExtensionDialog for ExtensionSelector {
    fn take_outcome(&mut self) -> Option<Outcome> {
        self.outcome.take()
    }

    fn take_tools_toggle(&mut self) -> bool {
        std::mem::take(&mut self.toggles_tools)
    }

    fn tick(&mut self, now: u64) -> bool {
        self.timed.tick(now, &mut self.outcome)
    }

    fn due(&self) -> Option<u64> {
        self.timed.due()
    }
}

pub struct ExtensionInput {
    input: Rc<RefCell<Input>>,
    layout: Container,
    timed: Timed,
    outcome: Option<Outcome>,
}

impl ExtensionInput {
    pub fn new(title: &str, timeout_ms: Option<f64>, now: u64) -> Self {
        let timed = Timed::new(title, input_title, timeout_ms, now);
        let input = component(Input::default());
        let hint = format!(
            "{}  {}",
            key_hint("tui.select.confirm", "submit"),
            key_hint("tui.select.cancel", "cancel")
        );
        let layout = dialog_layout(timed.title.clone(), input.clone(), &hint);
        ExtensionInput {
            input,
            layout,
            timed,
            outcome: None,
        }
    }
}

impl Component for ExtensionInput {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.layout.render(width)
    }

    fn invalidate(&mut self) {
        self.layout.invalidate();
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.borrow_mut().set_focused(focused);
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.confirm") || data == "\n" {
            self.outcome = Some(Outcome::Chosen(self.input.borrow().value().to_owned()));
        } else if kb.matches(data, "tui.select.cancel") {
            self.outcome = Some(Outcome::Cancelled);
        } else {
            self.input.borrow_mut().handle_input(data);
        }
    }
}

impl ExtensionDialog for ExtensionInput {
    fn take_outcome(&mut self) -> Option<Outcome> {
        self.outcome.take()
    }

    fn tick(&mut self, now: u64) -> bool {
        self.timed.tick(now, &mut self.outcome)
    }

    fn due(&self) -> Option<u64> {
        self.timed.due()
    }
}

pub struct ExtensionEditor {
    editor: Rc<RefCell<Editor>>,
    layout: Container,
    submitted: Rc<RefCell<Option<String>>>,
    outcome: Option<Outcome>,
    external: bool,
}

impl ExtensionEditor {
    pub fn new(title: &str, prefill: Option<&str>) -> Self {
        let submitted = Rc::new(RefCell::new(None));
        let mut editor = Editor::new(theme::editor(), EditorOptions::default());
        if let Some(prefill) = prefill.filter(|prefill| !prefill.is_empty()) {
            editor.set_text(prefill);
        }
        let sink = submitted.clone();
        editor.on_submit = Some(Box::new(move |text: &str| {
            *sink.borrow_mut() = Some(text.to_owned());
        }));
        let editor = component(editor);
        let hint = format!(
            "{}  {}  {}  {}",
            key_hint("tui.select.confirm", "submit"),
            key_hint("tui.input.newLine", "newline"),
            key_hint("tui.select.cancel", "cancel"),
            key_hint("app.editor.external", "external editor")
        );
        let title = component(Text::new(&fg("accent", title), 1, 0));
        let layout = dialog_layout(title, editor.clone(), &hint);
        ExtensionEditor {
            editor,
            layout,
            submitted,
            outcome: None,
            external: false,
        }
    }
}

impl Component for ExtensionEditor {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.layout.render(width)
    }

    fn invalidate(&mut self) {
        self.layout.invalidate();
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.editor.borrow_mut().set_focused(focused);
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.cancel") {
            self.outcome = Some(Outcome::Cancelled);
        } else if kb.matches(data, "app.editor.external") {
            self.external = true;
        } else {
            self.editor.borrow_mut().handle_input(data);
            if let Some(text) = self.submitted.borrow_mut().take() {
                self.outcome = Some(Outcome::Chosen(text));
            }
        }
    }
}

impl ExtensionDialog for ExtensionEditor {
    fn take_outcome(&mut self) -> Option<Outcome> {
        self.outcome.take()
    }

    fn tick(&mut self, _now: u64) -> bool {
        false
    }

    fn due(&self) -> Option<u64> {
        None
    }

    fn take_external(&mut self) -> Option<String> {
        std::mem::take(&mut self.external).then(|| self.editor.borrow().get_text())
    }

    fn edited(&mut self, text: &str) {
        self.editor.borrow_mut().set_text(text);
    }
}
