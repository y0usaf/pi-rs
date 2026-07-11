use crate::{
    component::Component,
    utils::{truncate_to_width, visible_width},
};
use std::sync::Mutex;
pub struct TruncatedText {
    text: Mutex<String>,
    padding_x: usize,
    padding_y: usize,
}
impl TruncatedText {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: Mutex::new(text.into()),
            padding_x,
            padding_y,
        }
    }
    pub fn set_text(&self, text: impl Into<String>) {
        if let Ok(mut t) = self.text.lock() {
            *t = text.into()
        }
    }
}
impl Component for TruncatedText {
    fn render(&self, width: usize) -> Vec<String> {
        let empty = " ".repeat(width);
        let mut out = vec![empty.clone(); self.padding_y];
        let available = width.saturating_sub(self.padding_x * 2).max(1);
        let text = self
            .text
            .lock()
            .map(|t| t.split('\n').next().unwrap_or("").to_owned())
            .unwrap_or_default();
        let display = truncate_to_width(&text, available, "...", false);
        let mut line = format!(
            "{}{}{}",
            " ".repeat(self.padding_x),
            display,
            " ".repeat(self.padding_x)
        );
        line.push_str(&" ".repeat(width.saturating_sub(visible_width(&line))));
        out.push(line);
        out.extend(std::iter::repeat_n(empty, self.padding_y));
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_line_only_and_ellipsis() {
        // Truncated output carries finalizeTruncatedResult's resets.
        assert_eq!(
            TruncatedText::new("abcdef\nignored", 1, 1).render(7),
            ["       ", " ab\x1b[0m...\x1b[0m ", "       "]
        );
    }
}
