//! Emacs-style kills and yanks.

use super::{Editor, LastAction};

impl Editor {
    fn kill_push(&mut self, text: &str, prepend: bool, accumulate: bool) {
        self.kill_ring.push(text, prepend, accumulate);
        self.last_action = Some(LastAction::Kill);
    }

    fn was_kill(&self) -> bool {
        self.last_action == Some(LastAction::Kill)
    }

    /// Join line `i` with the line after it.
    fn join_with_next(&mut self, i: usize) {
        let next = self.state.lines.remove(i + 1);
        self.state.lines[i].push_str(&next);
    }

    pub(crate) fn delete_to_start_of_line(&mut self) {
        self.exit_history_browsing();
        let line = self.current_line().to_string();
        let col = self.state.cursor_col;
        let cl = self.state.cursor_line;
        if col > 0 {
            self.push_undo_snapshot();
            let acc = self.was_kill();
            self.kill_push(&line[..col], true, acc);
            self.state.lines[cl] = line[col..].to_string();
            self.set_cursor_col(0);
        } else if cl > 0 {
            self.push_undo_snapshot();
            let acc = self.was_kill();
            self.kill_push("\n", true, acc);
            let prev_len = self.state.lines[cl - 1].len();
            self.join_with_next(cl - 1);
            self.state.cursor_line -= 1;
            self.set_cursor_col(prev_len);
        }
        self.emit_change();
    }

    pub(crate) fn delete_to_end_of_line(&mut self) {
        self.exit_history_browsing();
        let line = self.current_line().to_string();
        let col = self.state.cursor_col;
        let cl = self.state.cursor_line;
        if col < line.len() {
            self.push_undo_snapshot();
            let acc = self.was_kill();
            self.kill_push(&line[col..], false, acc);
            self.state.lines[cl] = line[..col].to_string();
        } else if cl + 1 < self.state.lines.len() {
            self.push_undo_snapshot();
            let acc = self.was_kill();
            self.kill_push("\n", false, acc);
            self.join_with_next(cl);
        }
        self.emit_change();
    }

    pub(crate) fn delete_word_backwards(&mut self) {
        self.exit_history_browsing();
        let line = self.current_line().to_string();
        let cl = self.state.cursor_line;
        if self.state.cursor_col == 0 {
            if cl > 0 {
                self.push_undo_snapshot();
                let acc = self.was_kill();
                self.kill_push("\n", true, acc);
                let prev_len = self.state.lines[cl - 1].len();
                self.join_with_next(cl - 1);
                self.state.cursor_line -= 1;
                self.set_cursor_col(prev_len);
            }
        } else {
            self.push_undo_snapshot();
            let acc = self.was_kill();
            let old = self.state.cursor_col;
            self.move_word_backwards();
            let from = self.state.cursor_col;
            self.kill_push(&line[from..old], true, acc);
            self.state.lines[cl] = format!("{}{}", &line[..from], &line[old..]);
            self.set_cursor_col(from);
        }
        self.emit_change();
    }

    pub(crate) fn delete_word_forward(&mut self) {
        self.exit_history_browsing();
        let line = self.current_line().to_string();
        let cl = self.state.cursor_line;
        let col = self.state.cursor_col;
        if col >= line.len() {
            if cl + 1 < self.state.lines.len() {
                self.push_undo_snapshot();
                let acc = self.was_kill();
                self.kill_push("\n", false, acc);
                self.join_with_next(cl);
            }
        } else {
            self.push_undo_snapshot();
            let acc = self.was_kill();
            self.move_word_forwards();
            let to = self.state.cursor_col;
            self.set_cursor_col(col);
            self.kill_push(&line[col..to], false, acc);
            self.state.lines[cl] = format!("{}{}", &line[..col], &line[to..]);
        }
        self.emit_change();
    }

    pub(crate) fn yank(&mut self) {
        let Some(text) = self.kill_ring.peek().map(str::to_string) else { return };
        self.push_undo_snapshot();
        self.insert_yanked_text(&text);
        self.last_action = Some(LastAction::Yank);
    }

    pub(crate) fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo_snapshot();
        self.delete_yanked_text();
        self.kill_ring.rotate();
        let text = self.kill_ring.peek().unwrap_or_default().to_string();
        self.insert_yanked_text(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn insert_yanked_text(&mut self, text: &str) {
        self.exit_history_browsing();
        let parts: Vec<&str> = text.split('\n').collect();
        let line = self.current_line().to_string();
        let col = self.state.cursor_col;
        let cl = self.state.cursor_line;
        let (before, after) = line.split_at(col);
        if parts.len() == 1 {
            self.state.lines[cl] = format!("{before}{text}{after}");
            self.set_cursor_col(col + text.len());
        } else {
            self.state.lines[cl] = format!("{before}{}", parts[0]);
            for (i, p) in parts[1..parts.len() - 1].iter().enumerate() {
                self.state.lines.insert(cl + 1 + i, p.to_string());
            }
            let last_i = cl + parts.len() - 1;
            let last = parts[parts.len() - 1];
            self.state.lines.insert(last_i, format!("{last}{after}"));
            self.state.cursor_line = last_i;
            self.set_cursor_col(last.len());
        }
        self.emit_change();
    }

    fn delete_yanked_text(&mut self) {
        let Some(y) = self.kill_ring.peek().map(str::to_string) else { return };
        if y.is_empty() {
            return;
        }
        let parts: Vec<&str> = y.split('\n').collect();
        let cl = self.state.cursor_line;
        let col = self.state.cursor_col;
        if parts.len() == 1 {
            let line = self.current_line().to_string();
            let from = col.saturating_sub(y.len());
            self.state.lines[cl] = format!("{}{}", &line[..from], &line[col..]);
            self.set_cursor_col(from);
        } else {
            let start = cl + 1 - parts.len();
            let start_col = self.state.lines[start].len() - parts[0].len();
            let after = self.state.lines[cl][col..].to_string();
            let merged = format!("{}{after}", &self.state.lines[start][..start_col]);
            self.state.lines.splice(start..start + parts.len(), [merged]);
            self.state.cursor_line = start;
            self.set_cursor_col(start_col);
        }
        self.emit_change();
    }
}
