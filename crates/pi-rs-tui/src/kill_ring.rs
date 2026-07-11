//! Emacs-style kill ring, ported from `packages/tui/src/kill-ring.ts`.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KillRing {
    entries: Vec<String>,
}

impl KillRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, text: &str, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate && let Some(last) = self.entries.pop() {
            self.entries.push(if prepend {
                format!("{text}{last}")
            } else {
                format!("{last}{text}")
            });
        } else {
            self.entries.push(text.to_owned());
        }
    }

    pub fn peek(&self) -> Option<&str> {
        self.entries.last().map(String::as_str)
    }

    pub fn rotate(&mut self) {
        if self.entries.len() > 1
            && let Some(last) = self.entries.pop()
        {
            self.entries.insert(0, last);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::KillRing;
    #[test]
    fn accumulation_and_yank_pop_match_pi() {
        let mut ring = KillRing::new();
        ring.push("one", false, false);
        ring.push(" two", false, true);
        ring.push("zero ", true, true);
        assert_eq!(ring.peek(), Some("zero one two"));
        ring.push("old", false, false);
        ring.rotate();
        assert_eq!(ring.peek(), Some("zero one two"));
    }
}
