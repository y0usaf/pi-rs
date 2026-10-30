use std::time::{Duration, Instant};

use crate::utils::truncate_to_width;

pub const DEFAULT_FLASH_DURATION: Duration = Duration::from_millis(1000);

#[derive(Default)]
pub struct AltScreenFlashContainer {
    entries: Vec<(String, Instant)>,
}

impl AltScreenFlashContainer {
    pub fn flash(&mut self, message: &str, duration: Duration) {
        self.entries
            .push((message.to_owned(), Instant::now() + duration));
    }

    pub fn dispose(&mut self) {
        self.entries.clear();
    }

    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(_, until)| *until > now);
        self.entries.len() != before
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.entries.iter().map(|(_, until)| *until).min()
    }

    pub fn render(&self, width: usize) -> Vec<String> {
        self.entries
            .iter()
            .map(|(message, _)| {
                let message = truncate_to_width(&format!(" {message} "), width, "", false);
                format!("\x1b[7m{message}\x1b[27m")
            })
            .collect()
    }
}
