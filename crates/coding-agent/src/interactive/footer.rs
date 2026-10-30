//! `components/footer.ts`: the working directory and git branch, then token totals, cost,
//! context use and the model.

use pi_durable::ai::types::{AssistantBlock, Message, UserBlock, UserContent};
use pi_durable::ai::utils::text::{system_content_text, utf16_len};
use pi_tui::tui::Component;
use pi_tui::utils::{truncate_to_width, visible_width};

use super::theme::fg;

/// `formatTokens`.
pub fn format_tokens(count: u64) -> String {
    let n = count as f64;
    if count < 1000 {
        count.to_string()
    } else if count < 10_000 {
        format!("{:.1}k", n / 1000.0)
    } else if count < 1_000_000 {
        format!("{}k", (n / 1000.0).round())
    } else if count < 10_000_000 {
        format!("{:.1}M", n / 1_000_000.0)
    } else {
        format!("{}M", (n / 1_000_000.0).round())
    }
}

/// `formatCwdForFooter`: the home directory as `~`.
pub fn format_cwd(cwd: &str, home: Option<&str>) -> String {
    match home {
        Some(home) if !home.is_empty() => {
            let home = home.trim_end_matches('/');
            if cwd == home {
                "~".into()
            } else if let Some(rest) = cwd.strip_prefix(&format!("{home}/")) {
                format!("~/{rest}")
            } else {
                cwd.into()
            }
        }
        _ => cwd.into(),
    }
}

