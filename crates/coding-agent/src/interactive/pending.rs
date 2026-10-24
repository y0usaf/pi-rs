//! The queued steering and follow-up messages above the status line
//! (`updatePendingMessagesDisplay`): one dim line each, then the key that puts them back in the
//! editor.

use pi_tui::tui::Component;

use super::slash_commands;
use super::theme::fg;

pub struct PendingMessages {
    rows: Vec<String>,
}

impl PendingMessages {
    pub fn new() -> Self {
        PendingMessages { rows: Vec::new() }
    }

    /// Replaces the lines with `steering` and `follow_up`'s messages; true when the text changed.
    pub fn set(&mut self, steering: &[String], follow_up: &[String]) -> bool {
        let mut rows = Vec::new();
        if !steering.is_empty() || !follow_up.is_empty() {
            rows.push(String::new());
            rows.extend(
                steering
                    .iter()
                    .map(|text| fg("dim", &format!("Steering: {text}"))),
            );
            rows.extend(
                follow_up
                    .iter()
                    .map(|text| fg("dim", &format!("Follow-up: {text}"))),
            );
            let key = slash_commands::bound_key("app.message.dequeue");
            rows.push(fg("dim", &format!("↳ {key} to edit all queued messages")));
        }
        let changed = rows != self.rows;
        self.rows = rows;
        changed
    }
}

impl Default for PendingMessages {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for PendingMessages {
    fn render(&mut self, width: usize) -> Vec<String> {
        let available = width.saturating_sub(2).max(1);
        self.rows
            .iter()
            .map(|row| {
                let first = row.split('\n').next().unwrap_or_default();
                let text = pi_tui::utils::truncate_to_width(first, available, "...", false);
                let line = format!(" {text} ");
                let visible = pi_tui::utils::visible_width(&line);
                format!("{line}{}", " ".repeat(width.saturating_sub(visible)))
            })
            .collect()
    }
}
