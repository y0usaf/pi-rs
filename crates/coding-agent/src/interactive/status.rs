//! The startup header and the status line above the editor (`status-indicator.ts`): a spinner
//! with "Working" while a run goes, two blank lines when idle.

use pi_tui::components::loader::Loader;
use pi_tui::tui::Component;

use super::theme::{fg, fg_fn};
use crate::config::VERSION;

/// `piWordmark()`: a coral P and a yellow i.
fn wordmark() -> String {
    "\x1b[38;2;228;138;122mP\x1b[39m\x1b[38;2;234;182;93mi\x1b[39m".to_string()
}

fn hint(key: &str, description: &str) -> String {
    fg("dim", key) + &fg("muted", &format!(" {description}"))
}

/// The header's text, upstream's compact form with the keys this mode handles: the logo, version
/// and key hints, which quietStartup hides, and the details line, which quietStartup "header"
/// hides too.
pub fn header(quiet: crate::settings::QuietStartup) -> Option<String> {
    use crate::settings::QuietStartup;
    if quiet == QuietStartup::On {
        return None;
    }
    let hints = [
        hint("escape", "interrupt"),
        hint("ctrl+c/ctrl+d", "clear/exit"),
        hint("ctrl+o", "expand tools"),
    ]
    .join(&fg("muted", " · "));
    let top = format!(
        "{} {}\n{hints}",
        wordmark(),
        fg("dim", &format!("v{VERSION}"))
    );
    Some(match quiet {
        QuietStartup::Off => format!(
            "{top}\n\n{}",
            fg(
                "dim",
                "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi."
            )
        ),
        _ => top,
    })
}

pub const WORKING: &str = "Working (escape to interrupt)";

/// The working indicator, or the idle status's two blank lines.
pub struct StatusArea {
    loader: Option<Loader>,
    message: String,
}

impl StatusArea {
    pub fn new() -> Self {
        StatusArea {
            loader: None,
            message: WORKING.into(),
        }
    }

    /// Shows the spinner with `message`, or clears it with `None`.
    pub fn set(&mut self, message: Option<&str>) {
        match message {
            None => self.loader = None,
            Some(m) => {
                if self.message != m || self.loader.is_none() {
                    self.message = m.to_string();
                    match &mut self.loader {
                        Some(l) => l.set_message(m),
                        None => {
                            let mut l = Loader::new(fg_fn("accent"), fg_fn("muted"), m, None);
                            l.start();
                            self.loader = Some(l);
                        }
                    }
                }
            }
        }
    }

    /// Advances the spinner; returns (changed, next frame due).
    pub fn tick(&mut self, now: u64) -> (bool, Option<u64>) {
        match &mut self.loader {
            Some(l) => l.tick(now),
            None => (false, None),
        }
    }
}

impl Default for StatusArea {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for StatusArea {
    fn render(&mut self, width: usize) -> Vec<String> {
        match &mut self.loader {
            Some(l) => l.render(width),
            None => vec![" ".repeat(width), " ".repeat(width)],
        }
    }
    fn invalidate(&mut self) {
        if let Some(l) = &mut self.loader {
            l.invalidate();
        }
    }
}
