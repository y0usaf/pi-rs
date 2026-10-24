//! The /trust overlay (upstream components/trust-selector.ts): this project, its parent
//! folder, or no trust, saved to the trust store for future sessions.

use pi_tui::keybindings::get_keybindings;
use pi_tui::keys::matches_key;
use pi_tui::tui::Component;

use super::{ActionSlot, OverlayAction, TrustUpdate, border, fit, key_hint, push, raw_key_hint};
use crate::interactive::theme::{bold, fg};
use crate::settings::TrustOption;

pub struct TrustSelector {
    cwd: String,
    options: Vec<TrustOption>,
    saved: Option<(String, bool)>,
    project_trusted: bool,
    selected: usize,
    action: ActionSlot,
}

impl TrustSelector {
    pub fn new(
        cwd: &str,
        options: Vec<TrustOption>,
        saved: Option<(String, bool)>,
        project_trusted: bool,
        action: ActionSlot,
    ) -> Self {
        let selected = options
            .iter()
            .position(|option| Self::is_saved(option, &saved))
            .unwrap_or(0);
        TrustSelector {
            cwd: cwd.to_string(),
            options,
            saved,
            project_trusted,
            selected,
            action,
        }
    }

    fn is_saved(option: &TrustOption, saved: &Option<(String, bool)>) -> bool {
        match (option.saved_path.as_ref(), saved) {
            (Some(path), Some((saved_path, decision))) => {
                path == saved_path && option.trusted == *decision
            }
            _ => false,
        }
    }

    fn format_decision(&self) -> String {
        let Some((path, decision)) = self.saved.as_ref() else {
            return "none".to_string();
        };
        let label = if *decision { "trusted" } else { "untrusted" };
        let own_path = self.options.first().and_then(|o| o.saved_path.as_ref());
        match own_path {
            Some(own) if own != path => format!("{label} (inherited from {path})"),
            _ => format!("{label} ({path})"),
        }
    }

    fn rows(&self) -> Vec<String> {
        self.options
            .iter()
            .enumerate()
            .map(|(i, option)| {
                let is_selected = i == self.selected;
                let current = if Self::is_saved(option, &self.saved) {
                    fg("accent", "✓ ")
                } else {
                    "  ".to_string()
                };
                let prefix = if is_selected {
                    fg("accent", "→ ")
                } else {
                    "  ".to_string()
                };
                let label = if is_selected {
                    fg("accent", &option.label)
                } else {
                    fg("text", &option.label)
                };
                format!(" {prefix}{current}{label}")
            })
            .collect()
    }

    fn choose(&self) {
        let Some(option) = self.options.get(self.selected) else {
            return;
        };
        let updates: Vec<TrustUpdate> = option
            .updates
            .iter()
            .map(|(path, decision)| TrustUpdate {
                path: path.clone(),
                decision: *decision,
            })
            .collect();
        push(
            &self.action,
            OverlayAction::SetTrust {
                updates,
                trusted: option.trusted,
            },
        );
    }
}

impl Component for TrustSelector {
    fn render(&mut self, _width: usize) -> Vec<String> {
        let width = _width;
        let mut lines = vec![
            border(width),
            String::new(),
            format!(" {}", bold(&fg("accent", "Project trust"))),
            format!(" {}", fg("muted", &self.cwd)),
            String::new(),
            format!(
                " {}",
                fg(
                    "muted",
                    &format!("Saved decision: {}", self.format_decision())
                )
            ),
            format!(
                " {}",
                fg(
                    "muted",
                    &format!(
                        "Current session: {}",
                        if self.project_trusted {
                            "trusted"
                        } else {
                            "untrusted"
                        }
                    )
                )
            ),
            String::new(),
        ];
        lines.extend(self.rows());
        lines.push(String::new());
        lines.push(format!(
            " {}  {}  {}",
            raw_key_hint("↑↓", "navigate"),
            key_hint("tui.select.confirm", "save"),
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
            self.selected = (self.selected + 1).min(self.options.len().saturating_sub(1));
        } else if kb.matches(data, "tui.select.confirm") || matches_key(data, "enter") {
            self.choose();
        } else if kb.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Close);
        }
    }
}
