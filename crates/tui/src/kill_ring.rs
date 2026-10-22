//! Emacs-style kill ring (upstream `kill-ring.ts`) and the undo stack (`undo-stack.ts`).

/// Killed text; consecutive kills can accumulate into one entry.
#[derive(Debug, Clone, Default)]
pub struct KillRing {
    ring: Vec<String>,
}

impl KillRing {
    /// Add killed text; with `accumulate`, merge into the latest entry (in front when `prepend`).
    pub fn push(&mut self, text: &str, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate {
            if let Some(last) = self.ring.pop() {
                self.ring.push(if prepend { format!("{text}{last}") } else { format!("{last}{text}") });
                return;
            }
        }
        self.ring.push(text.into());
    }
    pub fn peek(&self) -> Option<&str> {
        self.ring.last().map(|s| s.as_str())
    }
    /// Move the latest entry to the front (yank-pop).
    pub fn rotate(&mut self) {
        if self.ring.len() > 1 {
            let last = self.ring.pop().unwrap();
            self.ring.insert(0, last);
        }
    }
    pub fn len(&self) -> usize {
        self.ring.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

/// Snapshots for undo.
#[derive(Debug, Clone)]
pub struct UndoStack<S: Clone> {
    stack: Vec<S>,
}

impl<S: Clone> Default for UndoStack<S> {
    fn default() -> Self {
        UndoStack { stack: Vec::new() }
    }
}

impl<S: Clone> UndoStack<S> {
    pub fn push(&mut self, s: &S) {
        self.stack.push(s.clone());
    }
    pub fn pop(&mut self) -> Option<S> {
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
