use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use pi_durable::cancel::CancelToken;
use pi_durable::harness::harness::Conversation;
use pi_tui::components::loader::Loader;
use pi_tui::components::text::Text;
use pi_tui::tui::Component;
use serde_json::Value;

use super::chat::Chat;
use super::theme::{bold, fg, fg_fn};
use crate::bash_exec::{self, ExitCode, Outcome, Step};
use crate::settings::SettingsManager;
use crate::tools::truncate::{Limits, truncate_tail};

/// Lines of output a collapsed block shows, as the tool rows do.
const PREVIEW_LINES: usize = 20;

struct Running {
    command: String,
    exclude_from_context: bool,
    cancel: CancelToken,
    rx: mpsc::Receiver<Step>,
    row: Option<Rc<RefCell<BashRow>>>,
}

impl Running {
    fn row(&mut self, chat: &mut Chat) -> Rc<RefCell<BashRow>> {
        let (command, exclude) = (&self.command, self.exclude_from_context);
        Rc::clone(self.row.get_or_insert_with(|| {
            chat.add_bash(BashRow::new(
                command,
                exclude,
                chat.output_pad,
                chat.expanded,
            ))
        }))
    }
}

/// `!` and `!!`: one command at a time, as upstream allows.
#[derive(Default)]
pub struct BashMode {
    running: Option<Running>,
}

impl BashMode {
    pub fn new() -> Self {
        BashMode { running: None }
    }

