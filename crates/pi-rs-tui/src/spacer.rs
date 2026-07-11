use crate::component::Component;
use std::sync::atomic::{AtomicUsize, Ordering};
pub struct Spacer {
    lines: AtomicUsize,
}
impl Spacer {
    pub fn new(lines: usize) -> Self {
        Self {
            lines: AtomicUsize::new(lines),
        }
    }
    pub fn set_lines(&self, lines: usize) {
        self.lines.store(lines, Ordering::Relaxed)
    }
}
impl Default for Spacer {
    fn default() -> Self {
        Self::new(1)
    }
}
impl Component for Spacer {
    fn render(&self, _: usize) -> Vec<String> {
        vec![String::new(); self.lines.load(Ordering::Relaxed)]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn count_is_mutable() {
        let s = Spacer::default();
        assert_eq!(s.render(0), [""]);
        s.set_lines(3);
        assert_eq!(s.render(9).len(), 3)
    }
}
