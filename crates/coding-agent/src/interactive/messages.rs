//! `components/user-message.ts` and `components/assistant-message.ts`.

use pi_durable::ai::types::{AssistantBlock, AssistantMessage, StopReason};
use pi_tui::components::markdown::DefaultTextStyle;
use pi_tui::components::markdown::{Markdown, MarkdownOptions};
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::tui::Component;
use pi_tui::utils::apply_background_to_line;

use super::theme;

const OSC133_ZONE_START: &str = "\x1b]133;A\x07";
const OSC133_ZONE_END: &str = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL: &str = "\x1b]133;C\x07";

/// Marks the lines as one prompt zone, as upstream does for user and plain assistant messages.
fn zone(mut lines: Vec<String>) -> Vec<String> {
    if let Some(first) = lines.first_mut() {
        first.insert_str(0, OSC133_ZONE_START);
    }
    if let Some(last) = lines.last_mut() {
        last.insert_str(0, &format!("{OSC133_ZONE_END}{OSC133_ZONE_FINAL}"));
    }
    lines
}

/// The user's message: Markdown on the `userMessageBg` background, one line of padding.
pub struct UserMessage {
    markdown: Markdown,
}

impl UserMessage {
    pub fn new(text: &str, output_pad: usize) -> Self {
        UserMessage {
            markdown: Markdown::new(
                text,
                output_pad,
                1,
                theme::markdown(),
                Some(theme::user_style()),
                MarkdownOptions {
                    preserve_ordered_list_markers: true,
                    preserve_backslash_escapes: true,
                    ..MarkdownOptions::default()
                },
            ),
        }
    }
}

impl Component for UserMessage {
    fn render(&mut self, width: usize) -> Vec<String> {
        zone(self.markdown.render(width))
    }
    fn invalidate(&mut self) {
        self.markdown.invalidate();
    }
}

pub struct CustomMessage {
    label: Text,
    body: Markdown,
    output_pad: usize,
}

impl CustomMessage {
    pub fn new(custom_type: &str, text: &str, output_pad: usize) -> Self {
        CustomMessage {
            label: Text::new(
                &theme::fg(
                    "customMessageLabel",
                    &theme::bold(&format!("[{custom_type}]")),
                ),
                0,
                0,
            ),
            body: Markdown::new(
                text,
                0,
                0,
                theme::markdown(),
                Some(DefaultTextStyle {
                    color: Some(theme::fg_fn("customMessageText")),
                    ..DefaultTextStyle::default()
                }),
                MarkdownOptions::default(),
            ),
            output_pad,
        }
    }
}

impl Component for CustomMessage {
    fn render(&mut self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(self.output_pad * 2).max(1);
        let margin = " ".repeat(self.output_pad);
        let bg = |text: &str| theme::bg("customMessageBg", text);
        let blank = apply_background_to_line(&" ".repeat(width), width, &bg);
        let mut body = self.label.render(inner);
        body.push(String::new());
        body.extend(self.body.render(inner));
        let mut lines = vec![String::new(), blank.clone()];
        lines.extend(
            body.iter()
                .map(|line| apply_background_to_line(&format!("{margin}{line}"), width, &bg)),
        );
        lines.push(blank);
        lines
    }
    fn invalidate(&mut self) {
        self.label.invalidate();
        self.body.invalidate();
    }
}

/// An assistant message: its text as Markdown, thinking in italics, and the stop notice for a
/// truncated, aborted or failed reply. Tool calls are drawn by their own rows.
pub struct AssistantView {
    parts: Vec<Box<dyn Component>>,
    hide_thinking: bool,
    output_pad: usize,
    has_tool_calls: bool,
    last: Option<AssistantMessage>,
}

fn visible(block: &AssistantBlock) -> bool {
    match block {
        AssistantBlock::Text(t) => !t.text.trim().is_empty(),
        AssistantBlock::Thinking(t) => !t.thinking.trim().is_empty(),
        AssistantBlock::ToolCall(_) => false,
    }
}