    /// The command a submitted line runs, and whether its output stays out of the model's context.
    pub fn parse(text: &str) -> Option<(&str, bool)> {
        let rest = text.strip_prefix('!')?;
        let (exclude, rest) = match rest.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let command = rest.trim();
        (!command.is_empty()).then_some((command, exclude))
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Runs the command with the shell `shellPath` and `shellCommandPrefix` name.
    pub fn start(
        &mut self,
        chat: &mut Chat,
        command: &str,
        exclude_from_context: bool,
        settings: &SettingsManager,
    ) {
        let (token, rx) = bash_exec::start_drawn(
            command,
            exclude_from_context,
            &chat.cwd,
            settings.shell_path().as_deref(),
            settings.shell_command_prefix(),
        );
        self.running = Some(Running {
            command: command.to_owned(),
            exclude_from_context,
            cancel: token,
            rx,
            row: None,
        });
    }

    /// `abortBash`: the command is killed and its block ends as cancelled.
    pub fn cancel(&mut self) -> bool {
        match &self.running {
            Some(running) => {
                running.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Takes what the thread produced; true when the screen changed. The run is recorded as an
    /// entry here, when it is over.
    pub fn poll(
        &mut self,
        chat: &mut Chat,
        conversation: &Conversation,
        cancel: &CancelToken,
    ) -> (bool, Option<String>) {
        let Some(mut running) = self.running.take() else {
            return (false, None);
        };
        let mut changed = false;
        let done = loop {
            match running.rx.try_recv() {
                Ok(Step::Started) => {
                    running.row(chat);
                    changed = true;
                }
                Ok(Step::Chunk(text)) => {
                    running.row(chat).borrow_mut().push(&text);
                    changed = true;
                }
                Ok(Step::Answered(outcome)) => {
                    running.row(chat).borrow_mut().push(&outcome.output);
                    break Ok(outcome);
                }
                Ok(Step::Refused(_)) => return (true, None),
                Ok(Step::Done(result)) => break result,
                Err(_) => {
                    self.running = Some(running);
                    return (changed, None);
                }
            }
        };
        let outcome = match done {
            Ok(outcome) => outcome,
            Err(e) => {
                if let Some(row) = &running.row {
                    row.borrow_mut().complete_missing();
                }
                return (true, Some(e));
            }
        };
        running.row(chat).borrow_mut().complete(&outcome);
        let (command, exclude) = (running.command, running.exclude_from_context);
        if !exclude {
            chat.drew_bash(&bash_exec::to_text(&command, &outcome));
        }
        let failure = bash_exec::record(conversation, cancel, &command, &outcome, exclude).err();
        (true, failure)
    }

    /// The spinner's next frame, as the status area's.
    pub fn tick(&mut self, now: u64) -> (bool, Option<u64>) {
        match &self.running {
            Some(Running { row: Some(row), .. }) => row.borrow_mut().tick(now),
            Some(Running { row: None, .. }) => (false, Some(now + 50)),
            None => (false, None),
        }
    }
}

#[derive(Default)]
enum State {
    #[default]
    Running,
    Complete {
        exit_code: Option<ExitCode>,
        cancelled: bool,
        truncated: bool,
        full_output_path: Option<String>,
    },
}

/// The bordered block a run draws: the command, its output, and how it ended.
pub struct BashRow {
    exclude_from_context: bool,
    expanded: bool,
    /// The command's stdout and stderr together, as the chunks arrived.
    output_text: String,
    header: Text,
    output: Text,
    status: Text,
    loader: Loader,
    state: State,
}

impl BashRow {
    pub fn new(
        command: &str,
        exclude_from_context: bool,
        output_pad: usize,
        expanded: bool,
    ) -> Self {
        let color = if exclude_from_context {
            "dim"
        } else {
            "bashMode"
        };
        BashRow {
            exclude_from_context,
            expanded,
            output_text: String::new(),
            header: Text::new(&fg(color, &bold(&format!("$ {command}"))), output_pad, 0),
            output: Text::new("", output_pad, 0),
            status: Text::new("", output_pad, 0),
            loader: Loader::new(
                fg_fn(color),
                fg_fn("muted"),
                "Running... (esc to cancel)",
                None,
            ),
            state: State::Running,
        }
    }

    /// The block a stored `bashExecution` entry held, for a session that is reopened.
    pub fn from_entry(data: &Value, output_pad: usize, expanded: bool) -> Option<BashRow> {
        let command = data.get("command")?.as_str()?;
        let mut row = BashRow::new(
            command,
            data.get("excludeFromContext")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            output_pad,
            expanded,
        );
        row.push(
            data.get("output")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        row.state = State::Complete {
            exit_code: data
                .get("exitCode")
                .cloned()
                .and_then(|code| serde_json::from_value(code).ok()),
            cancelled: data
                .get("cancelled")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            truncated: data
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            full_output_path: data
                .get("fullOutputPath")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        row.refresh();
        Some(row)
    }

    fn color(&self) -> &'static str {
        if self.exclude_from_context {
            "dim"
        } else {
            "bashMode"
        }
    }

    /// `appendOutput`: the run's output keeps arriving a chunk at a time.
    pub fn push(&mut self, chunk: &str) {
        let clean = pi_tui::utils::strip_terminal_sequences(chunk)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        self.output_text.push_str(&clean);
        self.refresh();
    }

    pub fn complete(&mut self, outcome: &Outcome) {
        self.loader.stop();
        self.state = State::Complete {
            exit_code: outcome.exit_code,
            cancelled: outcome.cancelled,
            truncated: outcome.truncated,
            full_output_path: outcome.full_output_path.clone(),
        };
        self.refresh();
    }

    pub fn complete_missing(&mut self) {
        self.loader.stop();
        self.state = State::Complete {
            exit_code: None,
            cancelled: false,
            truncated: false,
            full_output_path: None,
        };
        self.refresh();
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// Ticks the spinner; `(changed, next due)`.
    pub fn tick(&mut self, now: u64) -> (bool, Option<u64>) {
        match self.state {
            State::Running => self.loader.tick(now),
            State::Complete { .. } => (false, None),
        }
    }

    /// `updateDisplay`: the output as the model's copy of it is cut, one muted line each.
    fn refresh(&mut self) {
        let truncation = truncate_tail(&self.output_text, Limits::default());
        let text = if truncation.content.is_empty() {
            String::new()
        } else {
            format!(
                "\n{}",
                truncation
                    .content
                    .split('\n')
                    .map(|line| fg("muted", line))
                    .collect::<Vec<String>>()
                    .join("\n")
            )
        };
        self.output.set_text(&text);
    }

    fn status_parts(&self, hidden: usize) -> Vec<String> {
        let mut parts = Vec::new();
        if hidden > 0 {
            parts.push(if self.expanded {
                format!(
                    "{}{}{}",
                    fg("muted", "("),
                    fg("muted", "ctrl+o to collapse"),
                    fg("muted", ")")
                )
            } else {
                format!(
                    "{}{}{}",
                    fg("muted", &format!("... {hidden} more lines (")),
                    fg("muted", "ctrl+o to expand"),
                    fg("muted", ")")
                )
            });
        }
        let State::Complete {
            exit_code,
            cancelled,
            truncated,
            full_output_path,
        } = &self.state
        else {
            return parts;
        };
        if *cancelled {
            parts.push(fg("warning", "(cancelled)"));
        } else if let Some(code) = exit_code.filter(|code| *code != 0) {
            parts.push(fg("error", &format!("(exit {code})")));
        }
        if *truncated && let Some(path) = full_output_path {
            parts.push(fg(
                "warning",
                &format!("Output truncated. Full output: {path}"),
            ));
        }
        parts
    }
}

impl Component for BashRow {
    fn render(&mut self, width: usize) -> Vec<String> {
        let border = fg(self.color(), &"─".repeat(width));
        let mut lines = vec![String::new(), border.clone()];
        lines.extend(self.header.render(width));
        let shown = self.output.render(width);
        let hidden = shown.len().saturating_sub(PREVIEW_LINES);
        if self.expanded {
            lines.extend(shown);
        } else {
            lines.extend(shown.into_iter().skip(hidden));
        }
        match self.state {
            State::Running => lines.extend(self.loader.render(width)),
            State::Complete { .. } => {
                let parts = self.status_parts(hidden);
                self.status.set_text(&if parts.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", parts.join("\n"))
                });
                lines.extend(self.status.render(width));
            }
        }
        lines.push(border);
        lines
    }

    fn invalidate(&mut self) {
        self.header.invalidate();
        self.output.invalidate();
        self.status.invalidate();
        self.loader.invalidate();
    }
}
