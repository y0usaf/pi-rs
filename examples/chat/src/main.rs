mod engine;

use std::borrow::Cow;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use engine::{Choice, Command, Line, Session, Update};
use gpui::{
    Animated, Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Context, Div,
    FocusHandle, FontWeight, HighlightStyle, Hsla, KeyDownEvent, Keystroke, Motion, MouseButton,
    Rems, Render, ScrollHandle, SharedString, SpringConfig, Stateful, StyledText, Task,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, ease_out_quint, linear_color_stop,
    linear_gradient, prelude::*, pulsating_between, px, relative, rems, rgb, rgb_to_hsla, size,
    transparent_black,
};
use gpui_platform::application;
use pi_durable::ai::types::ModelThinkingLevel;

const BG: u32 = 0x0f0f12;
const PANEL: u32 = 0x17171c;
const RAISED: u32 = 0x1b1b21;
const USER: u32 = 0x24242c;
const FG: u32 = 0xe9e7e4;
const DIM: u32 = 0x85838e;
const FAINT: u32 = 0x4b4a55;
const ACCENT: u32 = 0xe0a96a;
const RED: u32 = 0xe2777f;
const EDGE: u32 = 0x25252d;
const EDGE_FOCUS: u32 = 0x3b3b47;
const CARET: &str = "▍";
const COLUMN: f32 = 760.;
const FONT_FAMILY: &str = "Inter Variable";
const ZOOMS: [f32; 11] = [0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2., 2.5];
const SETTLE: SpringConfig = SpringConfig::new(170., 26., 1.);
const POP: SpringConfig = SpringConfig::new(420., 34., 1.);
const INK: f32 = 0.45;
const SHOWN: usize = 120;

struct Chat {
    loaded: bool,
    session: Option<u64>,
    lines: Vec<Line>,
    sessions: Vec<Session>,
    models: Vec<Choice>,
    model: Option<Choice>,
    thinking: ModelThinkingLevel,
    error: Option<String>,
    draft: String,
    font: Option<SharedString>,
    commands: mpsc::Sender<Command>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    list: ScrollHandle,
    cascade: Option<Instant>,
    fresh: bool,
    greetings: usize,
    center: Animated<f32>,
    drawer: Animated<f32>,
    drawer_open: bool,
    drawer_at: usize,
    drawer_opens: usize,
    picker: Animated<f32>,
    picker_open: bool,
    query: String,
    picked: usize,
    zoom: f32,
    scale: Animated<f32>,
    toast: Option<Instant>,
    ink: Ink,
    _updates: Task<()>,
}

#[derive(Default)]
struct Ink {
    key: Option<(Option<u64>, usize)>,
    base: usize,
    len: usize,
    marks: Vec<(usize, Instant)>,
}

impl Ink {
    fn track(&mut self, session: Option<u64>, lines: &[Line], now: Instant) {
        let Some((
            at,
            Line::Reply {
                text, live: true, ..
            },
        )) = lines.iter().enumerate().next_back()
        else {
            return;
        };
        let key = Some((session, at));
        if self.key != key || text.len() < self.len {
            *self = Self {
                key,
                base: text.len(),
                len: text.len(),
                marks: Vec::new(),
            };
        } else if text.len() > self.len {
            self.len = text.len();
            self.marks.push((text.len(), now));
        }
    }

    fn text(&self, text: &str, now: Instant) -> (StyledText, bool) {
        let mut start = self.base;
        let mut runs = Vec::new();
        for &(end, at) in &self.marks {
            let age = now.saturating_duration_since(at).as_secs_f32() / INK;
            if age < 1. && start < end && text.is_char_boundary(start) && text.is_char_boundary(end)
            {
                let shown = 1. - (1. - age).powi(3);
                runs.push((start..end, tint(FG, 0.1 + 0.9 * shown)));
            }
            start = end;
        }
        let active = !runs.is_empty();
        (
            StyledText::new(text.to_owned()).with_highlights(runs),
            active,
        )
    }
}

