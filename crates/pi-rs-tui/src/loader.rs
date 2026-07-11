//! Loader component state ported from `packages/tui/src/components/loader.ts`.
//!
//! Timing is expressed as explicit [`Loader::advance`] calls so terminal/front-end
//! policy can drive it from Lua while deterministic snapshots remain possible.

use crate::component::{Component, render_text};

pub const DEFAULT_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const DEFAULT_INTERVAL_MS: u64 = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indicator {
    pub frames: Vec<String>,
    pub interval_ms: u64,
}

impl Default for Indicator {
    fn default() -> Self {
        Self {
            frames: DEFAULT_FRAMES
                .iter()
                .map(|frame| (*frame).to_owned())
                .collect(),
            interval_ms: DEFAULT_INTERVAL_MS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Loader {
    frames: Vec<String>,
    interval_ms: u64,
    current_frame: usize,
    elapsed_ms: u64,
    running: bool,
    message: String,
}

impl Loader {
    pub fn new(message: impl Into<String>, indicator: Option<Indicator>) -> Self {
        let indicator = indicator.unwrap_or_default();
        Self {
            frames: indicator.frames,
            interval_ms: if indicator.interval_ms > 0 {
                indicator.interval_ms
            } else {
                DEFAULT_INTERVAL_MS
            },
            current_frame: 0,
            elapsed_ms: 0,
            running: true,
            message: message.into(),
        }
    }

    pub fn start(&mut self) {
        self.running = true;
    }
    pub fn stop(&mut self) {
        self.running = false;
    }
    pub fn running(&self) -> bool {
        self.running
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub fn set_indicator(&mut self, indicator: Option<Indicator>) {
        let indicator = indicator.unwrap_or_default();
        self.frames = indicator.frames;
        self.interval_ms = if indicator.interval_ms > 0 {
            indicator.interval_ms
        } else {
            DEFAULT_INTERVAL_MS
        };
        self.current_frame = 0;
        self.elapsed_ms = 0;
        self.running = true;
    }

    /// Advance the animation clock and return whether the visible frame changed.
    pub fn advance(&mut self, elapsed_ms: u64) -> bool {
        if !self.running || self.frames.len() <= 1 {
            return false;
        }
        self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
        let steps = self.elapsed_ms / self.interval_ms;
        self.elapsed_ms %= self.interval_ms;
        if steps == 0 {
            return false;
        }
        let step = usize::try_from(steps % self.frames.len() as u64).unwrap_or(0);
        self.current_frame = (self.current_frame + step) % self.frames.len();
        true
    }

    pub fn frame(&self) -> &str {
        self.frames
            .get(self.current_frame)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn display_text(&self) -> String {
        let frame = self.frame();
        if frame.is_empty() {
            self.message.clone()
        } else {
            format!("{frame} {}", self.message)
        }
    }
}

impl Component for Loader {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![String::new()];
        lines.extend(render_text(&self.display_text(), width, 1, 0));
        lines
    }
}

#[derive(Clone, Debug)]
pub struct CancellableLoader {
    loader: Loader,
    aborted: bool,
}

impl CancellableLoader {
    pub fn new(message: impl Into<String>, indicator: Option<Indicator>) -> Self {
        Self {
            loader: Loader::new(message, indicator),
            aborted: false,
        }
    }
    pub fn loader(&self) -> &Loader {
        &self.loader
    }
    pub fn loader_mut(&mut self) -> &mut Loader {
        &mut self.loader
    }
    pub fn aborted(&self) -> bool {
        self.aborted
    }
    /// Matches the default `tui.select.cancel` keybinding (Escape).
    pub fn handle_input(&mut self, data: &str) -> bool {
        if data == "\x1b" {
            self.aborted = true;
            true
        } else {
            false
        }
    }
    pub fn dispose(&mut self) {
        self.loader.stop();
    }
}

impl Component for CancellableLoader {
    fn render(&self, width: usize) -> Vec<String> {
        self.loader.render(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_animation_and_snapshot_match_spec() {
        let mut loader = Loader::new("Loading...", None);
        assert_eq!(loader.render(16), ["", " ⠋ Loading...   "]);
        assert!(!loader.advance(79));
        assert!(loader.advance(1));
        assert_eq!(loader.frame(), "⠙");
        assert!(loader.advance(80 * 11));
        assert_eq!(loader.frame(), "⠹");
    }

    #[test]
    fn indicator_edge_cases_and_message_updates_match_spec() {
        let mut loader = Loader::new(
            "Working",
            Some(Indicator {
                frames: vec![],
                interval_ms: 0,
            }),
        );
        assert_eq!(loader.display_text(), "Working");
        assert!(!loader.advance(1000));
        loader.set_indicator(Some(Indicator {
            frames: vec!["x".into()],
            interval_ms: 20,
        }));
        loader.set_message("Done");
        assert_eq!(loader.display_text(), "x Done");
        assert!(!loader.advance(20));
    }

    #[test]
    fn cancellable_loader_aborts_on_escape_and_disposes() {
        let mut loader = CancellableLoader::new("Working...", None);
        assert!(!loader.handle_input("x"));
        assert!(loader.handle_input("\x1b"));
        assert!(loader.aborted());
        loader.dispose();
        assert!(!loader.loader().running());
    }
}
