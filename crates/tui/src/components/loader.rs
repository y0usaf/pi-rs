//! A spinner with a message (upstream `components/loader.ts` and `cancellable-loader.ts`).
//!
//! Upstream animates with `setInterval` and asks the TUI to render. Here the owner calls
//! [`Loader::tick`] with the current time; it advances the frame when due and returns `true`
//! when the text changed and a render should be requested.

use super::text::{StyleFn, Text};
use crate::keybindings::get_keybindings;
use crate::tui::Component;

pub const DEFAULT_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const DEFAULT_INTERVAL_MS: u64 = 80;

/// Custom indicator: frames (empty hides it) and interval.
#[derive(Debug, Clone, Default)]
pub struct LoaderIndicatorOptions {
    pub frames: Option<Vec<String>>,
    pub interval_ms: Option<u64>,
}

pub struct Loader {
    text: Text,
    frames: Vec<String>,
    interval_ms: u64,
    current: usize,
    running: bool,
    next_at: Option<u64>,
    verbatim: bool,
    spinner_color: StyleFn,
    message_color: StyleFn,
    message: String,
}

impl Loader {
    pub fn new(spinner_color: StyleFn, message_color: StyleFn, message: &str, indicator: Option<LoaderIndicatorOptions>) -> Self {
        let mut l = Loader {
            text: Text::new("", 1, 0),
            frames: Vec::new(),
            interval_ms: DEFAULT_INTERVAL_MS,
            current: 0,
            running: false,
            next_at: None,
            verbatim: false,
            spinner_color,
            message_color,
            message: message.into(),
        };
        l.set_indicator(indicator);
        l
    }

    /// Start animating (frame changes happen in [`Loader::tick`]).
    pub fn start(&mut self) {
        self.update_display();
        self.running = self.frames.len() > 1;
        self.next_at = None;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.next_at = None;
    }

    pub fn set_message(&mut self, message: &str) {
        self.message = message.into();
        self.update_display();
    }

    pub fn set_indicator(&mut self, indicator: Option<LoaderIndicatorOptions>) {
        self.verbatim = indicator.is_some();
        let ind = indicator.unwrap_or_default();
        self.frames = ind.frames.unwrap_or_else(|| DEFAULT_FRAMES.iter().map(|s| s.to_string()).collect());
        self.interval_ms = ind.interval_ms.filter(|&i| i > 0).unwrap_or(DEFAULT_INTERVAL_MS);
        self.current = 0;
        self.start();
    }

    /// Advance the animation at time `now`. Returns `(changed, next_due)`.
    pub fn tick(&mut self, now: u64) -> (bool, Option<u64>) {
        if !self.running {
            return (false, None);
        }
        let due = *self.next_at.get_or_insert(now + self.interval_ms);
        if now < due {
            return (false, Some(due));
        }
        self.current = (self.current + 1) % self.frames.len();
        self.next_at = Some(now + self.interval_ms);
        self.update_display();
        (true, self.next_at)
    }

    fn rendered_indicator(&self) -> String {
        let frame = self.frames.get(self.current).map_or("", |s| s);
        if self.verbatim { frame.to_string() } else { (self.spinner_color)(frame) }
    }

    fn update_display(&mut self) {
        let f = self.rendered_indicator();
        let ind = if f.is_empty() { String::new() } else { format!("{f} ") };
        let text = format!("{ind}{}", (self.message_color)(&self.message));
        self.text.set_text(&text);
    }
}

impl Component for Loader {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut out = vec![String::new()];
        out.extend(self.text.render(width));
        out
    }
    fn invalidate(&mut self) {
        self.text.invalidate();
        self.update_display();
    }
}

/// A loader the user can cancel with the select-cancel key (Escape).
pub struct CancellableLoader {
    pub loader: Loader,
    aborted: bool,
    pub on_abort: Option<Box<dyn FnMut()>>,
}

impl CancellableLoader {
    pub fn new(loader: Loader) -> Self {
        CancellableLoader { loader, aborted: false, on_abort: None }
    }
    pub fn aborted(&self) -> bool {
        self.aborted
    }
    pub fn dispose(&mut self) {
        self.loader.stop();
    }
}

impl Component for CancellableLoader {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.loader.render(width)
    }
    fn invalidate(&mut self) {
        self.loader.invalidate();
    }
    fn accepts_input(&self) -> bool {
        true
    }
    fn handle_input(&mut self, data: &str) {
        if get_keybindings().matches(data, "tui.select.cancel") {
            self.aborted = true;
            if let Some(f) = self.on_abort.as_mut() {
                f();
            }
        }
    }
}
