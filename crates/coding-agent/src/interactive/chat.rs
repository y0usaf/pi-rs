//! The chat: `interactive-mode.ts`'s `handleEvent` and `renderSessionContext` on pi-durable's
//! agent events. It owns the chat container's children and the footer's numbers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pi_durable::ai::types::{
    AssistantBlock, AssistantMessage, Message, StopReason, UserBlock, UserContent,
};
use pi_durable::harness::events::{AgentEvent, MessageChange, SnapshotEvent, ToolOutput};
use pi_durable::harness::usage::UsageState;
use pi_durable::types::EntryRecord;
use pi_tui::components::markdown::{Markdown, MarkdownOptions};
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::tui::{ComponentRef, Container, same};
use serde_json::Value;

use super::bash_mode::BashRow;
use super::footer::Totals;
use super::messages::{AssistantView, CustomMessage, UserMessage};
use super::theme;
use super::theme::fg;
use super::tool_row::{ShownResult, ToolRow};

pub struct Chat {
    pub container: Rc<RefCell<Container>>,
    streaming: Option<(Rc<RefCell<AssistantView>>, AssistantMessage)>,
    rows: HashMap<String, Rc<RefCell<ToolRow>>>,
    all_rows: Vec<Rc<RefCell<ToolRow>>>,
    bash_rows: Vec<Rc<RefCell<BashRow>>>,
    /// The model's text for a `!` run this chat already drew as a block.
    bash_texts: Vec<String>,
    views: Vec<Rc<RefCell<AssistantView>>>,
    /// The spacer and line of the last status shown, which a second status updates in place.
    status_line: Option<(ComponentRef, Rc<RefCell<Text>>)>,
    pub cwd: String,
    pub expanded: bool,
    pub hide_thinking: bool,
    pub output_pad: usize,
    /// A run is going: the working indicator shows and Escape aborts.
    pub working: bool,
    pub totals: Totals,
    pub context_tokens: Option<u64>,
    /// A line for the status area (retries, compaction), cleared when the run ends.
    pub status: Option<String>,
}

fn user_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(t) => t.clone(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                UserBlock::Text(t) => Some(t.text.as_str()),
                UserBlock::Image(_) => None,
            })
            .collect(),
    }
}

fn totals_of(usage: &UsageState) -> Totals {
    let mut totals = Totals::default();
    for u in usage.models.values() {
        totals.add(u);
    }
    totals
}

impl Chat {
    pub fn new(cwd: &str) -> Self {
        Chat {
            container: Rc::new(RefCell::new(Container::new())),
            streaming: None,
            rows: HashMap::new(),
            all_rows: Vec::new(),
            bash_rows: Vec::new(),
            bash_texts: Vec::new(),
            views: Vec::new(),
            status_line: None,
            cwd: cwd.to_string(),
            expanded: false,
            hide_thinking: false,
            output_pad: 1,
            working: false,
            totals: Totals::default(),
            context_tokens: None,
            status: None,
        }
    }

    /// Adds a `!` run's block to the transcript, and keeps it for Ctrl+O.
    pub fn add_bash(&mut self, row: BashRow) -> Rc<RefCell<BashRow>> {
        let row = Rc::new(RefCell::new(row));
        self.add(row.clone());
        self.bash_rows.push(Rc::clone(&row));
        row
    }

    /// The model's text for a run this chat drew as a block: the entry carries it for the next
    /// request, and the chat does not draw it again as the user's own message.
    pub fn drew_bash(&mut self, model_text: &str) {
        self.bash_texts.push(model_text.to_owned());
    }

    fn add(&mut self, c: ComponentRef) {
        self.container.borrow_mut().add_child(c);
    }

    fn is_empty(&self) -> bool {
        self.container.borrow().children.is_empty()
    }

    /// A line in the chat, as upstream's `showError` / `showStatus` add them.
    pub fn notice(&mut self, token: &str, text: &str) {
        self.add(Rc::new(RefCell::new(Spacer::new(1))));
        let line = Text::new(&fg(token, text), 1, 0);
        self.add(Rc::new(RefCell::new(line)));
    }

    /// `showExtensionError`: what a handler threw, then its stack below the line that repeats the
    /// message, dimmed.
    pub fn extension_error(&mut self, path: &str, error: &str, stack: Option<&str>) {
        let line = Text::new(
            &fg("error", &format!("Extension \"{path}\" error: {error}")),
            1,
            0,
        );
        self.add(Rc::new(RefCell::new(line)));
        let frames: Vec<String> = stack
            .into_iter()
            .flat_map(|stack| stack.split('\n').skip(1))
            .map(|line| fg("dim", &format!("  {}", line.trim())))
            .collect();
        if !frames.is_empty() {
            let text = Text::new(&frames.join("\n"), 1, 0);
            self.add(Rc::new(RefCell::new(text)));
        }
    }

