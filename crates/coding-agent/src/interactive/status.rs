use std::cell::RefCell;
use std::rc::Rc;

use pi_tui::components::editor::{Style, TopBorder};
use pi_tui::components::loader::Loader;
use pi_tui::components::text::StyleFn;
use pi_tui::tui::Component;
use pi_tui::utils::{truncate_to_width, visible_width};

pub const WORKING: &str = "Working";

pub struct StatusIndicator {
    loader: Loader,
}

impl StatusIndicator {
    pub fn new(message: &str, color: StyleFn) -> StatusIndicator {
        let mut loader = Loader::new(color.clone(), color, message, None);
        loader.start();
        StatusIndicator { loader }
    }

    pub fn set_message(&mut self, message: &str) {
        self.loader.set_message(message);
    }

    pub fn tick(&mut self, now: u64) -> (bool, Option<u64>) {
        self.loader.tick(now)
    }

    pub fn invalidate(&mut self) {
        self.loader.invalidate();
    }

    fn render_in_border(&mut self, width: usize) -> String {
        let line = self
            .loader
            .render(width + 2)
            .into_iter()
            .nth(1)
            .unwrap_or_default();
        let line = line.strip_prefix(' ').unwrap_or(&line).trim_end();
        truncate_to_width(line, width, "", false)
    }

    fn render_spinner_in_border(&self, width: usize) -> String {
        truncate_to_width(&self.loader.rendered_indicator(), width, "", false)
    }
}

pub type Embedded = Rc<RefCell<Option<StatusIndicator>>>;

pub fn embedded_top_border(status: Embedded) -> TopBorder {
    Box::new(move |width: usize, hidden: usize, border: &Style| {
        let mut slot = status.borrow_mut();
        let indicator = slot.as_mut()?;
        if width == 0 {
            return None;
        }
        let mut text = indicator.render_in_border(width.saturating_sub(5).max(1));
        let mut text_width = visible_width(&text);
        if text_width == 0 {
            return None;
        }
        let overflow = (hidden > 0).then(|| format!(" ↑ {hidden} more "));
        let overflow_width = overflow.as_deref().map_or(0, visible_width);
        let overflow_start = (width as i64 - overflow_width as i64).div_euclid(2);
        let fits = |text_width: usize| {
            overflow.is_some()
                && overflow_width + 2 <= width
                && overflow_start - (3 + text_width as i64 + 1) >= 1
        };
        if overflow.is_some() && !fits(text_width) {
            text = indicator.render_spinner_in_border(width);
            text_width = visible_width(&text);
        }
        if let Some(label) = overflow.as_deref().filter(|_| fits(text_width)) {
            let left = 3 + text_width as i64 + 1;
            let start = overflow_start as usize;
            return Some(format!(
                "{}{text}{}",
                border("── "),
                border(&format!(
                    " {}{label}{}",
                    "─".repeat((overflow_start - left) as usize),
                    "─".repeat(width - start - overflow_width)
                ))
            ));
        }
        if width >= text_width + 5 {
            return Some(format!(
                "{}{text}{}",
                border("── "),
                border(&format!(" {}", "─".repeat(width - text_width - 4)))
            ));
        }
        text = indicator.render_spinner_in_border(width);
        text_width = visible_width(&text);
        let prefix = 3.min(width.saturating_sub(text_width));
        Some(format!(
            "{}{text}{}",
            border(&"─".repeat(prefix)),
            border(&"─".repeat(width.saturating_sub(prefix + text_width)))
        ))
    })
}
