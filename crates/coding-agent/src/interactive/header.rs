use pi_tui::components::text::Text;
use pi_tui::terminal::is_apple_terminal_session;
use pi_tui::tui::Component;

use super::selectors::{key_hint, key_text, raw_key_hint};
use super::theme::{Color, fg};
use crate::config::VERSION;

const CORAL: Color = Color::Rgb(228, 138, 122);
const BLUE: Color = Color::Rgb(79, 142, 179);
const YELLOW: Color = Color::Rgb(234, 182, 93);
const RESET: &str = "\x1b[0m";

pub type Build = Box<dyn Fn() -> String>;

pub struct ExpandableText {
    collapsed: Build,
    expanded_text: Build,
    expanded: bool,
    text: Text,
    stale: bool,
}

impl ExpandableText {
    pub fn new(
        collapsed: Build,
        expanded_text: Build,
        expanded: bool,
        padding_x: usize,
        padding_y: usize,
    ) -> ExpandableText {
        ExpandableText {
            collapsed,
            expanded_text,
            expanded,
            text: Text::new("", padding_x, padding_y),
            stale: true,
        }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.invalidate();
    }
}

impl Component for ExpandableText {
    fn render(&mut self, width: usize) -> Vec<String> {
        if self.stale {
            self.stale = false;
            let text = if self.expanded {
                (self.expanded_text)()
            } else {
                (self.collapsed)()
            };
            self.text.set_text(&text);
        }
        self.text.render(width)
    }
    fn invalidate(&mut self) {
        self.text.invalidate();
        self.stale = true;
    }
}

pub fn pi_logo_lines() -> (String, String) {
    let top = format!(
        "{}{}▀{RESET}{}▀█{RESET} ",
        CORAL.ansi(false),
        BLUE.ansi(true),
        CORAL.ansi(false)
    );
    let bottom = format!(
        "{}█▀{RESET} {}█{RESET}",
        BLUE.ansi(false),
        YELLOW.ansi(false)
    );
    (top, bottom)
}

pub fn pi_wordmark() -> String {
    format!(
        "{}P{RESET}{}i{RESET}",
        CORAL.ansi(false),
        YELLOW.ansi(false)
    )
}

fn with_logo(hints: &str) -> String {
    let version = fg("dim", &format!("v{VERSION}"));
    if is_apple_terminal_session() {
        return format!("{} {version}\n{hints}", pi_wordmark());
    }
    let (top, bottom) = pi_logo_lines();
    format!("{top} {version}\n{bottom} {hints}")
}

fn expanded_instructions() -> String {
    let clear = key_text("app.clear");
    let cycle = format!(
        "{}/{}",
        key_text("app.model.cycleForward"),
        key_text("app.model.cycleBackward")
    );
    [
        key_hint("app.interrupt", "to interrupt"),
        key_hint("app.clear", "to clear"),
        raw_key_hint(&format!("{clear} twice"), "to exit"),
        key_hint("app.exit", "to exit (empty)"),
        key_hint("app.suspend", "to suspend"),
        key_hint("tui.editor.deleteToLineEnd", "to delete to end"),
        key_hint("app.thinking.cycle", "to cycle thinking level"),
        raw_key_hint(&cycle, "to cycle models"),
        key_hint("app.model.select", "to select model"),
        key_hint("app.tools.expand", "to expand tools"),
        key_hint("app.thinking.toggle", "to expand thinking"),
        key_hint("app.editor.external", "for external editor"),
        raw_key_hint("/", "for commands"),
        raw_key_hint("!", "to run bash"),
        raw_key_hint("!!", "to run bash (no context)"),
        key_hint("app.message.followUp", "to queue follow-up"),
        key_hint("app.message.dequeue", "to edit all queued messages"),
        key_hint(
            "app.clipboard.pasteImage",
            "to paste files on macOS, images, or text",
        ),
        raw_key_hint("drop files", "to attach"),
    ]
    .join("\n")
}

fn compact_instructions() -> String {
    [
        key_hint("app.interrupt", "interrupt"),
        raw_key_hint(
            &format!("{}/{}", key_text("app.clear"), key_text("app.exit")),
            "clear/exit",
        ),
        raw_key_hint("/", "commands"),
        raw_key_hint("!", "bash"),
        key_hint("app.tools.expand", "more"),
    ]
    .join(&fg("muted", " · "))
}

fn onboarding() -> String {
    fg(
        "dim",
        "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.",
    )
}

pub fn built_in_header(show_details: bool, expanded: bool) -> ExpandableText {
    let details = if show_details {
        " and loaded resources"
    } else {
        ""
    };
    ExpandableText::new(
        Box::new(move || {
            let compact_onboarding = fg(
                "dim",
                &format!(
                    "Press {} to show full startup help{details}.",
                    key_text("app.tools.expand")
                ),
            );
            format!(
                "{}\n{compact_onboarding}\n\n{}",
                with_logo(&compact_instructions()),
                onboarding()
            )
        }),
        Box::new(|| {
            format!(
                "{}\n\n{}",
                with_logo(&expanded_instructions()),
                onboarding()
            )
        }),
        expanded,
        1,
        0,
    )
}