impl Chat {
    fn new(
        font: Option<SharedString>,
        zoom: f32,
        commands: mpsc::Sender<Command>,
        updates: async_channel::Receiver<Update>,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            while let Ok(update) = updates.recv().await {
                let mut batch = vec![update];
                while let Ok(more) = updates.try_recv() {
                    batch.push(more);
                }
                let applied = this.update(cx, |chat, cx| {
                    for update in batch {
                        chat.apply(update);
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        Self {
            loaded: false,
            session: None,
            lines: Vec::new(),
            sessions: Vec::new(),
            models: Vec::new(),
            model: None,
            thinking: ModelThinkingLevel::Off,
            error: None,
            draft: String::new(),
            font,
            commands,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            list: ScrollHandle::new(),
            cascade: None,
            fresh: false,
            greetings: 0,
            center: Animated::new(1., SETTLE),
            drawer: Animated::new(0., POP),
            drawer_open: false,
            drawer_at: 0,
            drawer_opens: 0,
            picker: Animated::new(0., POP),
            picker_open: false,
            query: String::new(),
            picked: 0,
            zoom,
            scale: Animated::new(zoom, SETTLE),
            toast: None,
            ink: Ink::default(),
            _updates: task,
        }
    }

    fn apply(&mut self, update: Update) {
        let now = Instant::now();
        match update {
            Update::Models(models) => self.models = models,
            Update::Sessions(sessions) => self.sessions = sessions,
            Update::Opened(session) => {
                self.session = session;
                self.fresh = true;
                self.ink = Ink::default();
                self.scroll.scroll_to_bottom();
            }
            Update::View(session, lines) => {
                if session != self.session {
                    return;
                }
                if std::mem::take(&mut self.fresh) {
                    self.cascade = (!lines.is_empty()).then_some(now);
                }
                let stick = self.near_bottom();
                self.ink.track(session, &lines, now);
                let empty = lines.is_empty();
                if empty && (!self.loaded || !self.lines.is_empty()) {
                    self.greetings += 1;
                }
                let center = if empty { 1. } else { 0. };
                if self.loaded {
                    self.center.set(center, SETTLE, now);
                } else {
                    self.center.jump_to(center);
                    self.loaded = true;
                }
                self.lines = lines;
                if stick {
                    self.scroll.scroll_to_bottom();
                }
            }
            Update::Agent(choice, thinking) => {
                self.model = Some(choice);
                self.thinking = thinking;
            }
            Update::Failed(error) => {
                self.loaded = true;
                self.error = Some(error);
            }
        }
    }

    fn near_bottom(&self) -> bool {
        -self.scroll.offset().y >= self.scroll.max_offset().y - px(64.)
    }

    fn command(&mut self, command: Command) {
        if self.commands.send(command).is_err() {
            self.error = Some("the store thread has stopped".into());
        }
    }

    fn busy(&self) -> bool {
        matches!(self.lines.last(), Some(Line::Reply { live: true, .. }))
    }

    fn send(&mut self) {
        let prompt = self.draft.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        self.error = None;
        self.command(Command::Send(prompt));
        self.draft.clear();
    }

    fn open(&mut self, session: Option<u64>) {
        self.set_drawer(false);
        if session != self.session || session.is_none() {
            self.error = None;
            self.command(Command::Open(session));
        }
    }

    fn set_drawer(&mut self, open: bool) {
        if open == self.drawer_open {
            return;
        }
        self.drawer_open = open;
        if open {
            self.set_picker(false);
            self.drawer_opens += 1;
            self.drawer_at = self
                .sessions
                .iter()
                .position(|session| Some(session.id) == self.session)
                .map_or(0, |at| at + 1);
        }
        self.drawer
            .set(if open { 1. } else { 0. }, POP, Instant::now());
    }

    fn set_picker(&mut self, open: bool) {
        if open == self.picker_open {
            return;
        }
        self.picker_open = open;
        if open {
            self.set_drawer(false);
            self.query.clear();
            self.picked = self
                .found()
                .iter()
                .position(|choice| Some(*choice) == self.model.as_ref())
                .unwrap_or(0);
            self.list.scroll_to_item(self.picked);
        }
        self.picker
            .set(if open { 1. } else { 0. }, POP, Instant::now());
    }

    fn found(&self) -> Vec<&Choice> {
        let query = self.query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        self.models
            .iter()
            .filter(|choice| {
                let hay =
                    format!("{} {} {}", choice.name, choice.provider, choice.id).to_lowercase();
                words.iter().all(|word| hay.contains(word))
            })
            .take(SHOWN)
            .collect()
    }

    fn choose(&mut self, choice: Choice) {
        self.set_picker(false);
        self.command(Command::Model(choice.clone()));
        self.model = Some(choice);
    }

    fn cycle_thinking(&mut self) {
        let Some(levels) = self.model.as_ref().map(|model| model.levels.clone()) else {
            return;
        };
        if levels.len() < 2 {
            return;
        }
        let at = levels
            .iter()
            .position(|level| *level == self.thinking)
            .unwrap_or(0);
        self.thinking = levels[(at + 1) % levels.len()];
        self.command(Command::Thinking(self.thinking));
    }

    fn set_zoom(&mut self, zoom: f32) {
        let now = Instant::now();
        self.zoom = zoom;
        self.scale.set(zoom, SETTLE, now);
        self.toast = Some(now);
        if let Some(path) = zoom_path()
            && let Err(error) = std::fs::write(&path, format!("{zoom}\n"))
        {
            eprintln!("pi-chat: {}: {error}", path.display());
        }
    }

    fn zoom_by(&mut self, step: i32) {
        let next = if step > 0 {
            ZOOMS.iter().find(|zoom| **zoom > self.zoom + 0.01)
        } else {
            ZOOMS.iter().rev().find(|zoom| **zoom < self.zoom - 0.01)
        };
        if let Some(zoom) = next {
            self.set_zoom(*zoom);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let command = stroke.modifiers.control || stroke.modifiers.platform;
        let handled = (command && self.shortcut(stroke))
            || if self.picker_open {
                self.picker_key(stroke)
            } else if self.drawer_open {
                self.drawer_key(stroke, cx)
            } else {
                self.composer_key(stroke, cx)
            };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn shortcut(&mut self, stroke: &Keystroke) -> bool {
        match stroke.key.as_str() {
            "=" | "+" | "plus" | "equal" => self.zoom_by(1),
            "-" | "_" | "minus" => self.zoom_by(-1),
            "0" => self.set_zoom(1.),
            "n" => self.open(None),
            "b" => self.set_drawer(!self.drawer_open),
            "m" => self.set_picker(!self.picker_open),
            "t" => self.cycle_thinking(),
            _ => return false,
        }
        true
    }

    fn picker_key(&mut self, stroke: &Keystroke) -> bool {
        let count = self.found().len();
        match stroke.key.as_str() {
            "escape" => self.set_picker(false),
            "up" => self.picked = self.picked.saturating_sub(1),
            "down" => self.picked = (self.picked + 1).min(count.saturating_sub(1)),
            "enter" => {
                if let Some(choice) = self
                    .found()
                    .get(self.picked)
                    .map(|choice| (*choice).clone())
                {
                    self.choose(choice);
                }
            }
            "backspace" => {
                self.query.pop();
                self.picked = 0;
            }
            _ => match typed(stroke) {
                Some(text) => {
                    self.query.push_str(text);
                    self.picked = 0;
                }
                None => return false,
            },
        }
        self.list.scroll_to_item(self.picked);
        true
    }

    fn drawer_key(&mut self, stroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        match stroke.key.as_str() {
            "escape" => self.set_drawer(false),
            "up" => self.drawer_at = self.drawer_at.saturating_sub(1),
            "down" => self.drawer_at = (self.drawer_at + 1).min(self.sessions.len()),
            "enter" => {
                let session = match self.drawer_at {
                    0 => None,
                    at => self.sessions.get(at - 1).map(|session| session.id),
                };
                self.open(session);
            }
            _ => {
                self.set_drawer(false);
                return self.composer_key(stroke, cx);
            }
        }
        true
    }

    fn composer_key(&mut self, stroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let held = stroke.modifiers;
        let command = held.control || held.platform;
        match stroke.key.as_str() {
            "escape" if self.busy() => self.command(Command::Stop),
            "enter" if held.shift => self.draft.push('\n'),
            "enter" => self.send(),
            "backspace" if command || held.alt => {
                let kept = self
                    .draft
                    .trim_end()
                    .char_indices()
                    .rev()
                    .find(|(_, letter)| letter.is_whitespace())
                    .map_or(0, |(at, letter)| at + letter.len_utf8());
                self.draft.truncate(kept);
            }
            "backspace" => {
                self.draft.pop();
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.draft.push_str(&text);
                }
            }
            _ => match typed(stroke) {
                Some(text) => self.draft.push_str(text),
                None => return false,
            },
        }
        true
    }
}

fn typed(stroke: &Keystroke) -> Option<&str> {
    let held = stroke.modifiers;
    stroke
        .key_char
        .as_deref()
        .filter(|text| !(held.control || held.platform) && !text.chars().any(char::is_control))
}

impl Render for Chat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let scale = self.scale.sample(now);
        let center = self.center.sample(now);
        let drawer = self.drawer.sample(now);
        let picker = self.picker.sample(now);
        let toast = self
            .toast
            .map(|at| now.saturating_duration_since(at).as_secs_f32())
            .filter(|age| *age < 1.5);
        if scale.is_active
            || center.is_active
            || drawer.is_active
            || picker.is_active
            || toast.is_some()
        {
            window.request_animation_frame();
        }
        window.set_rem_size(px(16. * scale.value));
        let focused = self.focus.is_focused(window);
        let (log, inking) = self.log(now);
        if inking {
            window.request_animation_frame();
        }
        let veil = drawer.value.clamp(0., 1.);
        let main = div()
            .size_full()
            .flex()
            .flex_col()
            .when(veil > 0.002, |main| main.blur(px(7. * veil)))
            .child(log)
            .child(self.composer(focused, picker.value, cx))
            .child(
                div()
                    .flex_basis(relative(0.))
                    .flex_grow(center.value.max(0.)),
            );
        div()
            .relative()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(FG))
            .text_size(u(15.))
            .line_height(u(24.))
            .when_some(self.font.clone(), |root, font| root.font_family(font))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|chat, _, _, cx| {
                    if chat.picker_open {
                        chat.set_picker(false);
                        cx.notify();
                    }
                }),
            )
            .when(self.loaded, |root| {
                root.child(main)
                    .child(div().absolute().top_0().left_0().right_0().h(u(56.)).bg(
                        linear_gradient(
                            180.,
                            linear_color_stop(hsla(BG, 1.), 0.),
                            linear_color_stop(hsla(BG, 0.), 1.),
                        ),
                    ))
                    .child(menu(1. - veil, cx))
                    .when(self.drawer_open || drawer.value > 0.002, |root| {
                        root.child(self.drawer_layer(drawer.value, cx))
                    })
                    .when_some(toast, |root, age| root.child(self.toast(age)))
            })
    }
}