/// The branch `HEAD` names, from the nearest `.git` up the tree; `detached` for a bare commit.
pub fn git_branch(cwd: &str) -> Option<String> {
    let mut dir = std::path::PathBuf::from(cwd);
    loop {
        let git = dir.join(".git");
        let head = if git.is_dir() {
            Some(git.join("HEAD"))
        } else if git.is_file() {
            // A worktree: `gitdir: <path>`.
            let text = std::fs::read_to_string(&git).ok()?;
            let path = text.trim().strip_prefix("gitdir:")?.trim().to_string();
            Some(dir.join(path).join("HEAD"))
        } else {
            None
        };
        if let Some(head) = head {
            let text = std::fs::read_to_string(head).ok()?;
            return Some(match text.trim().strip_prefix("ref: refs/heads/") {
                Some(branch) => branch.to_string(),
                None => "detached".to_string(),
            });
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Token and cost totals over the session's replies.
#[derive(Default, Clone, Copy)]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

impl Totals {
    pub fn add(&mut self, usage: &pi_durable::ai::types::Usage) {
        self.input += usage.input;
        self.output += usage.output;
        self.cache_read += usage.cache_read;
        self.cache_write += usage.cache_write;
        self.cost += usage.cost.total;
    }
}

const ESTIMATED_IMAGE_CHARS: usize = 4800;

fn json_chars(value: &impl serde::Serialize) -> usize {
    serde_json::to_string(value).map_or(0, |json| utf16_len(&json))
}

fn user_chars(blocks: &[UserBlock]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            UserBlock::Text(text) => utf16_len(&text.text),
            UserBlock::Image(_) => ESTIMATED_IMAGE_CHARS,
        })
        .sum()
}

pub fn estimate_tokens(message: &Message) -> u64 {
    let chars = match message {
        Message::System(system) => {
            utf16_len(&system_content_text(&system.content))
                + system.sections.as_ref().map_or(0, |sections| {
                    sections
                        .0
                        .iter()
                        .filter_map(|(_, text)| text.as_deref())
                        .map(utf16_len)
                        .sum()
                })
                + system.tools_added.as_ref().map_or(0, json_chars)
        }
        Message::User(user) => match &user.content {
            UserContent::Text(text) => utf16_len(text),
            UserContent::Blocks(blocks) => user_chars(blocks),
        },
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .map(|block| match block {
                AssistantBlock::Text(text) => utf16_len(&text.text),
                AssistantBlock::Thinking(thinking) => utf16_len(&thinking.thinking),
                AssistantBlock::ToolCall(call) => {
                    utf16_len(&call.name) + json_chars(&call.arguments)
                }
            })
            .sum(),
        Message::ToolResult(result) => user_chars(&result.content),
    };
    chars.div_ceil(4) as u64
}

#[derive(Default, Clone)]
pub struct FooterState {
    pub cwd: String,
    pub branch: Option<String>,
    pub totals: Totals,
    pub context_tokens: Option<u64>,
    pub context_unknown: bool,
    pub context_window: u64,
    pub model_id: Option<String>,
    pub provider: Option<String>,
    pub reasoning: bool,
    pub thinking: String,
    pub provider_count: usize,
    pub auto_compact: bool,
}

pub struct Footer {
    pub state: FooterState,
}

impl Component for Footer {
    fn render(&mut self, width: usize) -> Vec<String> {
        let s = &self.state;
        let home = std::env::var("HOME").ok();
        let mut pwd = format_cwd(&s.cwd, home.as_deref());
        if let Some(branch) = &s.branch {
            pwd = format!("{pwd} ({branch})");
        }
        let t = &s.totals;
        let mut parts = Vec::new();
        if t.input > 0 {
            parts.push(format!("↑{}", format_tokens(t.input)));
        }
        if t.output > 0 {
            parts.push(format!("↓{}", format_tokens(t.output)));
        }
        if t.cache_read > 0 {
            parts.push(format!("R{}", format_tokens(t.cache_read)));
        }
        if t.cache_write > 0 {
            parts.push(format!("W{}", format_tokens(t.cache_write)));
        }
        if t.cost > 0.0 {
            parts.push(format!("${:.3}", t.cost));
        }
        let auto = if s.auto_compact { " (auto)" } else { "" };
        let window = format_tokens(s.context_window);
        let (percent_value, shown) = match s.context_tokens {
            _ if s.context_unknown => (0.0, format!("?/{window}{auto}")),
            Some(tokens) if s.context_window > 0 => {
                let p = tokens as f64 * 100.0 / s.context_window as f64;
                (p, format!("{p:.1}%/{window}{auto}"))
            }
            _ => (0.0, format!("0.0%/{window}{auto}")),
        };
        parts.push(if percent_value > 90.0 {
            fg("error", &shown)
        } else if percent_value > 70.0 {
            fg("warning", &shown)
        } else {
            shown
        });
        let mut left = parts.join(" ");
        let mut left_width = visible_width(&left);
        if left_width > width {
            left = truncate_to_width(&left, width, "...", false);
            left_width = visible_width(&left);
        }
        let model = s.model_id.clone().unwrap_or_else(|| "no-model".into());
        let mut right_plain = model.clone();
        if s.reasoning {
            right_plain = if s.thinking == "off" {
                format!("{model} • thinking off")
            } else {
                format!("{model} • {}", s.thinking)
            };
        }
        let min_padding = 2;
        let mut right = right_plain.clone();
        if s.provider_count > 1
            && let Some(provider) = &s.provider
        {
            right = format!("({provider}) {right_plain}");
            if left_width + min_padding + visible_width(&right) > width {
                right = right_plain;
            }
        }
        let right_width = visible_width(&right);
        let remainder = if left_width + min_padding + right_width <= width {
            format!("{}{right}", " ".repeat(width - left_width - right_width))
        } else if width > left_width + min_padding {
            let cut = truncate_to_width(&right, width - left_width - min_padding, "", false);
            format!(
                "{}{cut}",
                " ".repeat(width.saturating_sub(left_width + visible_width(&cut)))
            )
        } else {
            String::new()
        };
        let pwd_line = truncate_to_width(&fg("dim", &pwd), width, &fg("dim", "..."), false);
        vec![pwd_line, fg("dim", &left) + &fg("dim", &remainder)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_format_like_upstream() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1234), "1.2k");
        assert_eq!(format_tokens(200_000), "200k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
        assert_eq!(format_tokens(12_000_000), "12M");
    }

    #[test]
    fn cwd_under_home_uses_tilde() {
        assert_eq!(format_cwd("/home/u/src", Some("/home/u")), "~/src");
        assert_eq!(format_cwd("/home/u", Some("/home/u")), "~");
        assert_eq!(format_cwd("/home/uu", Some("/home/u")), "/home/uu");
    }

    #[test]
    fn stats_line_right_aligns_the_model() {
        let mut footer = Footer {
            state: FooterState {
                cwd: "/w".into(),
                context_window: 200_000,
                model_id: Some("m".into()),
                reasoning: true,
                thinking: "high".into(),
                auto_compact: true,
                ..FooterState::default()
            },
        };
        let lines: Vec<String> = footer
            .render(40)
            .iter()
            .map(|l| pi_tui::utils::strip_terminal_sequences(l))
            .collect();
        assert_eq!(lines[0], "/w");
        assert_eq!(
            lines[1],
            format!("0.0%/200k (auto){}m • high", " ".repeat(40 - 16 - 8))
        );
    }
}
