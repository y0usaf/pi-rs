mod engine;

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use engine::Line;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, Context, FocusHandle, HighlightStyle,
    KeyDownEvent, Render, ScrollHandle, SharedString, StyledText, Task, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, prelude::*, pulsating_between, px, relative, rgb,
    rgb_to_hsla, size,
};
use gpui_platform::application;

const BG: u32 = 0x0f0f12;
const PANEL: u32 = 0x17171c;
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

struct Chat {
    lines: Vec<Line>,
    error: Option<String>,
    draft: String,
    label: SharedString,
    font: Option<SharedString>,
    prompts: mpsc::Sender<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _updates: Task<()>,
}

impl Chat {
    fn new(
        label: SharedString,
        font: Option<SharedString>,
        prompts: mpsc::Sender<String>,
        updates: async_channel::Receiver<engine::Update>,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            while let Ok(mut update) = updates.recv().await {
                while let Ok(newer) = updates.try_recv() {
                    update = newer;
                }
                let applied = this.update(cx, |chat, cx| {
                    match update {
                        Ok(lines) => chat.lines = lines,
                        Err(error) => chat.error = Some(error),
                    }
                    chat.scroll.scroll_to_bottom();
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        Self {
            lines: Vec::new(),
            error: None,
            draft: String::new(),
            label,
            font,
            prompts,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _updates: task,
        }
    }

    fn key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let held = stroke.modifiers;
        let command = held.control || held.platform;
        match stroke.key.as_str() {
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
            _ => match stroke.key_char.as_deref() {
                Some(typed) if !command && !typed.chars().any(char::is_control) => {
                    self.draft.push_str(typed)
                }
                _ => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn send(&mut self) {
        let prompt = self.draft.trim();
        if prompt.is_empty() {
            return;
        }
        match self.prompts.send(prompt.to_owned()) {
            Ok(()) => self.draft.clear(),
            Err(_) => self.error = Some("the store thread has stopped".into()),
        }
    }

    fn busy(&self) -> bool {
        matches!(self.lines.last(), Some(Line::Reply { live: true, .. }))
    }
}

impl Render for Chat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(FG))
            .text_size(px(15.))
            .line_height(px(24.))
            .when_some(self.font.clone(), |root, font| root.font_family(font))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .child(header(&self.label, self.busy()))
            .child(if self.lines.is_empty() {
                empty()
            } else {
                div()
                    .id("log")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .w_full()
                            .max_w(px(COLUMN))
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap(px(22.))
                            .px(px(28.))
                            .pt(px(8.))
                            .pb(px(20.))
                            .children(self.lines.iter().map(line)),
                    )
                    .into_any_element()
            })
            .child(composer(&self.draft, self.error.as_deref(), focused))
    }
}

fn header(label: &SharedString, busy: bool) -> impl IntoElement {
    let dot = div().size(px(7.)).rounded_full();
    let dot = if busy {
        dot.bg(rgb(ACCENT))
            .with_animation(
                "busy",
                Animation::new(Duration::from_millis(1100))
                    .repeat()
                    .with_easing(pulsating_between(0.3, 1.)),
                |dot, delta| dot.opacity(delta),
            )
            .into_any_element()
    } else {
        dot.bg(rgb(FAINT)).into_any_element()
    };
    div()
        .flex_none()
        .h(px(44.))
        .px(px(20.))
        .flex()
        .items_center()
        .justify_between()
        .text_xs()
        .text_color(rgb(DIM))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(dot)
                .child("pi-durable"),
        )
        .child(label.clone())
}

fn empty() -> AnyElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .child(div().text_color(rgb(DIM)).child("Say something."))
        .child(
            div()
                .text_xs()
                .text_color(rgb(FAINT))
                .child("Kill this window mid-reply, then open it again."),
        )
        .into_any_element()
}

