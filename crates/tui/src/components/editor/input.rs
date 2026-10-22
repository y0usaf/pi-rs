//! Key dispatch (upstream `handleInput`).

use super::{Editor, Jump};
use crate::keybindings::get_keybindings;
use crate::keys::{decode_printable_key, matches_key};

const PASTE_START: &str = "\x1b\x5b200~";
const PASTE_END: &str = "\x1b\x5b201~";

fn first_code(data: &str) -> u32 {
    data.encode_utf16().next().map_or(0, u32::from)
}

impl Editor {
    pub(crate) fn handle_key(&mut self, data: &str) {
        let kb = get_keybindings();
        let mut data = data.to_string();

        if let Some(dir) = self.jump_mode {
            if kb.matches(&data, "tui.editor.jumpForward") || kb.matches(&data, "tui.editor.jumpBackward") {
                self.jump_mode = None;
                return;
            }
            let printable = decode_printable_key(&data).or_else(|| (first_code(&data) >= 32).then(|| data.clone()));
            self.jump_mode = None;
            if let Some(p) = printable {
                self.jump_to_char(&p, dir);
                return;
            }
        }

        if data.contains(PASTE_START) {
            self.is_in_paste = true;
            self.paste_buffer.clear();
            data = data.replacen(PASTE_START, "", 1);
        }
        if self.is_in_paste {
            self.paste_buffer.push_str(&data);
            if let Some(end) = self.paste_buffer.find(PASTE_END) {
                let content = self.paste_buffer[..end].to_string();
                let remaining = self.paste_buffer[end + PASTE_END.len()..].to_string();
                if !content.is_empty() {
                    self.handle_paste(&content);
                }
                self.is_in_paste = false;
                self.paste_buffer.clear();
                if !remaining.is_empty() {
                    self.handle_key(&remaining);
                }
            }
            return;
        }

        let m = |id: &str| kb.matches(&data, id);
        if m("tui.input.copy") {
            return;
        }
        if m("tui.editor.undo") {
            self.undo();
            return;
        }

        if self.autocomplete_state.is_some() && self.autocomplete_list.is_some() {
            if m("tui.select.cancel") {
                self.cancel_autocomplete();
                return;
            }
            if m("tui.select.up") || m("tui.select.down") {
                if let Some(l) = self.autocomplete_list.as_mut() {
                    crate::tui::Component::handle_input(l, &data);
                }
                self.apply_picked();
                return;
            }
            if m("tui.input.tab") {
                if let Some(item) = self.selected_autocomplete_item() {
                    if self.autocomplete_provider.is_some() {
                        self.push_undo_snapshot();
                        self.last_action = None;
                        let prefix = self.autocomplete_prefix.clone();
                        self.apply_completion(&item, &prefix);
                        self.cancel_autocomplete();
                        self.emit_change();
                    }
                }
                return;
            }
            if m("tui.select.confirm") {
                if let Some(item) = self.selected_autocomplete_item() {
                    if self.autocomplete_provider.is_some() {
                        self.push_undo_snapshot();
                        self.last_action = None;
                        let prefix = self.autocomplete_prefix.clone();
                        self.apply_completion(&item, &prefix);
                        self.cancel_autocomplete();
                        if !prefix.starts_with('/') {
                            self.emit_change();
                            return;
                        }
                        // Slash commands fall through to submit.
                    }
                }
            }
        }

        if m("tui.input.tab") && self.autocomplete_state.is_none() {
            self.handle_tab_completion();
            return;
        }

        if m("tui.editor.deleteToLineEnd") {
            return self.delete_to_end_of_line();
        }
        if m("tui.editor.deleteToLineStart") {
            return self.delete_to_start_of_line();
        }
        if m("tui.editor.deleteWordBackward") {
            return self.delete_word_backwards();
        }
        if m("tui.editor.deleteWordForward") {
            return self.delete_word_forward();
        }
        if m("tui.editor.deleteCharBackward") || matches_key(&data, "shift+backspace") {
            return self.handle_backspace();
        }
        if m("tui.editor.deleteCharForward") || matches_key(&data, "shift+delete") {
            return self.handle_forward_delete();
        }
        if m("tui.editor.yank") {
            return self.yank();
        }
        if m("tui.editor.yankPop") {
            return self.yank_pop();
        }
        if m("tui.editor.historyPrevious") {
            self.cancel_autocomplete();
            return self.navigate_history(-1);
        }
        if m("tui.editor.historyNext") {
            self.cancel_autocomplete();
            return self.navigate_history(1);
        }
        if m("tui.editor.cursorLineStart") {
            return self.move_to_line_start();
        }
        if m("tui.editor.cursorLineEnd") {
            return self.move_to_line_end();
        }
        if m("tui.editor.cursorWordLeft") {
            return self.move_word_backwards();
        }
        if m("tui.editor.cursorWordRight") {
            return self.move_word_forwards();
        }

        let utf16_len = data.encode_utf16().count();
        if m("tui.input.newLine")
            || (first_code(&data) == 10 && utf16_len > 1)
            || data == "\x1b\r"
            || data == "\x1b\x5b13;2~"
            || (utf16_len > 1 && data.contains('\x1b') && data.contains('\r'))
            || data == "\n"
        {
            if self.should_submit_on_backslash_enter(&data) {
                self.handle_backspace();
                self.submit_value();
                return;
            }
            return self.add_new_line();
        }

        if m("tui.input.submit") {
            if self.disable_submit {
                return;
            }
            if self.char_before_cursor_is_backslash() {
                self.handle_backspace();
                self.add_new_line();
                return;
            }
            return self.submit_value();
        }

        if m("tui.editor.cursorUp") {
            let first = self.is_on_first_visual_line();
            if first && (self.is_editor_empty() || self.history_index > -1 || self.state.cursor_col == 0) {
                self.navigate_history(-1);
            } else if first {
                self.move_to_line_start();
            } else {
                self.move_cursor(-1, 0);
            }
            return;
        }
        if m("tui.editor.cursorDown") {
            let last = self.is_on_last_visual_line();
            if self.history_index > -1 && last {
                self.navigate_history(1);
            } else if last {
                self.move_to_line_end();
            } else {
                self.move_cursor(1, 0);
            }
            return;
        }
        if m("tui.editor.cursorRight") {
            return self.move_cursor(0, 1);
        }
        if m("tui.editor.cursorLeft") {
            return self.move_cursor(0, -1);
        }
        if m("tui.editor.pageUp") {
            return self.page_scroll(-1);
        }
        if m("tui.editor.pageDown") {
            return self.page_scroll(1);
        }
        if m("tui.editor.jumpForward") {
            self.jump_mode = Some(Jump::Forward);
            return;
        }
        if m("tui.editor.jumpBackward") {
            self.jump_mode = Some(Jump::Backward);
            return;
        }
        if matches_key(&data, "shift+space") {
            return self.insert_character(" ", false);
        }
        if let Some(p) = decode_printable_key(&data) {
            return self.insert_character(&p, false);
        }
        if first_code(&data) >= 32 {
            self.insert_character(&data, false);
        }
    }

    fn char_before_cursor_is_backslash(&self) -> bool {
        self.state.cursor_col > 0 && self.text_before_cursor().ends_with('\\')
    }

    fn should_submit_on_backslash_enter(&self, data: &str) -> bool {
        if self.disable_submit || !matches_key(data, "enter") {
            return false;
        }
        let keys = get_keybindings().keys("tui.input.submit");
        let shift_enter = keys.iter().any(|k| k == "shift+enter" || k == "shift+return");
        shift_enter && self.char_before_cursor_is_backslash()
    }
}
