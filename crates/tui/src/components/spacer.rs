//! Empty lines (upstream `components/spacer.ts`).

use crate::tui::Component;

pub struct Spacer {
    pub lines: usize,
}

impl Spacer {
    pub fn new(lines: usize) -> Self {
        Spacer { lines }
    }
    pub fn set_lines(&mut self, lines: usize) {
        self.lines = lines;
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Spacer::new(1)
    }
}

impl Component for Spacer {
    fn render(&mut self, _width: usize) -> Vec<String> {
        vec![String::new(); self.lines]
    }
}
