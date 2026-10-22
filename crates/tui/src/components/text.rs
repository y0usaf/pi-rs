//! Multi-line text with word wrapping and padding (upstream `components/text.ts`).

use crate::tui::Component;
use crate::utils::{apply_background_to_line, js_trim, visible_width, wrap_text_with_ansi};

pub type StyleFn = std::rc::Rc<dyn Fn(&str) -> String>;

pub struct Text {
    text: String,
    padding_x: usize,
    padding_y: usize,
    bg: Option<StyleFn>,
    cache: Option<(String, usize, Vec<String>)>,
}

impl Default for Text {
    fn default() -> Self {
        Text::new("", 1, 1)
    }
}

impl Text {
    pub fn new(text: &str, padding_x: usize, padding_y: usize) -> Self {
        Text { text: text.into(), padding_x, padding_y, bg: None, cache: None }
    }
    pub fn with_bg(text: &str, padding_x: usize, padding_y: usize, bg: StyleFn) -> Self {
        Text { bg: Some(bg), ..Text::new(text, padding_x, padding_y) }
    }
    pub fn set_text(&mut self, text: &str) {
        self.text = text.into();
        self.cache = None;
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn set_custom_bg_fn(&mut self, bg: Option<StyleFn>) {
        self.bg = bg;
        self.cache = None;
    }
    pub fn set_padding_x(&mut self, p: usize) {
        self.padding_x = p;
        self.cache = None;
    }
}

impl Component for Text {
    fn render(&mut self, width: usize) -> Vec<String> {
        if let Some((t, w, lines)) = &self.cache {
            if *t == self.text && *w == width {
                return lines.clone();
            }
        }
        if js_trim(&self.text).is_empty() {
            self.cache = Some((self.text.clone(), width, Vec::new()));
            return Vec::new();
        }
        let normalized = self.text.replace('\t', "   ");
        let px = self.padding_x.min(width.saturating_sub(1) / 2);
        let content_width = width.saturating_sub(px * 2).max(1);
        let margin = " ".repeat(px);
        let mut content = Vec::new();
        for line in wrap_text_with_ansi(&normalized, content_width) {
            let l = format!("{margin}{line}{margin}");
            match &self.bg {
                Some(bg) => content.push(apply_background_to_line(&l, width, &|s| bg(s))),
                None => {
                    let pad = width.saturating_sub(visible_width(&l));
                    content.push(format!("{l}{}", " ".repeat(pad)));
                }
            }
        }
        let empty = " ".repeat(width);
        let empty_line = match &self.bg {
            Some(bg) => apply_background_to_line(&empty, width, &|s| bg(s)),
            None => empty,
        };
        let mut result = vec![empty_line.clone(); self.padding_y];
        result.extend(content);
        result.extend(std::iter::repeat_n(empty_line, self.padding_y));
        self.cache = Some((self.text.clone(), width, result.clone()));
        if result.is_empty() { vec![String::new()] } else { result }
    }
    fn invalidate(&mut self) {
        self.cache = None;
    }
}