impl AssistantView {
    pub fn new(hide_thinking: bool, output_pad: usize) -> Self {
        AssistantView {
            parts: Vec::new(),
            hide_thinking,
            output_pad,
            has_tool_calls: false,
            last: None,
        }
    }

    pub fn with_message(
        message: &AssistantMessage,
        hide_thinking: bool,
        output_pad: usize,
    ) -> Self {
        let mut view = Self::new(hide_thinking, output_pad);
        view.update(message);
        view
    }

    /// Show or hide the thinking blocks, redrawing the message that was here.
    pub fn set_hide_thinking(&mut self, hide: bool) {
        self.hide_thinking = hide;
        if let Some(message) = self.last.clone() {
            self.update(&message);
        }
    }

    /// `updateContent`: rebuilds the parts from the message as it stands.
    pub fn update(&mut self, message: &AssistantMessage) {
        self.last = Some(message.clone());
        let pad = self.output_pad;
        let mut parts: Vec<Box<dyn Component>> = Vec::new();
        let content = &message.content;
        if content.iter().any(visible) {
            parts.push(Box::new(Spacer::new(1)));
        }
        let mut i = 0;
        while i < content.len() {
            match &content[i] {
                AssistantBlock::Text(t) if !t.text.trim().is_empty() => {
                    parts.push(Box::new(Markdown::new(
                        t.text.trim(),
                        pad,
                        0,
                        theme::markdown(),
                        None,
                        MarkdownOptions::default(),
                    )));
                    i += 1;
                }
                AssistantBlock::Thinking(_) => {
                    let mut blocks = Vec::new();
                    while let Some(AssistantBlock::Thinking(t)) = content.get(i) {
                        let text = t.thinking.trim();
                        if !text.is_empty() {
                            blocks.push(text.to_string());
                        }
                        i += 1;
                    }
                    if blocks.is_empty() {
                        continue;
                    }
                    let after = content[i..].iter().any(visible);
                    if self.hide_thinking {
                        parts.push(Box::new(Text::new(
                            &theme::italic(&theme::fg("thinkingText", "Thinking...")),
                            pad,
                            0,
                        )));
                    } else {
                        parts.push(Box::new(Markdown::new(
                            &blocks.join("\n\n"),
                            pad,
                            0,
                            theme::markdown(),
                            Some(theme::thinking_style()),
                            MarkdownOptions::default(),
                        )));
                    }
                    if after {
                        parts.push(Box::new(Spacer::new(1)));
                    }
                }
                _ => i += 1,
            }
        }
        self.has_tool_calls = content
            .iter()
            .any(|c| matches!(c, AssistantBlock::ToolCall(_)));
        let notice = |text: String| -> Box<dyn Component> {
            Box::new(Text::new(&theme::fg("error", &text), pad, 0))
        };
        if message.stop_reason == StopReason::Length {
            parts.push(Box::new(Spacer::new(1)));
            parts.push(notice("Response was truncated before completion.".into()));
        } else if !self.has_tool_calls {
            match message.stop_reason {
                StopReason::Aborted => {
                    let text = match message.error_message.as_deref() {
                        Some(m) if m != "Request was aborted" => m.to_string(),
                        _ => "Operation aborted".to_string(),
                    };
                    parts.push(Box::new(Spacer::new(1)));
                    parts.push(notice(text));
                }
                StopReason::Error => {
                    let text = message.error_message.as_deref().unwrap_or("Unknown error");
                    parts.push(Box::new(Spacer::new(1)));
                    parts.push(notice(format!("Error: {text}")));
                }
                _ => {}
            }
        }
        self.parts = parts;
    }
}

impl Component for AssistantView {
    fn render(&mut self, width: usize) -> Vec<String> {
        let lines: Vec<String> = self
            .parts
            .iter_mut()
            .flat_map(|p| p.render(width))
            .collect();
        if self.has_tool_calls || lines.is_empty() {
            return lines;
        }
        zone(lines)
    }
    fn invalidate(&mut self) {
        if let Some(message) = self.last.take() {
            self.update(&message);
        }
    }
}
