//! Component primitives and the deterministic, terminal-independent render seam.

use std::sync::{Arc, Mutex};

pub trait Component: Send + Sync {
    fn render(&self, width: usize) -> Vec<String>;
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    crate::utils::wrap_text_with_ansi(&text.replace('\t', "   "), width.max(1))
}

pub struct Text {
    text: Mutex<String>,
    padding_x: usize,
    padding_y: usize,
}
impl Text {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: Mutex::new(text.into()),
            padding_x,
            padding_y,
        }
    }
    pub fn set_text(&self, text: impl Into<String>) {
        if let Ok(mut current) = self.text.lock() {
            *current = text.into();
        }
    }
}
impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        let width = width.max(1);
        let text = self.text.lock().map(|v| v.clone()).unwrap_or_default();
        if text.trim().is_empty() {
            return Vec::new();
        }
        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let left = " ".repeat(self.padding_x);
        let empty = " ".repeat(width);
        let mut out = vec![empty.clone(); self.padding_y];
        for line in wrap_text(&text, content_width) {
            let mut value = format!("{left}{line}{left}");
            let n = crate::utils::visible_width(&value);
            value.push_str(&" ".repeat(width.saturating_sub(n)));
            out.push(value);
        }
        out.extend(std::iter::repeat_n(empty, self.padding_y));
        out
    }
}

pub struct Container {
    children: Mutex<Vec<Arc<dyn Component>>>,
}
impl Container {
    pub fn new() -> Self {
        Self {
            children: Mutex::new(Vec::new()),
        }
    }
    pub fn add(&self, child: Arc<dyn Component>) {
        if let Ok(mut children) = self.children.lock() {
            children.push(child);
        }
    }
}
impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}
impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        self.children
            .lock()
            .map(|children| children.iter().flat_map(|c| c.render(width)).collect())
            .unwrap_or_default()
    }
}

/// Stateful differential renderer. Output is a deterministic ANSI write buffer.
pub struct Renderer {
    previous: Vec<String>,
    full_redraws: usize,
}
impl Renderer {
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
            full_redraws: 0,
        }
    }
    pub fn render(&mut self, lines: Vec<String>, clear: bool) -> String {
        // The standalone seam historically assumes the cursor is already on
        // the first changed row. Live terminal rendering uses `render_at`.
        self.render_at(lines, clear, None)
    }

    /// Render while accounting for the terminal's current logical cursor row.
    ///
    /// A differential frame cannot simply emit `\r` and the changed suffix:
    /// the hardware cursor is normally left at the editor cursor, which may be
    /// several rows below the first changed line. Without this repositioning,
    /// every keystroke appends another copy of the lower frame.
    pub fn render_at(
        &mut self,
        lines: Vec<String>,
        clear: bool,
        current_row: Option<usize>,
    ) -> String {
        if clear || self.previous.is_empty() {
            self.full_redraws += 1;
            let mut out = String::from("\x1b[?2026h");
            if clear {
                out.push_str("\x1b[2J\x1b[H\x1b[3J");
            }
            out.push_str(&lines.join("\r\n"));
            out.push_str("\x1b[?2026l");
            self.previous = lines;
            return out;
        }
        let first = self
            .previous
            .iter()
            .zip(lines.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(self.previous.len().min(lines.len()));
        if first == self.previous.len() && lines.len() == self.previous.len() {
            return String::new();
        }

        let mut out = String::from("\x1b[?2026h");
        if first < lines.len() {
            let appended = first == self.previous.len() && first > 0;
            let target_row = if appended { first - 1 } else { first };
            if let Some(current_row) = current_row {
                match current_row.cmp(&target_row) {
                    std::cmp::Ordering::Greater => {
                        out.push_str(&format!("\x1b[{}A", current_row - target_row));
                    }
                    std::cmp::Ordering::Less => {
                        out.push_str(&format!("\x1b[{}B", target_row - current_row));
                    }
                    std::cmp::Ordering::Equal => {}
                }
            }
            if appended {
                out.push_str("\r\n");
            } else {
                out.push('\r');
            }
            out.push_str(&lines[first..].join("\r\n"));
        }
        if lines.len() < self.previous.len() {
            for _ in lines.len()..self.previous.len() {
                out.push_str("\r\n\x1b[2K");
            }
        }
        out.push_str("\x1b[?2026l");
        self.previous = lines;
        out
    }
    pub fn full_redraws(&self) -> usize {
        self.full_redraws
    }
}
impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the pi Text component without terminal state.
pub fn render_text(text: &str, width: usize, padding_x: usize, padding_y: usize) -> Vec<String> {
    Text::new(text, padding_x, padding_y).render(width)
}

/// Render the pi differential seam against a prior cell-line snapshot.
pub fn differential_render(previous: &[String], lines: &[String], clear: bool) -> String {
    let mut renderer = Renderer {
        previous: previous.to_vec(),
        full_redraws: 0,
    };
    renderer.render(lines.to_vec(), clear)
}

#[cfg(test)]
mod tests {
    use super::{Renderer, differential_render, render_text};

    #[test]
    fn text_and_cell_fixture_is_deterministic() {
        let lines = render_text("hello\nworld", 10, 1, 0);
        assert_eq!(lines, [" hello    ", " world    "]);
        assert_eq!(
            differential_render(&[], &lines, true),
            "\x1b[?2026h\x1b[2J\x1b[H\x1b[3J hello    \r\n world    \x1b[?2026l"
        );
    }

    #[test]
    fn unchanged_cells_emit_no_output() {
        let mut renderer = Renderer::new();
        let lines = vec![" hello    ".to_owned()];
        let _ = renderer.render(lines.clone(), false);
        assert_eq!(renderer.render(lines, false), "");
    }
}
