//! Padded component container matching pi's `Box` rendering rules.
use crate::{component::Component, utils::visible_width};
use std::sync::{Arc, Mutex};

type Background = Arc<dyn Fn(&str) -> String + Send + Sync>;
pub struct BoxComponent {
    children: Mutex<Vec<Arc<dyn Component>>>,
    padding_x: usize,
    padding_y: usize,
    background: Mutex<Option<Background>>,
}
impl BoxComponent {
    pub fn new(padding_x: usize, padding_y: usize) -> Self {
        Self {
            children: Mutex::new(Vec::new()),
            padding_x,
            padding_y,
            background: Mutex::new(None),
        }
    }
    pub fn add(&self, child: Arc<dyn Component>) {
        if let Ok(mut c) = self.children.lock() {
            c.push(child)
        }
    }
    pub fn remove(&self, child: &Arc<dyn Component>) {
        if let Ok(mut c) = self.children.lock()
            && let Some(i) = c.iter().position(|x| Arc::ptr_eq(x, child))
        {
            c.remove(i);
        }
    }
    pub fn clear(&self) {
        if let Ok(mut c) = self.children.lock() {
            c.clear()
        }
    }
    pub fn set_background(&self, background: Option<Background>) {
        if let Ok(mut b) = self.background.lock() {
            *b = background
        }
    }
    fn finish(&self, line: String, width: usize) -> String {
        let mut line = line;
        line.push_str(&" ".repeat(width.saturating_sub(visible_width(&line))));
        self.background
            .lock()
            .ok()
            .and_then(|b| b.clone())
            .map_or(line.clone(), |bg| bg(&line))
    }
}
impl Component for BoxComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let Some(children) = self.children.lock().ok() else {
            return Vec::new();
        };
        if children.is_empty() {
            return Vec::new();
        }
        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let left = " ".repeat(self.padding_x);
        let child: Vec<_> = children
            .iter()
            .flat_map(|c| c.render(content_width))
            .map(|l| format!("{left}{l}"))
            .collect();
        if child.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for _ in 0..self.padding_y {
            out.push(self.finish(String::new(), width));
        }
        for l in child {
            out.push(self.finish(l, width));
        }
        for _ in 0..self.padding_y {
            out.push(self.finish(String::new(), width));
        }
        out
    }
}
impl Default for BoxComponent {
    fn default() -> Self {
        Self::new(1, 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Text;
    #[test]
    fn empty_and_padding_match_reference() {
        let b = BoxComponent::new(1, 1);
        assert!(b.render(8).is_empty());
        b.add(Arc::new(Text::new("x", 0, 0)));
        assert_eq!(b.render(8), ["        ", " x      ", "        "]);
    }
}
