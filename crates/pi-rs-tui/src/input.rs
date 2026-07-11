//! Single-line input component with pi-compatible scrolling and cursor rendering.
use crate::{
    component::Component,
    editor::{Editor, decode_key},
    utils::{slice_by_column, visible_width},
};
use std::sync::Mutex;
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed(String),
    Submit(String),
    Cancel,
    None,
}
struct InputState {
    editor: Editor,
    focused: bool,
    paste: Option<String>,
}
pub struct Input {
    state: Mutex<InputState>,
}
impl Input {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: Mutex::new(InputState {
                editor: Editor::new(value),
                focused: false,
                paste: None,
            }),
        }
    }
    pub fn set_focused(&self, focused: bool) {
        if let Ok(mut s) = self.state.lock() {
            s.focused = focused
        }
    }
    pub fn value(&self) -> String {
        self.state
            .lock()
            .map(|s| s.editor.value().to_owned())
            .unwrap_or_default()
    }
    pub fn set_value(&self, value: impl Into<String>) {
        if let Ok(mut s) = self.state.lock() {
            s.editor.set_value(value);
        }
    }
    pub fn handle_input(&self, data: &str) -> InputEvent {
        let Ok(mut s) = self.state.lock() else {
            return InputEvent::None;
        };
        if let Some(start) = data.find("\x1b[200~") {
            s.paste = Some(data[start + 6..].to_owned());
        }
        if let Some(mut buffer) = s.paste.take() {
            if !data.contains("\x1b[200~") {
                buffer.push_str(data);
            }
            if let Some(end) = buffer.find("\x1b[201~") {
                let clean = buffer[..end]
                    .replace("\r\n", "")
                    .replace(['\r', '\n'], "")
                    .replace('\t', "    ");
                let remaining = buffer[end + 6..].to_owned();
                let before = s.editor.value().to_owned();
                s.editor.insert(&clean);
                if !remaining.is_empty() {
                    apply_input(&mut s.editor, &remaining);
                }
                return if before == s.editor.value() {
                    InputEvent::None
                } else {
                    InputEvent::Changed(s.editor.value().to_owned())
                };
            }
            s.paste = Some(buffer);
            return InputEvent::None;
        }
        match decode_key(data).as_deref() {
            Some("enter") => InputEvent::Submit(s.editor.value().to_owned()),
            Some("escape") | Some("ctrl+c") => InputEvent::Cancel,
            _ => {
                let before = s.editor.value().to_owned();
                apply_input(&mut s.editor, data);
                if before == s.editor.value() {
                    InputEvent::None
                } else {
                    InputEvent::Changed(s.editor.value().to_owned())
                }
            }
        }
    }
}
fn apply_input(editor: &mut Editor, data: &str) {
    if decode_key(data).is_none() && !data.chars().any(char::is_control) {
        editor.insert(data);
    } else {
        editor.handle(data);
    }
}
impl Default for Input {
    fn default() -> Self {
        Self::new("")
    }
}
impl Component for Input {
    fn render(&self, width: usize) -> Vec<String> {
        let Ok(s) = self.state.lock() else {
            return Vec::new();
        };
        let prompt = "> ";
        let available = width.saturating_sub(2);
        if available == 0 {
            return vec![prompt.to_owned()];
        };
        let value = s.editor.value();
        let total = visible_width(value);
        let cursor_col = visible_width(&value[..s.editor.cursor()]);
        let (visible, cursor_byte) = if total < available {
            (value.to_owned(), s.editor.cursor())
        } else {
            let sw = if s.editor.cursor() == value.len() {
                available.saturating_sub(1)
            } else {
                available
            };
            let half = sw / 2;
            let start = if cursor_col < half {
                0
            } else if cursor_col > total.saturating_sub(half) {
                total.saturating_sub(sw)
            } else {
                cursor_col - half
            };
            let v = slice_by_column(value, start, sw, true);
            let before = slice_by_column(value, start, cursor_col.saturating_sub(start), true);
            (v, before.len())
        };
        let tail = &visible[cursor_byte.min(visible.len())..];
        let at = tail.graphemes(true).next().unwrap_or(" ");
        let before = &visible[..cursor_byte.min(visible.len())];
        let after = &tail[at.len().min(tail.len())..];
        let marker = if s.focused { CURSOR_MARKER } else { "" };
        let body = format!("{before}{marker}\x1b[7m{at}\x1b[27m{after}");
        let pad = " ".repeat(available.saturating_sub(visible_width(&body)));
        vec![format!("{prompt}{body}{pad}")]
    }
}
use unicode_segmentation::UnicodeSegmentation;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_and_scroll() {
        let i = Input::new("abcdef");
        assert_eq!(i.render(7), ["> cdef\x1b[7m \x1b[27m"]);
        i.set_focused(true);
        assert!(i.render(20)[0].contains(CURSOR_MARKER));
    }
    #[test]
    fn paste_is_cleaned() {
        let i = Input::default();
        i.handle_input("\x1b[200~a\n\tb\x1b[201~");
        assert_eq!(i.value(), "a    b");
    }
    #[test]
    fn set_value_preserves_cursor_and_escape_cancels() {
        let i = Input::new("abcd");
        i.handle_input("\x1b[D");
        i.set_value("xy");
        i.handle_input("!");
        assert_eq!(i.value(), "xy!");
        assert_eq!(i.handle_input("\x1b"), InputEvent::Cancel);
    }
    #[test]
    fn paste_processes_trailing_input() {
        let i = Input::default();
        assert_eq!(
            i.handle_input("\x1b[200~a\n\tb\x1b[201~z"),
            InputEvent::Changed("a    bz".into())
        );
    }
}