fn line(line: &Line) -> AnyElement {
    match line {
        Line::User(text) => div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(relative(0.78))
                    .px(px(15.))
                    .py(px(9.))
                    .rounded(px(16.))
                    .bg(rgb(USER))
                    .child(text.clone()),
            )
            .into_any_element(),
        Line::Reply {
            text,
            resumed,
            live,
        } => div()
            .flex()
            .flex_col()
            .gap_1()
            .when(*resumed, |reply| {
                reply.child(
                    div()
                        .text_xs()
                        .text_color(rgb(ACCENT))
                        .child("↻ resumed after restart"),
                )
            })
            .child(match (*live, text.is_empty()) {
                (true, true) => div()
                    .text_color(rgb(DIM))
                    .child("•••")
                    .with_animation(
                        "thinking",
                        Animation::new(Duration::from_millis(1100))
                            .repeat()
                            .with_easing(pulsating_between(0.25, 1.)),
                        |dots, delta| dots.opacity(delta),
                    )
                    .into_any_element(),
                (true, false) => with_caret(text, FG).into_any_element(),
                (false, _) => div().child(text.clone()).into_any_element(),
            })
            .into_any_element(),
        Line::Failed(text) => div()
            .text_sm()
            .text_color(rgb(RED))
            .child(text.clone())
            .into_any_element(),
    }
}

fn with_caret(text: &str, color: u32) -> StyledText {
    let caret = text.len()..text.len() + CARET.len();
    StyledText::new(format!("{text}{CARET}"))
        .with_highlights([(0..text.len(), tint(color)), (caret, tint(ACCENT))])
}

fn tint(color: u32) -> HighlightStyle {
    HighlightStyle {
        color: Some(rgb_to_hsla(rgb(color))),
        ..Default::default()
    }
}

fn composer(draft: &str, error: Option<&str>, focused: bool) -> impl IntoElement {
    let field = match (draft.is_empty(), focused) {
        (true, true) => {
            let shown = format!("{CARET}Message");
            StyledText::new(shown.clone())
                .with_highlights([
                    (0..CARET.len(), tint(ACCENT)),
                    (CARET.len()..shown.len(), tint(FAINT)),
                ])
                .into_any_element()
        }
        (true, false) => div()
            .text_color(rgb(FAINT))
            .child("Message")
            .into_any_element(),
        (false, true) => with_caret(draft, FG).into_any_element(),
        (false, false) => div().child(draft.to_owned()).into_any_element(),
    };
    div()
        .flex_none()
        .w_full()
        .max_w(px(COLUMN))
        .mx_auto()
        .flex()
        .flex_col()
        .gap_2()
        .px(px(16.))
        .pb(px(16.))
        .when_some(error, |composer, error| {
            composer.child(
                div()
                    .px(px(4.))
                    .text_xs()
                    .text_color(rgb(RED))
                    .child(error.to_owned()),
            )
        })
        .child(
            div()
                .flex()
                .items_end()
                .gap_3()
                .px(px(16.))
                .py(px(11.))
                .rounded(px(14.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(if focused { EDGE_FOCUS } else { EDGE }))
                .child(div().flex_1().min_w(px(0.)).child(field))
                .child(div().text_xs().text_color(rgb(FAINT)).child("⏎")),
        )
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

fn db_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::args_os().nth(1) {
        return Ok(PathBuf::from(path));
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|state| !state.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or("set XDG_STATE_HOME or HOME, or pass a database path")?;
    let dir = state.join("pi-chat");
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    Ok(dir.join("chat.db"))
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
    let name = model.as_ref().map_or_else(
        || "faux".to_owned(),
        |model| format!("{}/{}", model.provider, model.model_id),
    );
    let file = db.file_name().map_or_else(
        || db.display().to_string(),
        |file| file.to_string_lossy().into_owned(),
    );
    let label = SharedString::from(format!("{name} · {file}"));
    let (prompts, updates) = engine::start(&db, model);
    application().run(move |cx: &mut App| {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let font = load_font(cx);
        let bounds = Bounds::centered(None, size(px(720.), px(760.)), cx);
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
                    let chat = Chat::new(label, font, prompts, updates, cx);
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