impl Chat {
    fn log(&self, now: Instant) -> (AnyElement, bool) {
        if self.lines.is_empty() {
            let hello = div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .gap(u(10.))
                .child(
                    div()
                        .text_size(u(26.))
                        .line_height(u(32.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Say something."),
                )
                .child(
                    div()
                        .text_size(u(13.))
                        .text_color(rgb(FAINT))
                        .child("Kill this window mid-reply, then open it again."),
                )
                .with_animation(
                    ("hello", self.greetings),
                    Animation::new(Duration::from_millis(900)).with_easing(ease_out_quint()),
                    |hello, shown| hello.opacity(shown).top(u(12. * (1. - shown))),
                );
            let log = div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .pb(u(34.))
                .child(hello)
                .into_any_element();
            return (log, false);
        }
        let count = self.lines.len();
        let stagger = self
            .cascade
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(400));
        let page = SharedString::from(format!("m{}", self.session.unwrap_or_default()));
        let mut inking = false;
        let lines = self.lines.iter().enumerate().map(|(at, line)| {
            let steps = if stagger { 7 - (count - at).min(7) } else { 0 };
            let body = match line {
                Line::User(text) => div().flex().justify_end().child(
                    div()
                        .max_w(relative(0.8))
                        .px(u(15.))
                        .py(u(9.))
                        .rounded(u(18.))
                        .bg(rgb(USER))
                        .child(text.clone()),
                ),
                Line::Reply {
                    text,
                    resumed,
                    stopped,
                    live,
                    thinking,
                } => {
                    let words = if *live && text.is_empty() {
                        if *thinking {
                            shimmer("Thinking")
                        } else {
                            dots()
                        }
                    } else if self.ink.key == Some((self.session, at)) {
                        let (styled, active) = self.ink.text(text, now);
                        inking |= active;
                        styled.into_any_element()
                    } else {
                        div().child(text.clone()).into_any_element()
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap(u(4.))
                        .when(*resumed, |reply| {
                            reply.child(note("↻ resumed after restart", ACCENT))
                        })
                        .child(words)
                        .when(*stopped, |reply| reply.child(note("Stopped", FAINT)))
                }
                Line::Failed(text) => div()
                    .text_size(u(13.))
                    .text_color(rgb(RED))
                    .child(text.clone()),
            };
            body.relative().with_animation(
                (page.clone(), at),
                Animation::new(
                    Motion::new(Duration::from_millis(560))
                        .with_delay(Duration::from_millis(28 * steps as u64)),
                )
                .with_easing(ease_out_quint()),
                |line, shown| line.opacity(shown).top(u(12. * (1. - shown))),
            )
        });
        let log = div()
            .id("log")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .w_full()
                    .max_w(u(COLUMN))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap(u(22.))
                    .px(u(28.))
                    .pt(u(60.))
                    .pb(u(20.))
                    .children(lines.collect::<Vec<_>>()),
            )
            .into_any_element();
        (log, inking)
    }

    fn composer(&self, focused: bool, picker: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let field = match (self.draft.is_empty(), focused) {
            (true, true) => {
                let shown = format!("{CARET}Message");
                StyledText::new(shown.clone())
                    .with_highlights([
                        (0..CARET.len(), tint(ACCENT, 1.)),
                        (CARET.len()..shown.len(), tint(FAINT, 1.)),
                    ])
                    .into_any_element()
            }
            (true, false) => div()
                .text_color(rgb(FAINT))
                .child("Message")
                .into_any_element(),
            (false, true) => {
                let text = &self.draft;
                StyledText::new(format!("{text}{CARET}"))
                    .with_highlights([(text.len()..text.len() + CARET.len(), tint(ACCENT, 1.))])
                    .into_any_element()
            }
            (false, false) => div().child(self.draft.clone()).into_any_element(),
        };
        let busy = self.busy();
        let ready = !self.draft.trim().is_empty();
        let stop = busy && !ready;
        let levels = self.model.as_ref().map_or(0, |model| model.levels.len());
        let model = self.model.as_ref().map_or_else(
            || SharedString::from("…"),
            |model| model.name.clone().into(),
        );
        let send = div()
            .id("send")
            .flex_none()
            .size(u(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(if stop {
                EDGE_FOCUS
            } else if ready {
                ACCENT
            } else {
                EDGE
            }))
            .transitions(|motion| motion.bg(Duration::from_millis(180)))
            .child(if stop {
                div().size(u(10.)).rounded(u(2.5)).bg(rgb(FG))
            } else {
                div()
                    .text_size(u(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if ready { BG } else { FAINT }))
                    .child("↑")
            })
            .on_click(cx.listener(move |chat, _: &ClickEvent, _, cx| {
                if stop {
                    chat.command(Command::Stop);
                } else {
                    chat.send();
                }
                cx.notify();
            }));
        div()
            .relative()
            .flex_none()
            .w_full()
            .max_w(u(COLUMN))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(u(8.))
            .px(u(16.))
            .pb(u(18.))
            .when_some(self.error.clone(), |composer, error| {
                composer.child(
                    div()
                        .px(u(6.))
                        .text_size(u(12.))
                        .text_color(rgb(RED))
                        .child(error),
                )
            })
            .child(
                div()
                    .id("box")
                    .flex()
                    .flex_col()
                    .gap(u(8.))
                    .px(u(16.))
                    .pt(u(13.))
                    .pb(u(9.))
                    .rounded(u(20.))
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(if focused { EDGE_FOCUS } else { EDGE }))
                    .transitions(|motion| motion.border_color(Duration::from_millis(240)))
                    .child(
                        div()
                            .id("field")
                            .max_h(u(220.))
                            .overflow_y_scroll()
                            .child(field),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(u(2.))
                            .ml(u(-8.))
                            .child(
                                chip("model")
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(|chat, _: &ClickEvent, _, cx| {
                                        chat.set_picker(!chat.picker_open);
                                        cx.notify();
                                    }))
                                    .child(model)
                                    .child(div().text_color(rgb(FAINT)).mt(u(-5.)).child("⌄")),
                            )
                            .when(levels > 1, |row| {
                                row.child(
                                    chip("thinking")
                                        .on_click(cx.listener(|chat, _: &ClickEvent, _, cx| {
                                            chat.cycle_thinking();
                                            cx.notify();
                                        }))
                                        .child(self.bars())
                                        .child(level_name(self.thinking)),
                                )
                            })
                            .child(div().flex_1())
                            .child(send),
                    ),
            )
            .when(self.picker_open || picker > 0.002, |composer| {
                composer.child(self.picker_layer(picker, cx))
            })
    }

