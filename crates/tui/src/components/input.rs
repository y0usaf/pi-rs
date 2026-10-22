//! Single-line text input with horizontal scrolling (upstream `components/input.ts`).
//! The cursor is a byte offset into the value.

use crate::keybindings::get_keybindings;
use crate::keys::decode_kitty_printable;
use crate::kill_ring::{KillRing, UndoStack};
use crate::tui::{Component, CURSOR_MARKER, MouseButton, MouseEventType, TuiMouseEvent, TuiMouseEventResult, render_fake_cursor};
use crate::unicode::{grapheme_indices, graphemes};
use crate::utils::{is_whitespace_char, slice_by_column, truncate_to_width, visible_width};
use crate::word_navigation::{WordNavigationOptions, find_word_backward, find_word_forward};
use std::rc::Rc;

#[derive(Clone, PartialEq, Eq, Debug)]
struct InputState {
    value: String,
    cursor: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

/// Prompt, placeholder and its style.
#[derive(Clone, Default)]
pub struct InputOptions {
    pub prompt: Option<String>,
    pub placeholder: Option<String>,
    pub placeholder_style: Option<Rc<dyn Fn(&str) -> String>>,
}

pub struct Input {
    value: String,
    cursor: usize,
    prompt: String,
    placeholder: String,
    placeholder_style: Rc<dyn Fn(&str) -> String>,
    rendered_start_column: usize,
    pub on_submit: Option<Box<dyn FnMut(&str)>>,
    pub on_escape: Option<Box<dyn FnMut()>>,
    pub focused: bool,
    paste_buffer: String,
    in_paste: bool,
    kill_ring: KillRing,
    last_action: Option<LastAction>,
    undo: UndoStack<InputState>,
}

fn last_grapheme_len(s: &str) -> usize {
    graphemes(s).last().map_or(1, str::len)
}

fn first_grapheme_len(s: &str) -> usize {
    graphemes(s).next().map_or(1, str::len)
}

impl Default for Input {
    fn default() -> Self {
        Input::new(InputOptions::default())
    }
}

impl Input {
    pub fn new(o: InputOptions) -> Self {
        Input {
            value: String::new(),
            cursor: 0,
            prompt: o.prompt.unwrap_or_else(|| "> ".into()),
            placeholder: o.placeholder.unwrap_or_default(),
            placeholder_style: o.placeholder_style.unwrap_or_else(|| Rc::new(|s: &str| s.to_string())),
            rendered_start_column: 0,
            on_submit: None,
            on_escape: None,
            focused: false,
            paste_buffer: String::new(),
            in_paste: false,
            kill_ring: KillRing::default(),
            last_action: None,
            undo: UndoStack::default(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_value(&mut self, v: &str) {
        self.value = v.into();
        self.cursor = self.cursor.min(v.len());
        while !self.value.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    fn push_undo(&mut self) {
        self.undo.push(&InputState { value: self.value.clone(), cursor: self.cursor });
    }

    fn insert(&mut self, text: &str) {
        self.value.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn insert_character(&mut self, ch: &str) {
        if is_whitespace_char(ch) || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);
        self.insert(ch);
    }

    fn backspace(&mut self) {
        self.last_action = None;
        if self.cursor > 0 {
            self.push_undo();
            let n = last_grapheme_len(&self.value[..self.cursor]);
            self.value.replace_range(self.cursor - n..self.cursor, "");
            self.cursor -= n;
        }
    }

    fn forward_delete(&mut self) {
        self.last_action = None;
        if self.cursor < self.value.len() {
            self.push_undo();
            let n = first_grapheme_len(&self.value[self.cursor..]);
            self.value.replace_range(self.cursor..self.cursor + n, "");
        }
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let deleted = self.value[..self.cursor].to_string();
        self.kill_ring.push(&deleted, true, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
        self.value.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    fn delete_to_line_end(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let deleted = self.value[self.cursor..].to_string();
        self.kill_ring.push(&deleted, false, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
        self.value.truncate(self.cursor);
    }

    fn delete_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let old = self.cursor;
        self.move_word_backwards();
        let from = self.cursor;
        self.cursor = old;
        let deleted = self.value[from..old].to_string();
        self.kill_ring.push(&deleted, true, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.value.replace_range(from..old, "");
        self.cursor = from;
    }

    fn delete_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let old = self.cursor;
        self.move_word_forwards();
        let to = self.cursor;
        self.cursor = old;
        let deleted = self.value[old..to].to_string();
        self.kill_ring.push(&deleted, false, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.value.replace_range(old..to, "");
    }

    fn yank(&mut self) {
        let Some(text) = self.kill_ring.peek().map(str::to_string) else { return };
        self.push_undo();
        self.insert(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
        let prev = self.kill_ring.peek().unwrap_or("").len();
        self.value.replace_range(self.cursor - prev..self.cursor, "");
        self.cursor -= prev;
        self.kill_ring.rotate();
        let text = self.kill_ring.peek().unwrap_or("").to_string();
        self.insert(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn do_undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.value = s.value;
            self.cursor = s.cursor;
            self.last_action = None;
        }
    }

    fn move_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.last_action = None;
        self.cursor = find_word_backward(&self.value, self.cursor, &WordNavigationOptions::default());
    }

    fn move_word_forwards(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.last_action = None;
        self.cursor = find_word_forward(&self.value, self.cursor, &WordNavigationOptions::default());
    }

    fn handle_paste(&mut self, text: &str) {
        self.last_action = None;
        self.push_undo();
        let clean = text.replace("\r\n", "").replace('\r', "").replace('\n', "").replace('\t', "    ");
        self.insert(&clean);
    }
}

const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

impl Component for Input {
    fn accepts_input(&self) -> bool {
        true
    }
    fn is_focusable(&self) -> bool {
        true
    }
    fn set_focused(&mut self, f: bool) {
        self.focused = f;
    }

    fn handle_input(&mut self, data: &str) {
        let mut data = data.to_string();
        if data.contains(PASTE_START) {
            self.in_paste = true;
            self.paste_buffer.clear();
            data = data.replacen(PASTE_START, "", 1);
        }
        if self.in_paste {
            self.paste_buffer.push_str(&data);
            if let Some(end) = self.paste_buffer.find(PASTE_END) {
                let content = self.paste_buffer[..end].to_string();
                self.handle_paste(&content);
                self.in_paste = false;
                let remaining = self.paste_buffer[end + PASTE_END.len()..].to_string();
                self.paste_buffer.clear();
                if !remaining.is_empty() {
                    self.handle_input(&remaining);
                }
            }
            return;
        }
        let kb = get_keybindings();
        let m = |id: &str| kb.matches(&data, id);
        if m("tui.select.cancel") {
            if let Some(f) = self.on_escape.as_mut() {
                f();
            }
            return;
        }
        if m("tui.editor.undo") {
            return self.do_undo();
        }
        if m("tui.input.submit") || data == "\n" {
            if let Some(f) = self.on_submit.as_mut() {
                f(&self.value);
            }
            return;
        }
        if m("tui.editor.deleteCharBackward") {
            return self.backspace();
        }
        if m("tui.editor.deleteCharForward") {
            return self.forward_delete();
        }
        if m("tui.editor.deleteWordBackward") {
            return self.delete_word_backwards();
        }
        if m("tui.editor.deleteWordForward") {
            return self.delete_word_forward();
        }
        if m("tui.editor.deleteToLineStart") {
            return self.delete_to_line_start();
        }
        if m("tui.editor.deleteToLineEnd") {
            return self.delete_to_line_end();
        }
        if m("tui.editor.yank") {
            return self.yank();
        }
        if m("tui.editor.yankPop") {
            return self.yank_pop();
        }
        if m("tui.editor.cursorLeft") {
            self.last_action = None;
            if self.cursor > 0 {
                self.cursor -= last_grapheme_len(&self.value[..self.cursor]);
            }
            return;
        }
        if m("tui.editor.cursorRight") {
            self.last_action = None;
            if self.cursor < self.value.len() {
                self.cursor += first_grapheme_len(&self.value[self.cursor..]);
            }
            return;
        }
        if m("tui.editor.cursorLineStart") {
            self.last_action = None;
            self.cursor = 0;
            return;
        }
        if m("tui.editor.cursorLineEnd") {
            self.last_action = None;
            self.cursor = self.value.len();
            return;
        }
        if m("tui.editor.cursorWordLeft") {
            return self.move_word_backwards();
        }
        if m("tui.editor.cursorWordRight") {
            return self.move_word_forwards();
        }
        if let Some(p) = decode_kitty_printable(&data) {
            return self.insert_character(&p);
        }
        let control = data.chars().any(|c| {
            let u = c as u32;
            u < 32 || u == 0x7f || (0x80..=0x9f).contains(&u)
        });
        if !control {
            self.insert_character(&data);
        }
    }

    fn handle_mouse(&mut self, e: &TuiMouseEvent) -> Option<TuiMouseEventResult> {
        if e.kind != MouseEventType::Press || e.button != MouseButton::Left || e.y != 0 {
            return None;
        }
        let target = self.rendered_start_column + (e.x - 2).max(0) as usize;
        let mut col = 0;
        self.cursor = self.value.len();
        for (i, g) in grapheme_indices(&self.value) {
            let next = col + visible_width(g);
            if target < next {
                self.cursor = i;
                break;
            }
            col = next;
        }
        self.last_action = None;
        Some(TuiMouseEventResult { handled: true, focus: true, ..Default::default() })
    }

    fn render(&mut self, width: usize) -> Vec<String> {
        let pw = visible_width(&self.prompt);
        if width <= pw {
            return vec![truncate_to_width(&self.prompt, width, "", false)];
        }
        let avail = width - pw;
        let marker = if self.focused { CURSOR_MARKER } else { "" };
        if self.value.is_empty() && !self.placeholder.is_empty() {
            let ph = truncate_to_width(&self.placeholder, avail, "", false);
            let at = graphemes(&ph).next().unwrap_or(" ").to_string();
            let after = ph.get(at.len()..).unwrap_or("");
            let text = format!("{marker}{}{}", render_fake_cursor(&(self.placeholder_style)(&at)), (self.placeholder_style)(after));
            let pad = " ".repeat(avail.saturating_sub(visible_width(&text)));
            return vec![format!("{}{text}{pad}", self.prompt)];
        }
        let mut visible = String::new();
        let mut cursor_display = self.cursor;
        self.rendered_start_column = 0;
        let total = visible_width(&self.value);
        if total < avail {
            visible = self.value.clone();
        } else {
            let scroll = if self.cursor == self.value.len() { avail - 1 } else { avail };
            let cursor_col = visible_width(&self.value[..self.cursor]);
            if scroll > 0 {
                let half = scroll / 2;
                let start = if cursor_col < half {
                    0
                } else if cursor_col as i64 > total as i64 - half as i64 {
                    total.saturating_sub(scroll)
                } else {
                    cursor_col.saturating_sub(half)
                };
                self.rendered_start_column = start;
                visible = slice_by_column(&self.value, start, scroll, true);
                cursor_display = slice_by_column(&self.value, start, cursor_col.saturating_sub(start), true).len();
            } else {
                cursor_display = 0;
            }
        }
        let rest = &visible[cursor_display..];
        let at = graphemes(rest).next().unwrap_or(" ");
        let before = &visible[..cursor_display];
        let after = visible.get(cursor_display + at.len()..).unwrap_or("");
        let text = format!("{before}{marker}{}{after}", render_fake_cursor(at));
        let pad = " ".repeat(avail.saturating_sub(visible_width(&text)));
        vec![format!("{}{text}{pad}", self.prompt)]
    }
}