    /// A status message in the chat. Back-to-back statuses update the line already there
    /// instead of appending more, as upstream's `showStatus` does.
    pub fn show_status(&mut self, message: &str) {
        let text = fg("dim", message);
        if let Some((spacer, line)) = &self.status_line {
            let children = &self.container.borrow().children;
            let count = children.len();
            let line_ref: ComponentRef = line.clone();
            if count >= 2
                && same(&children[count - 1], &line_ref)
                && same(&children[count - 2], spacer)
            {
                line.borrow_mut().set_text(&text);
                return;
            }
        }
        let spacer: ComponentRef = Rc::new(RefCell::new(Spacer::new(1)));
        let line = Rc::new(RefCell::new(Text::new(&text, 1, 0)));
        self.add(Rc::clone(&spacer));
        let line_ref: ComponentRef = line.clone();
        self.add(line_ref);
        self.status_line = Some((spacer, line));
    }

    /// A block of Markdown in the chat, as upstream's `/hotkeys` and `/session` add them.
    pub fn block(&mut self, markdown: &str) {
        self.add(Rc::new(RefCell::new(Spacer::new(1))));
        self.add(Rc::new(RefCell::new(Markdown::new(
            markdown,
            self.output_pad,
            1,
            theme::markdown(),
            None,
            MarkdownOptions::default(),
        ))));
    }

    /// Empties the transcript, for a new session.
    pub fn clear(&mut self) {
        self.container.borrow_mut().clear();
        self.rows.clear();
        self.all_rows.clear();
        self.bash_rows.clear();
        self.bash_texts.clear();
        self.views.clear();
        self.status_line = None;
        self.streaming = None;
    }

    /// Hide or show the thinking blocks of every assistant message already in the transcript.
    pub fn set_hide_thinking(&mut self, hide: bool) {
        self.hide_thinking = hide;
        for view in &self.views {
            view.borrow_mut().set_hide_thinking(hide);
        }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        for row in &self.all_rows {
            row.borrow_mut().set_expanded(expanded);
        }
        for row in &self.bash_rows {
            row.borrow_mut().set_expanded(expanded);
        }
    }

    /// Ticks the elapsed timers of running rows; true when one is running.
    pub fn tick_rows(&mut self) -> bool {
        let mut any = false;
        for row in self.rows.values() {
            let mut r = row.borrow_mut();
            if r.is_running() {
                r.tick();
                any = true;
            }
        }
        any
    }

    fn add_user(&mut self, content: &UserContent) {
        let text = user_text(content);
        if text.is_empty() {
            return;
        }
        if !self.is_empty() {
            self.add(Rc::new(RefCell::new(Spacer::new(1))));
        }
        self.add(Rc::new(RefCell::new(UserMessage::new(
            &text,
            self.output_pad,
        ))));
    }

