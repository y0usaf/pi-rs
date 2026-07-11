//! Clone-on-push undo snapshots, ported from `packages/tui/src/undo-stack.ts`.

#[derive(Clone, Debug, PartialEq)]
pub struct UndoStack<T> {
    stack: Vec<T>,
}

impl<T> Default for UndoStack<T> {
    fn default() -> Self {
        Self { stack: Vec::new() }
    }
}

impl<T: Clone> UndoStack<T> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, state: &T) {
        self.stack.push(state.clone());
    }
    pub fn pop(&mut self) -> Option<T> {
        self.stack.pop()
    }
    pub fn clear(&mut self) {
        self.stack.clear();
    }
    pub fn len(&self) -> usize {
        self.stack.len()
    }
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::UndoStack;
    #[test]
    fn push_detaches_snapshot() {
        let mut stack = UndoStack::new();
        let mut state = vec!["before"];
        stack.push(&state);
        state[0] = "after";
        assert_eq!(stack.pop(), Some(vec!["before"]));
    }
}
