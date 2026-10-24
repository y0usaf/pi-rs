//! The yes/no dialog `/import` asks before it replaces the session (upstream
//! `showExtensionConfirm` in `modes/interactive/interactive-mode.ts`).

use pi_tui::keybindings::get_keybindings;
use pi_tui::keys::matches_key;
use pi_tui::tui::Component;

use super::{ActionSlot, OverlayAction, border, fit, key_hint, push, raw_key_hint};
use crate::interactive::theme::{bold, fg};

pub struct Confirm {
    title: String,
    message: String,
    options: [&'static str; 2],
    selected: usize,
    action: ActionSlot,
}

impl Confirm {
    pub fn new(title: &str, message: &str, action: ActionSlot) -> Self {
        Confirm {
            title: title.to_owned(),
            message: message.to_owned(),
            options: ["Yes", "No"],
            selected: 0,
            action,
        }
    }

    fn rows(&self) -> Vec<String> {
        self.options
            .iter()
            .enumerate()
            .map(|(i, option)| {
                let selected = i == self.selected;
                let prefix = if selected {
                    fg("accent", "→ ")
                } else {
                    "  ".to_string()
                };
                let label = if selected {
                    fg("accent", option)
                } else {
                    fg("text", option)
                };
                format!(" {prefix}{label}")
            })
            .collect()
    }
}

impl Component for Confirm {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![
            border(width),
            String::new(),
            format!(" {}", bold(&fg("accent", &self.title))),
            format!(" {}", fg("muted", &self.message)),
            String::new(),
        ];
        lines.extend(self.rows());
        lines.push(String::new());
        lines.push(format!(
            " {}  {}  {}",
            raw_key_hint("↑↓", "navigate"),
            key_hint("tui.select.confirm", "confirm"),
            key_hint("tui.select.cancel", "cancel")
        ));
        lines.push(String::new());
        lines.push(border(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.up") || matches_key(data, "k") {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.matches(data, "tui.select.down") || matches_key(data, "j") {
            self.selected = (self.selected + 1).min(self.options.len() - 1);
        } else if kb.matches(data, "tui.select.confirm") || matches_key(data, "enter") {
            push(&self.action, OverlayAction::Confirmed(self.selected == 0));
        } else if kb.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Confirmed(false));
        }
    }
}