    fn bars(&self) -> impl IntoElement {
        let levels = self
            .model
            .as_ref()
            .map_or(&[][..], |model| &model.levels[..]);
        let at = levels
            .iter()
            .position(|level| *level == self.thinking)
            .unwrap_or(0) as f32;
        let lit = (4. * at / (levels.len().max(2) - 1) as f32).ceil() as usize;
        div()
            .flex()
            .items_end()
            .gap(u(2.))
            .h(u(11.))
            .children((0..4).map(|bar| {
                div()
                    .id(("bar", bar))
                    .w(u(2.5))
                    .h(u(4. + 2.3 * bar as f32))
                    .rounded(u(1.))
                    .bg(rgb(if bar < lit { ACCENT } else { FAINT }))
                    .transitions(|motion| motion.bg(Duration::from_millis(260)))
            }))
    }

    fn picker_layer(&self, shown: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let found = self.found();
        let query: AnyElement = if self.query.is_empty() {
            StyledText::new(format!("{CARET}Search models"))
                .with_highlights([
                    (0..CARET.len(), tint(ACCENT, 1.)),
                    (CARET.len()..CARET.len() + 13, tint(FAINT, 1.)),
                ])
                .into_any_element()
        } else {
            let text = &self.query;
            StyledText::new(format!("{text}{CARET}"))
                .with_highlights([(text.len()..text.len() + CARET.len(), tint(ACCENT, 1.))])
                .into_any_element()
        };
        let rows = found.iter().enumerate().map(|(at, choice)| {
            let current = Some(*choice) == self.model.as_ref();
            let picked = (*choice).clone();
            row(("pick", at), at == self.picked, false)
                .on_click(cx.listener(move |chat, _: &ClickEvent, _, cx| {
                    chat.choose(picked.clone());
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(rgb(if current { FG } else { DIM }))
                        .child(choice.name.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(u(12.))
                        .text_color(rgb(FAINT))
                        .child(choice.provider.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .w(u(12.))
                        .text_size(u(12.))
                        .text_color(rgb(ACCENT))
                        .when(current, |mark| mark.child("✓")),
                )
        });
        let visible = shown.clamp(0., 1.);
        div()
            .id("picker")
            .occlude()
            .absolute()
            .left(u(16.))
            .bottom(relative(1.))
            .mb(u(6. - 10. * (1. - shown)))
            .w(u(400.))
            .opacity(visible)
            .rounded(u(16.))
            .bg(hsla(RAISED, 0.92))
            .backdrop_blur(px(24.))
            .border_1()
            .border_color(rgb(EDGE))
            .shadow_2xl()
            .overflow_hidden()
            .flex()
            .flex_col()
            .text_size(u(14.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(u(44.))
                    .px(u(16.))
                    .border_b_1()
                    .border_color(rgb(EDGE))
                    .child(query),
            )
            .child(
                div()
                    .id("models")
                    .max_h(u(320.))
                    .overflow_y_scroll()
                    .track_scroll(&self.list)
                    .flex()
                    .flex_col()
                    .p(u(6.))
                    .when(found.is_empty(), |list| {
                        list.child(
                            div()
                                .px(u(10.))
                                .py(u(8.))
                                .text_color(rgb(FAINT))
                                .child("No model matches."),
                        )
                    })
                    .children(rows),
            )
    }

    fn drawer_layer(&self, shown: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = shown.clamp(0., 1.);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        let wave = SharedString::from(format!("r{}", self.drawer_opens));
        let enter = move |at: usize| {
            Animation::new(
                Motion::new(Duration::from_millis(420))
                    .with_delay(Duration::from_millis(24 * at.min(12) as u64)),
            )
            .with_easing(ease_out_quint())
        };
        let fresh = row(
            ("chat", 0usize),
            self.drawer_at == 0,
            self.session.is_none(),
        )
        .on_click(cx.listener(|chat, _: &ClickEvent, _, cx| {
            chat.open(None);
            cx.notify();
        }))
        .child(div().w(u(8.)).text_color(rgb(ACCENT)).child("+"))
        .child(div().flex_1().child("New chat"))
        .child(hint("⌃N"))
        .relative()
        .with_animation((wave.clone(), 0usize), enter(0), |row, shown| {
            row.opacity(shown).left(u(-8. * (1. - shown)))
        });
        let rows = self.sessions.iter().enumerate().map(|(at, session)| {
            let id = session.id;
            let current = Some(id) == self.session;
            row(("chat", at + 1), self.drawer_at == at + 1, current)
                .on_click(cx.listener(move |chat, _: &ClickEvent, _, cx| {
                    chat.open(Some(id));
                    cx.notify();
                }))
                .child(
                    div()
                        .w(u(8.))
                        .flex()
                        .justify_center()
                        .when(session.live, |slot| slot.child(pulse(("live", at)))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(rgb(if current { FG } else { DIM }))
                        .child(session.title.lines().next().unwrap_or_default().to_owned()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(u(11.5))
                        .text_color(rgb(FAINT))
                        .child(ago(session.at, now)),
                )
                .relative()
                .with_animation((wave.clone(), at + 1), enter(at + 1), |row, shown| {
                    row.opacity(shown).left(u(-8. * (1. - shown)))
                })
        });
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("scrim")
                    .absolute()
                    .inset_0()
                    .bg(hsla(0x000000, 0.35 * visible))
                    .occlude()
                    .on_click(cx.listener(|chat, _: &ClickEvent, _, cx| {
                        chat.set_drawer(false);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("drawer")
                    .occlude()
                    .absolute()
                    .top(u(10.))
                    .bottom(u(10.))
                    .left(u(10. - 40. * (1. - shown)))
                    .w(u(300.))
                    .opacity(visible)
                    .rounded(u(18.))
                    .bg(hsla(RAISED, 0.9))
                    .backdrop_blur(px(28.))
                    .border_1()
                    .border_color(rgb(EDGE))
                    .shadow_2xl()
                    .flex()
                    .flex_col()
                    .gap(u(2.))
                    .p(u(8.))
                    .text_size(u(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(u(36.))
                            .px(u(10.))
                            .text_size(u(12.))
                            .text_color(rgb(FAINT))
                            .child("Chats")
                            .child(hint("⌃B")),
                    )
                    .child(fresh)
                    .child(
                        div()
                            .id("chats")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(u(2.))
                            .mt(u(6.))
                            .children(rows.collect::<Vec<_>>()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(u(12.))
                            .gap_y(u(4.))
                            .px(u(10.))
                            .py(u(8.))
                            .text_size(u(11.))
                            .text_color(rgb(FAINT))
                            .child("⌃M model")
                            .child("⌃T thinking")
                            .child("⌃+ ⌃− zoom")
                            .child("Esc stop"),
                    ),
            )
    }

    fn toast(&self, age: f32) -> impl IntoElement {
        let shown = (age / 0.14).min(1.) * ((1.5 - age) / 0.4).clamp(0., 1.);
        div()
            .absolute()
            .top(u(14.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .px(u(12.))
                    .py(u(4.))
                    .rounded_full()
                    .bg(hsla(RAISED, 0.92))
                    .border_1()
                    .border_color(rgb(EDGE))
                    .text_size(u(12.))
                    .text_color(rgb(DIM))
                    .opacity(shown)
                    .child(format!("{:.0}%", self.zoom * 100.)),
            )
    }
}

fn u(pixels: f32) -> Rems {
    rems(pixels / 16.)
}

fn hsla(color: u32, alpha: f32) -> Hsla {
    let mut color = rgb_to_hsla(rgb(color));
    color.alpha *= alpha;
    color
}

fn tint(color: u32, alpha: f32) -> HighlightStyle {
    HighlightStyle {
        color: Some(hsla(color, alpha)),
        ..Default::default()
    }
}

fn menu(shown: f32, cx: &mut Context<Chat>) -> impl IntoElement {
    div()
        .id("menu")
        .opacity(shown)
        .absolute()
        .top(u(12.))
        .left(u(12.))
        .size(u(34.))
        .rounded(u(10.))
        .flex()
        .flex_col()
        .justify_center()
        .gap(u(4.))
        .px(u(10.))
        .cursor_pointer()
        .hover_style(|style| style.bg(rgb(EDGE)))
        .transitions(|motion| motion.bg(Duration::from_millis(160)))
        .child(div().w(u(14.)).h(u(2.)).rounded_full().bg(rgb(DIM)))
        .child(div().w(u(9.)).h(u(2.)).rounded_full().bg(rgb(DIM)))
        .on_click(cx.listener(|chat, _: &ClickEvent, _, cx| {
            chat.set_drawer(!chat.drawer_open);
            cx.notify();
        }))
}

fn chip(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(u(7.))
        .h(u(28.))
        .px(u(8.))
        .rounded(u(9.))
        .text_size(u(12.5))
        .text_color(rgb(DIM))
        .cursor_pointer()
        .hover_style(|style| style.bg(rgb(EDGE)).text_color(rgb(FG)))
        .transitions(|motion| {
            motion
                .bg(Duration::from_millis(140))
                .text_color(Duration::from_millis(140))
        })
}

fn row(id: (&'static str, usize), selected: bool, current: bool) -> Stateful<Div> {
    let bg = if selected {
        rgb_to_hsla(rgb(USER))
    } else if current {
        hsla(USER, 0.55)
    } else {
        transparent_black()
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(u(10.))
        .h(u(36.))
        .px(u(10.))
        .rounded(u(10.))
        .cursor_pointer()
        .bg(bg)
        .hover_style(|style| style.bg(rgb(EDGE)))
        .transitions(|motion| motion.bg(Duration::from_millis(140)))
}

fn hint(keys: &'static str) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(u(11.))
        .text_color(rgb(FAINT))
        .child(keys)
}

fn note(text: &'static str, color: u32) -> impl IntoElement {
    div().text_size(u(12.)).text_color(rgb(color)).child(text)
}

fn pulse(id: (&'static str, usize)) -> impl IntoElement {
    div()
        .size(u(6.))
        .rounded_full()
        .bg(rgb(ACCENT))
        .with_animation(
            id,
            Animation::new(Duration::from_millis(1400))
                .repeat()
                .with_easing(pulsating_between(0.3, 1.)),
            |dot, shown| dot.opacity(shown),
        )
}

fn dots() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(u(5.))
        .h(u(24.))
        .children((0..3usize).map(|at| {
            div()
                .relative()
                .size(u(6.))
                .rounded_full()
                .bg(rgb(DIM))
                .with_animation(
                    ("dot", at),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    move |dot, phase| {
                        let lift = ((phase - at as f32 * 0.14).rem_euclid(1.) * TAU)
                            .sin()
                            .max(0.);
                        dot.opacity(0.3 + 0.7 * lift).top(u(-3. * lift))
                    },
                )
        }))
        .into_any_element()
}

fn shimmer(label: &'static str) -> AnyElement {
    let count = label.chars().count() as f32;
    StyledText::new(label)
        .with_animation(
            "shimmer",
            Animation::new(Duration::from_millis(1800)).repeat(),
            move |text, phase| {
                let sweep = phase * 1.8 - 0.4;
                text.with_highlights(label.char_indices().enumerate().map(
                    |(at, (from, letter))| {
                        let place = (at as f32 + 0.5) / count;
                        let glow = (1. - (place - sweep).abs() / 0.25).max(0.);
                        (from..from + letter.len_utf8(), tint(FG, 0.4 + 0.6 * glow))
                    },
                ))
            },
        )
        .into_any_element()
}

fn level_name(level: ModelThinkingLevel) -> &'static str {
    match level {
        ModelThinkingLevel::Off => "No thinking",
        ModelThinkingLevel::Minimal => "Minimal",
        ModelThinkingLevel::Low => "Low",
        ModelThinkingLevel::Medium => "Medium",
        ModelThinkingLevel::High => "High",
        ModelThinkingLevel::Xhigh => "Extra high",
        ModelThinkingLevel::Max => "Max",
    }
}

fn ago(at: u64, now: u64) -> String {
    match now.saturating_sub(at) / 1000 {
        0..60 => "now".into(),
        seconds @ 60..3_600 => format!("{}m", seconds / 60),
        seconds @ 3_600..86_400 => format!("{}h", seconds / 3_600),
        seconds @ 86_400..604_800 => format!("{}d", seconds / 86_400),
        seconds @ 604_800..2_592_000 => format!("{}w", seconds / 604_800),
        seconds @ 2_592_000..31_536_000 => format!("{}mo", seconds / 2_592_000),
        seconds => format!("{}y", seconds / 31_536_000),
    }
}

fn load_font(cx: &App) -> Option<SharedString> {
    let path = option_env!("PI_CHAT_FONT")?;
    let loaded = std::fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            cx.text_system()
                .add_fonts(vec![Cow::Owned(bytes)])
                .map_err(|error| error.to_string())
        });
    match loaded {
        Ok(()) => Some(FONT_FAMILY.into()),
        Err(error) => {
            eprintln!("pi-chat: {path}: {error}");
            None
        }
    }
}

fn state_dir() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|state| !state.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    let dir = state.join("pi-chat");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn zoom_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("zoom"))
}

fn saved_zoom() -> f32 {
    zoom_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| text.trim().parse::<f32>().ok())
        .filter(|zoom| (ZOOMS[0]..=ZOOMS[ZOOMS.len() - 1]).contains(zoom))
        .unwrap_or(1.)
}

fn db_path() -> Result<PathBuf, String> {
    match std::env::args_os().nth(1) {
        Some(path) => Ok(PathBuf::from(path)),
        None => state_dir()
            .map(|dir| dir.join("chat.db"))
            .ok_or_else(|| "set XDG_STATE_HOME or HOME, or pass a database path".into()),
    }
}

fn main() {
    let setup = db_path().and_then(|db| {
        let model = std::env::var("PI_CHAT_MODEL")
            .ok()
            .map(|spec| engine::model_ref(&spec))
            .transpose()?;
        Ok((db, model))
    });
    let (db, model) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            eprintln!("pi-chat: {error}");
            std::process::exit(2);
        }
    };
    let zoom = saved_zoom();
    let (commands, updates) = engine::start(&db, model);
    application().run(move |cx: &mut App| {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let font = load_font(cx);
        let mut wanted = size(px(760. * zoom), px(800. * zoom));
        if let Some(display) = cx.primary_display() {
            let room = display.bounds().size;
            wanted.width = wanted.width.min(room.width * 0.9);
            wanted.height = wanted.height.min(room.height * 0.9);
        }
        let bounds = Bounds::centered(None, wanted, cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("pi chat".into()),
                    ..Default::default()
                }),
                app_id: Some("pi-chat".into()),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let chat = Chat::new(font, zoom, commands, updates, cx);
                    chat.focus.focus(window, cx);
                    chat
                })
            },
        );
        if let Err(error) = opened {
            eprintln!("pi-chat: {error}");
            cx.quit();
        }
        cx.activate(true);
    });
}
