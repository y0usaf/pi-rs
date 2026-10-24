//! One line of text truncated to the width (upstream `components/truncated-text.ts`).

use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width};

pub struct TruncatedText {
    pub text: String,
    pub padding_x: usize,
    pub padding_y: usize,
}

impl TruncatedText {
    pub fn new(text: &str, padding_x: usize, padding_y: usize) -> Self {
        TruncatedText {
            text: text.into(),
            padding_x,
            padding_y,
        }
    }
}

impl Component for TruncatedText {
    fn render(&mut self, width: usize) -> Vec<String> {
        let empty = " ".repeat(width);
        let mut out = vec![empty.clone(); self.padding_y];
        let avail = width.saturating_sub(self.padding_x * 2).max(1);
        let first = self.text.split('\n').next().unwrap_or("");
        let shown = truncate_to_width(first, avail, "...", false);
        let pad = " ".repeat(self.padding_x);
        let line = format!("{pad}{shown}{pad}");
        let fill = width.saturating_sub(visible_width(&line));
        out.push(format!("{line}{}", " ".repeat(fill)));
        out.extend(std::iter::repeat_n(empty, self.padding_y));
        out
    }
}
