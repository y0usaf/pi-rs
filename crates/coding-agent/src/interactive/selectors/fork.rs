//! The /fork overlay (upstream `modes/interactive/components/user-message-selector.ts`): the
//! session's user messages, one of them picked to copy the active path up to that point into a
//! new session.

use pi_durable::ids::EntryId;
use pi_tui::keybindings::get_keybindings;
use pi_tui::tui::Component;
use pi_tui::utils::truncate_to_width;

use super::{ActionSlot, OverlayAction, border, fit, push, wrap};
use crate::interactive::theme::{bold, fg};

const MAX_VISIBLE: usize = 10;

pub struct ForkSelector {
    messages: Vec<(EntryId, String)>,
    selected: usize,
    action: ActionSlot,
}

impl ForkSelector {
    pub fn new(
        messages: Vec<(EntryId, String)>,
        initial: Option<EntryId>,
        action: ActionSlot,
    ) -> Self {
        let selected = initial
            .and_then(|id| messages.iter().position(|(entry, _)| *entry == id))
            .unwrap_or_else(|| messages.len().saturating_sub(1));
        Self {
            messages,
            selected,
            action,
        }
    }
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\t'], " ").trim().to_owned()
}

impl Component for ForkSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![String::new(), border(width)];
        lines.push(truncate_to_width(
            &bold(" Fork from Message"),
            width,
            "",
            false,
        ));
        lines.extend(wrap(
            &fg(
                "muted",
                " Select a user message to copy the active path up to that point into a new session",
            ),
            width,
        ));
        lines.push(String::new());
        lines.push(border(width));
        lines.push(String::new());
        let count = self.messages.len();
        if count == 0 {
            lines.push(fg("muted", "  No user messages found"));
        } else {
            let start = self
                .selected
                .saturating_sub(MAX_VISIBLE / 2)
                .min(count.saturating_sub(MAX_VISIBLE));
            let end = (start + MAX_VISIBLE).min(count);
            for position in start..end {
                let (_, text) = &self.messages[position];
                let selected = position == self.selected;
                let cursor = if selected {
                    fg("accent", "\u{203a} ")
                } else {
                    "  ".to_owned()
                };
                let text = truncate_to_width(&one_line(text), width.saturating_sub(2), "", false);
                let text = if selected { bold(&text) } else { text };
                lines.push(format!("{cursor}{text}"));
                lines.push(fg(
                    "muted",
                    &format!("  Message {} of {count}", position + 1),
                ));
                lines.push(String::new());
            }
            if start > 0 || end < count {
                lines.push(fg("muted", &format!("  ({}/{count})", self.selected + 1)));
            }
        }
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
        if self.messages.is_empty() {
            return;
        }
        if kb.matches(data, "tui.select.up") {
            self.selected = if self.selected == 0 {
                self.messages.len() - 1
            } else {
                self.selected - 1
            };
            return;
        }
        if kb.matches(data, "tui.select.down") {
            self.selected = if self.selected == self.messages.len() - 1 {
                0
            } else {
                self.selected + 1
            };
            return;
        }
        if kb.matches(data, "tui.select.confirm") {
            if let Some((id, _)) = self.messages.get(self.selected) {
                push(&self.action, OverlayAction::Fork(*id));
            }
            return;
        }
        if kb.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Close);
        }
    }
}