    fn add_custom(&mut self, entry: &EntryRecord) -> bool {
        let Some(data) = entry.data.as_ref() else {
            return false;
        };
        if data.get("display").and_then(Value::as_bool) != Some(true) {
            return false;
        }
        let label = data
            .get("customType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = match data.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        self.add(Rc::new(RefCell::new(CustomMessage::new(
            label,
            &text,
            self.output_pad,
        ))));
        true
    }

    fn row(
        &mut self,
        id: &str,
        name: &str,
        args: &serde_json::Map<String, Value>,
    ) -> Rc<RefCell<ToolRow>> {
        if let Some(row) = self.rows.get(id) {
            return Rc::clone(row);
        }
        let row = Rc::new(RefCell::new(ToolRow::new(
            name,
            id,
            args.clone(),
            &self.cwd,
            self.expanded,
        )));
        self.add(row.clone());
        self.rows.insert(id.to_string(), Rc::clone(&row));
        self.all_rows.push(Rc::clone(&row));
        row
    }

    /// Rows for the message's tool calls, created as their blocks appear.
    fn sync_tool_calls(&mut self, message: &AssistantMessage) {
        for block in &message.content {
            if let AssistantBlock::ToolCall(call) = block {
                let known = self.rows.contains_key(&call.id);
                let row = self.row(&call.id, &call.name, &call.arguments);
                if known {
                    row.borrow_mut().set_args(call.arguments.clone());
                }
            }
        }
    }

    fn start_assistant(&mut self, message: &AssistantMessage) {
        let view = Rc::new(RefCell::new(AssistantView::new(
            self.hide_thinking,
            self.output_pad,
        )));
        self.add(view.clone());
        view.borrow_mut().update(message);
        self.views.push(Rc::clone(&view));
        self.streaming = Some((view, message.clone()));
        self.sync_tool_calls(message);
    }

    fn end_assistant(&mut self, message: &AssistantMessage) {
        match self.streaming.take() {
            Some((view, _)) => view.borrow_mut().update(message),
            None => {
                let view = Rc::new(RefCell::new(AssistantView::with_message(
                    message,
                    self.hide_thinking,
                    self.output_pad,
                )));
                self.add(view.clone());
                self.views.push(view);
            }
        }
        self.sync_tool_calls(message);
        if message.stop_reason != StopReason::Aborted && message.stop_reason != StopReason::Error {
            let u = &message.usage;
            self.context_tokens = Some(u.input + u.output + u.cache_read + u.cache_write);
        }
        if matches!(message.stop_reason, StopReason::Aborted | StopReason::Error) {
            let text = message
                .error_message
                .clone()
                .unwrap_or_else(|| "Error".into());
            for block in &message.content {
                if let AssistantBlock::ToolCall(call) = block
                    && let Some(row) = self.rows.remove(&call.id)
                {
                    row.borrow_mut().set_result(ShownResult {
                        text: text.clone(),
                        is_error: true,
                        ..ShownResult::default()
                    });
                }
            }
        }
    }

    fn tool_result(&mut self, message: &Message) {
        if let Message::ToolResult(result) = message
            && let Some(row) = self.rows.remove(&result.tool_call_id)
        {
            row.borrow_mut()
                .set_result(ShownResult::from_message(result));
        }
    }

    /// Shows a stored message, as `addMessageToChat` does when a session is loaded.
    fn show(&mut self, message: &Message) {
        match message {
            Message::User(u) => self.add_user(&u.content),
            Message::Assistant(a) => self.end_assistant(a),
            Message::ToolResult(_) => self.tool_result(message),
            Message::System(_) => {}
        }
    }

    fn entry_messages(entry: &EntryRecord) -> &[Message] {
        entry.model.as_deref().unwrap_or(&[])
    }

    /// Redraws the whole chat from a snapshot: the stored conversation, then whatever is live.
    pub fn load(&mut self, snapshot: &SnapshotEvent) {
        self.container.borrow_mut().clear();
        self.rows.clear();
        self.all_rows.clear();
        self.bash_rows.clear();
        self.views.clear();
        self.status_line = None;
        self.streaming = None;
        for entry in &snapshot.entries {
            if entry.kind == crate::sessions::drafts::MESSAGE_KIND {
                self.add_custom(entry);
                continue;
            }
            if entry.kind == crate::bash_exec::KIND {
                if let Some(data) = &entry.data
                    && let Some(row) = BashRow::from_entry(data, self.output_pad, self.expanded)
                {
                    self.add_bash(row);
                }
                continue;
            }
            for message in Self::entry_messages(entry) {
                self.show(message);
            }
        }
        // Calls without a stored result are still pending; their live slots say which run.
        for slot in &snapshot.tools {
            let row = self.row(&slot.call_id, &slot.name, &serde_json::Map::new());
            let mut r = row.borrow_mut();
            r.start();
            if let Some(output) = &slot.output {
                r.set_partial(output.clone());
            }
            if let Some(partial) = &slot.partial {
                r.set_partial(partial_text(&partial.value));
            }
        }
        if let Some(message) = snapshot
            .generation
            .as_ref()
            .and_then(|g| g.message.as_ref())
        {
            self.start_assistant(message);
        }
        self.working = snapshot.run.is_some();
        self.totals = totals_of(&snapshot.usage);
        self.context_tokens = None;
        self.status = None;
    }

    fn apply_change(message: &mut AssistantMessage, change: &MessageChange) {
        let content = &mut message.content;
        match change {
            MessageChange::TextStart {
                content_index,
                block,
            }
            | MessageChange::ThinkingStart {
                content_index,
                block,
            }
            | MessageChange::ToolcallStart {
                content_index,
                block,
            } => {
                let at = (*content_index).min(content.len());
                content.insert(at, (**block).clone());
            }
            MessageChange::TextDelta {
                content_index,
                delta,
            } => {
                if let Some(AssistantBlock::Text(t)) = content.get_mut(*content_index) {
                    t.text.push_str(delta);
                }
            }
            MessageChange::ThinkingDelta {
                content_index,
                delta,
            } => {
                if let Some(AssistantBlock::Thinking(t)) = content.get_mut(*content_index) {
                    t.thinking.push_str(delta);
                }
            }
            MessageChange::ToolcallDelta {
                content_index,
                path,
                delta,
            } => {
                if let Some(AssistantBlock::ToolCall(call)) = content.get_mut(*content_index) {
                    append_at(&mut call.arguments, path, delta);
                }
            }
            MessageChange::Block {
                content_index,
                block,
            } => {
                if let Some(slot) = content.get_mut(*content_index) {
                    *slot = (**block).clone();
                } else {
                    content.push((**block).clone());
                }
            }
            MessageChange::Message { message: whole } => *message = (**whole).clone(),
        }
    }

    /// One agent event. Returns true when the screen should redraw.
    pub fn apply(&mut self, event: &AgentEvent) -> bool {
        match event {
            AgentEvent::Snapshot(snapshot) => self.load(snapshot),
            AgentEvent::RunStart { .. } => self.working = true,
            AgentEvent::RunEnd { .. } => {
                self.working = false;
                self.status = None;
            }
            AgentEvent::MessageStart {
                message: Message::Assistant(a),
            } => self.start_assistant(a),
            AgentEvent::MessageUpdate { usage, changes } => {
                let Some((view, mut message)) = self.streaming.take() else {
                    return false;
                };
                for change in changes {
                    Self::apply_change(&mut message, change);
                }
                message.usage = usage.clone();
                view.borrow_mut().update(&message);
                self.sync_tool_calls(&message);
                self.streaming = Some((view, message));
            }
            AgentEvent::MessageEnd { entry } => {
                if entry.kind == crate::sessions::drafts::MESSAGE_KIND {
                    return self.add_custom(entry);
                }
                for message in Self::entry_messages(entry) {
                    match message {
                        Message::User(u) => {
                            let text = user_text(&u.content);
                            if let Some(at) =
                                self.bash_texts.iter().position(|drawn| *drawn == text)
                            {
                                self.bash_texts.remove(at);
                                return false;
                            }
                            self.add_user(&u.content);
                        }
                        Message::Assistant(a) => self.end_assistant(a),
                        Message::ToolResult(_) => self.tool_result(message),
                        _ => {}
                    }
                }
            }
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let row = self.row(tool_call_id, tool_name, args);
                let mut r = row.borrow_mut();
                if !args.is_empty() {
                    r.set_args(args.clone());
                }
                r.start();
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial: Some(partial),
                ..
            } => {
                let Some(row) = self.rows.get(tool_call_id) else {
                    return false;
                };
                row.borrow_mut().set_partial(partial_text(partial));
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                output: Some(output),
                ..
            } => {
                let Some(row) = self.rows.get(tool_call_id) else {
                    return false;
                };
                let mut r = row.borrow_mut();
                let next = match output {
                    ToolOutput::Set { set } => set.clone(),
                    ToolOutput::Edit { trim_start, append } => {
                        let current = r.partial().unwrap_or_default();
                        let skip = trim_start.unwrap_or(0);
                        // Counted in UTF-16 units, as chord's Truncate counts them.
                        let units: Vec<u16> = current.encode_utf16().skip(skip).collect();
                        let kept = String::from_utf16_lossy(&units);
                        kept + append.as_deref().unwrap_or("")
                    }
                };
                r.set_partial(next);
            }
            AgentEvent::ToolExecutionEnd {
                entry: Some(entry), ..
            } => {
                for message in Self::entry_messages(entry) {
                    self.tool_result(message);
                }
            }
            AgentEvent::UsageChanged { usage } => self.totals = totals_of(usage),
            AgentEvent::AutoRetryStart {
                attempt,
                error_message,
                ..
            } => {
                self.status = Some(format!(
                    "Retrying (attempt {attempt}) after: {error_message}"
                ));
            }
            AgentEvent::AutoRetryEnd { .. } => self.status = None,
            AgentEvent::CompactionStart { .. } => {
                self.status = Some("Compacting context...".into())
            }
            AgentEvent::CompactionEnd { .. } => self.status = None,
            AgentEvent::TaskFailed { kind, message, .. } => {
                self.notice("error", &format!("Error ({kind}): {message}"))
            }
            _ => return false,
        }
        true
    }
}

fn partial_text(partial: &Value) -> String {
    partial
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Appends `delta` to the string at `path` in a streamed tool call's arguments.
fn append_at(args: &mut serde_json::Map<String, Value>, path: &[Value], delta: &str) {
    let Some((Value::String(first), rest)) = path.split_first() else {
        return;
    };
    let mut node = args
        .entry(first.clone())
        .or_insert_with(|| Value::String(String::new()));
    for seg in rest {
        node = match (seg, node) {
            (Value::String(k), Value::Object(o)) => o
                .entry(k.clone())
                .or_insert_with(|| Value::String(String::new())),
            (Value::Number(i), Value::Array(a)) => {
                match i.as_u64().and_then(|i| a.get_mut(i as usize)) {
                    Some(v) => v,
                    None => return,
                }
            }
            _ => return,
        };
    }
    if let Value::String(s) = node {
        s.push_str(delta);
    }
}
