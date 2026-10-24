//! Text edits: typing, paste, newline, deletion, kill ring, undo, submit.

use super::text::{paste_marker_at, paste_markers};
use super::{Editor, EditorSnapshot, EditorState, LastAction};
use crate::unicode::{is_cjk_break, utf16_len};
use crate::utils::{is_whitespace_char, js_trim};

/// Line endings to \n, tabs to four spaces.
pub(crate) fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
}

/// Decode CSI-u Ctrl+letter sequences (ESC [ code ; 5 u) back to control bytes.
fn decode_ctrl_csi_u(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(p) = rest.find("\x1b\x5b") {
        out.push_str(&rest[..p]);
        let after = &rest[p + 2..];
        let n = after.bytes().take_while(u8::is_ascii_digit).count();
        let mut done = false;
        if n > 0
            && after[n..].starts_with(";5u")
            && let Ok(cp) = after[..n].parse::<u32>()
        {
            let c = match cp {
                97..=122 => Some(cp - 96),
                65..=90 => Some(cp - 64),
                _ => None,
            };
            if let Some(c) = c.and_then(char::from_u32) {
                out.push(c);
                rest = &after[n + 3..];
                done = true;
            }
        }
        if !done {
            out.push_str(&rest[p..p + 2]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// JS `trimStart`.
pub(crate) fn trim_start(s: &str) -> &str {
    let i = s
        .char_indices()
        .find(|&(i, c)| !js_trim(&s[i..i + c.len_utf8()]).is_empty())
        .map_or(s.len(), |(i, _)| i);
    &s[i..]
}

impl Editor {
    pub(crate) fn push_undo_snapshot(&mut self) {
        let snap = EditorSnapshot {
            state: self.state.clone(),
            pastes: self.pastes.clone(),
            paste_counter: self.paste_counter,
        };
        self.undo_stack.push(&snap);
    }

    pub(crate) fn undo(&mut self) {
        self.exit_history_browsing();
        let Some(s) = self.undo_stack.pop() else {
            return;
        };
        self.state = s.state;
        self.pastes = s.pastes;
        self.paste_counter = s.paste_counter;
        self.last_action = None;
        self.preferred_visual_col = None;
        self.emit_change();
    }

    /// Replace the whole text (undoable), clearing pastes and history browsing.
    pub fn set_text(&mut self, text: &str) {
        self.cancel_autocomplete();
        self.last_action = None;
        self.exit_history_browsing();
        let normalized = normalize_text(text);
        if self.get_text() != normalized {
            self.push_undo_snapshot();
        }
        self.pastes.clear();
        self.paste_counter = 0;
        self.set_text_internal(&normalized, false);
    }

    /// Insert text at the cursor as one undo unit.
    pub fn insert_text_at_cursor(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.cancel_autocomplete();
        self.push_undo_snapshot();
        self.last_action = None;
        self.exit_history_browsing();
        self.insert_text_at_cursor_internal(text);
    }

    pub(crate) fn insert_text_at_cursor_internal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let normalized = normalize_text(text);
        let inserted: Vec<&str> = normalized.split('\n').collect();
        let line = self.current_line().to_string();
        let col = self.state.cursor_col.min(line.len());
        let (before, after) = line.split_at(col);
        let cl = self.state.cursor_line;
        if inserted.len() == 1 {
            self.state.lines[cl] = format!("{before}{normalized}{after}");
            self.set_cursor_col(col + normalized.len());
        } else {
            let last = inserted[inserted.len() - 1];
            let mut new_lines: Vec<String> = self.state.lines[..cl].to_vec();
            new_lines.push(format!("{before}{}", inserted[0]));
            new_lines.extend(
                inserted[1..inserted.len() - 1]
                    .iter()
                    .map(|s| s.to_string()),
            );
            new_lines.push(format!("{last}{after}"));
            new_lines.extend(self.state.lines[cl + 1..].iter().cloned());
            self.state.lines = new_lines;
            self.state.cursor_line += inserted.len() - 1;
            self.set_cursor_col(last.len());
        }
        self.emit_change();
    }

    pub(crate) fn insert_character(&mut self, ch: &str, skip_undo_coalescing: bool) {
        self.exit_history_browsing();
        if !skip_undo_coalescing {
            if is_whitespace_char(ch) || self.last_action != Some(LastAction::TypeWord) {
                self.push_undo_snapshot();
            }
            self.last_action = Some(LastAction::TypeWord);
        }
        let line = self.current_line().to_string();
        let col = self.state.cursor_col;
        let cl = self.state.cursor_line;
        self.state.lines[cl] = format!("{}{ch}{}", &line[..col], &line[col..]);
        self.set_cursor_col(col + ch.len());
        self.emit_change();

        if self.autocomplete_state.is_some() {
            self.update_autocomplete();
            return;
        }
        let before = self.text_before_cursor();
        if ch == "/" && self.is_at_start_of_message() {
            self.try_trigger_autocomplete(false);
        } else if self.autocomplete_trigger_characters.iter().any(|t| t == ch) {
            if self.matches_trigger(&before) {
                self.try_trigger_autocomplete(false);
            }
        } else if ch
            .chars()
            .any(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') || is_cjk_break(c))
            && (self.is_in_slash_command_context(&before) || self.matches_trigger(&before))
        {
            self.try_trigger_autocomplete(false);
        }
    }

    /// After a deletion: refresh an open autocomplete, or reopen it in a completable context.
    fn retrigger_autocomplete(&mut self) {
        if self.autocomplete_state.is_some() {
            self.update_autocomplete();
            return;
        }
        let before = self.text_before_cursor();
        if self.is_in_slash_command_context(&before) || self.matches_trigger(&before) {
            self.try_trigger_autocomplete(false);
        }
    }

    pub(crate) fn handle_paste(&mut self, pasted: &str) {
        self.cancel_autocomplete();
        self.exit_history_browsing();
        self.last_action = None;
        self.push_undo_snapshot();
        let clean = normalize_text(&decode_ctrl_csi_u(pasted));
        let mut filtered: String = clean.chars().filter(|&c| c == '\n' || c >= ' ').collect();
        if filtered.starts_with(['/', '~', '.']) {
            let prev = self.text_before_cursor().chars().last();
            if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                filtered = format!(" {filtered}");
            }
        }
        let line_count = filtered.split('\n').count();
        let total = utf16_len(&filtered);
        if line_count > 10 || total > 1000 {
            self.paste_counter += 1;
            let id = self.paste_counter;
            self.pastes.insert(id, filtered);
            let marker = if line_count > 10 {
                format!("\x5bpaste \x23{id} +{line_count} lines\x5d")
            } else {
                format!("\x5bpaste \x23{id} {total} chars\x5d")
            };
            self.insert_text_at_cursor_internal(&marker);
            return;
        }
        self.insert_text_at_cursor_internal(&filtered);
    }

    pub(crate) fn add_new_line(&mut self) {
        self.cancel_autocomplete();
        self.exit_history_browsing();
        self.last_action = None;
        self.push_undo_snapshot();
        let line = self.current_line().to_string();
        let col = self.state.cursor_col;
        let cl = self.state.cursor_line;
        self.state.lines[cl] = line[..col].to_string();
        self.state.lines.insert(cl + 1, line[col..].to_string());
        self.state.cursor_line += 1;
        self.set_cursor_col(0);
        self.emit_change();
    }

    pub(crate) fn submit_value(&mut self) {
        self.cancel_autocomplete();
        let result = js_trim(&self.get_expanded_text()).to_string();
        self.state = EditorState {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
        };
        self.pastes.clear();
        self.paste_counter = 0;
        self.exit_history_browsing();
        self.scroll_offset = 0;
        self.undo_stack.clear();
        self.last_action = None;
        if let Some(f) = self.on_change.as_mut() {
            f("");
        }
        if let Some(f) = self.on_submit.as_mut() {
            f(&result);
        }
    }

    /// Remove paste `id` and renumber the higher ones, in the registry and in the text.
    fn remove_paste(&mut self, id: u64) {
        self.pastes.remove(&id);
        self.paste_counter = self.paste_counter.saturating_sub(1);
        let higher: Vec<u64> = self.pastes.keys().copied().filter(|&k| k > id).collect();
        for k in higher {
            if let Some(v) = self.pastes.remove(&k) {
                self.pastes.insert(k - 1, v);
            }
        }
        for line in &mut self.state.lines {
            let mut out = String::new();
            let mut i = 0;
            for (s, e, x) in paste_markers(line) {
                if x > id {
                    let digits = x.to_string().len();
                    let head = "\x5bpaste \x23".len();
                    out.push_str(&line[i..s]);
                    out.push_str(&format!("\x5bpaste \x23{}", x - 1));
                    out.push_str(&line[s + head + digits..e]);
                    i = e;
                }
            }
            out.push_str(&line[i..]);
            *line = out;
        }
    }

    pub(crate) fn handle_backspace(&mut self) {
        self.exit_history_browsing();
        self.last_action = None;
        if self.state.cursor_col > 0 {
            self.push_undo_snapshot();
            let before = self.text_before_cursor();
            let segs = self.seg_graphemes(&before);
            let last = segs
                .last()
                .map(|g| g.segment.to_string())
                .unwrap_or_default();
            let glen = if last.is_empty() { 1 } else { last.len() };
            if let Some((len, id)) = paste_marker_at(&last, 0)
                && len == last.len()
            {
                self.remove_paste(id);
            }
            let line = self.current_line().to_string();
            let col = self.state.cursor_col;
            let cl = self.state.cursor_line;
            self.state.lines[cl] = format!("{}{}", &line[..col - glen], &line[col..]);
            self.set_cursor_col(col - glen);
        } else if self.state.cursor_line > 0 {
            self.push_undo_snapshot();
            let cl = self.state.cursor_line;
            let cur = self.state.lines.remove(cl);
            let prev_len = self.state.lines[cl - 1].len();
            self.state.lines[cl - 1].push_str(&cur);
            self.state.cursor_line -= 1;
            self.set_cursor_col(prev_len);
        }
        self.emit_change();
        self.retrigger_autocomplete();
    }

    pub(crate) fn handle_forward_delete(&mut self) {
        self.exit_history_browsing();
        self.last_action = None;
        let line = self.current_line().to_string();
        let col = self.state.cursor_col;
        let cl = self.state.cursor_line;
        if col < line.len() {
            self.push_undo_snapshot();
            let g = self.first_grapheme_len(&line[col..]);
            self.state.lines[cl] = format!("{}{}", &line[..col], &line[col + g..]);
        } else if cl + 1 < self.state.lines.len() {
            self.push_undo_snapshot();
            let next = self.state.lines.remove(cl + 1);
            self.state.lines[cl].push_str(&next);
        }
        self.emit_change();
        self.retrigger_autocomplete();
    }
}
