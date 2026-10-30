//! The /tree overlay (upstream `modes/interactive/components/tree-selector.ts`): the session's
//! entries as a foldable tree, filtered, searched and selected to move the session's leaf.

use std::collections::{BTreeSet, HashMap};

use pi_durable::ai::types::{AssistantBlock, Message, StopReason, UserBlock, UserContent};
use pi_durable::entries::{compaction_entry, context_edit_entry, reset_entry, system_entry};
use pi_durable::ids::EntryId;
use pi_durable::types::{ContextAction, EntryRecord};
use pi_tui::components::input::{Input, InputOptions};
use pi_tui::keybindings::get_keybindings;
use pi_tui::tui::Component;
use pi_tui::utils::{slice_by_column, truncate_to_width, visible_width};
use serde_json::Value;

use super::{ActionSlot, OverlayAction, border, fit, key_hint, push, wrap};
use crate::interactive::theme::{bold, fg};
use crate::session_tree::Tree;
use crate::sessions::drafts::LABEL_KIND;
use crate::sessions::export::now_ms;
use crate::sessions::{from_iso8601, iso8601};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

impl Filter {
    /// The treeFilterMode setting, whose unknown values read as default.
    pub fn from_setting(mode: &str) -> Filter {
        match mode {
            "no-tools" => Filter::NoTools,
            "user-only" => Filter::UserOnly,
            "labeled-only" => Filter::LabeledOnly,
            "all" => Filter::All,
            _ => Filter::Default,
        }
    }
}

const FILTERS: [Filter; 5] = [
    Filter::Default,
    Filter::NoTools,
    Filter::UserOnly,
    Filter::LabeledOnly,
    Filter::All,
];

struct Frame {
    node: usize,
    indent: usize,
    just_branched: bool,
    show_connector: bool,
    is_last: bool,
    gutters: Vec<(usize, bool)>,
    virtual_child: bool,
}

struct Row {
    node: usize,
    indent: usize,
    show_connector: bool,
    is_last: bool,
    gutters: Vec<(usize, bool)>,
    virtual_child: bool,
}

pub struct TreeSelector {
    tree: Tree,
    rows: Vec<Row>,
    selected: usize,
    filter: Filter,
    search: Input,
    folded: BTreeSet<EntryId>,
    leaf: Option<EntryId>,
    action: ActionSlot,
    max_visible: usize,
    multiple_roots: bool,
    tool_calls: HashMap<String, (String, serde_json::Map<String, Value>)>,
    label_times: bool,
    label_input: Option<(usize, Input)>,
}

impl TreeSelector {
    pub fn new(tree: Tree, max_visible: usize, action: ActionSlot, filter: Filter) -> Self {
        let mut tool_calls = HashMap::new();
        for node in &tree.nodes {
            let Some(Message::Assistant(assistant)) = node
                .entry
                .model
                .as_ref()
                .and_then(|messages| messages.first())
            else {
                continue;
            };
            for block in &assistant.content {
                if let AssistantBlock::ToolCall(call) = block {
                    tool_calls.insert(call.id.clone(), (call.name.clone(), call.arguments.clone()));
                }
            }
        }
        let leaf = tree.leaf;
        let mut selector = Self {
            tree,
            rows: Vec::new(),
            selected: 0,
            filter,
            search: Input::new(InputOptions::default()),
            folded: BTreeSet::new(),
            leaf,
            action,
            max_visible: max_visible.max(5),
            multiple_roots: false,
            tool_calls,
            label_times: false,
            label_input: None,
        };
        selector.refresh();
        selector
    }

    fn pre_order(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.tree.roots.iter().rev().copied().collect();
        while let Some(index) = stack.pop() {
            out.push(index);
            for child in self.tree.nodes[index].children.iter().rev() {
                stack.push(*child);
            }
        }
        out
    }

    fn is_leaf(&self, index: usize) -> bool {
        Some(self.tree.nodes[index].entry.id) == self.leaf
    }

    fn passes(&self, index: usize) -> bool {
        let entry = &self.tree.nodes[index].entry;
        let is_settings = entry.kind == context_edit_entry().kind()
            || entry.kind == reset_entry().kind()
            || entry.kind == system_entry().kind()
            || entry.kind == LABEL_KIND;
        let labeled = self.tree.nodes[index].label.is_some();
        if entry.kind == pi_durable::entries::user_entry().kind() {
            if self.filter != Filter::LabeledOnly || labeled {
                return self.matches_search(index);
            }
            return false;
        }
        let is_tool_result = entry.kind == pi_durable::entries::tool_result_entry().kind();
        if is_tool_result && self.filter == Filter::UserOnly {
            return false;
        }
        if entry.kind == pi_durable::entries::assistant_entry().kind()
            && !self.is_leaf(index)
            && !Self::has_text(entry)
        {
            let aborted = matches!(
                stop_reason(entry),
                Some(StopReason::Error | StopReason::Aborted)
            );
            if !aborted {
                return false;
            }
        }
        let visible = match self.filter {
            Filter::UserOnly => false,
            Filter::All => true,
            Filter::LabeledOnly => labeled,
            Filter::NoTools => !is_settings && !is_tool_result,
            Filter::Default => !is_settings,
        };
        visible && self.matches_search(index)
    }

    fn matches_search(&self, index: usize) -> bool {
        let query = self.search.value().trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let text = self.search_text(index).to_lowercase();
        query.split_whitespace().all(|token| text.contains(token))
    }

    fn search_text(&self, index: usize) -> String {
        let node = &self.tree.nodes[index];
        let entry = &node.entry;
        let mut parts: Vec<String> = node.label.iter().cloned().collect();
        parts.push(entry.kind.clone());
        for message in entry.model.iter().flatten() {
            match message {
                Message::User(user) => parts.push(user_text(&user.content)),
                Message::Assistant(assistant) => {
                    parts.push("assistant".to_owned());
                    parts.push(blocks_text(&assistant.content));
                }
                Message::ToolResult(result) => {
                    parts.push(result.tool_name.clone());
                    parts.push(user_blocks_text(&result.content));
                }
                Message::System(system) => {
                    parts.push("system".to_owned());
                    parts.push(match &system.content {
                        pi_durable::ai::types::SystemContent::Text(text) => text.clone(),
                        pi_durable::ai::types::SystemContent::Blocks(_) => String::new(),
                    });
                }
            }
        }
        parts.join(" ")
    }

    fn visible(&self) -> Vec<usize> {
        let mut hidden: BTreeSet<EntryId> = BTreeSet::new();
        if !self.folded.is_empty() {
            for index in self.pre_order() {
                let node = &self.tree.nodes[index];
                if let Some(parent) = node.parent
                    && (self.folded.contains(&self.tree.nodes[parent].entry.id)
                        || hidden.contains(&self.tree.nodes[parent].entry.id))
                {
                    hidden.insert(node.entry.id);
                }
            }
        }
        self.pre_order()
            .into_iter()
            .filter(|index| !hidden.contains(&self.tree.nodes[*index].entry.id))
            .filter(|index| self.passes(*index))
            .collect()
    }

    fn arrange(&self, visible: &[usize]) -> (Vec<Row>, bool) {
        if visible.is_empty() {
            return (Vec::new(), false);
        }
        let visible_set: BTreeSet<usize> = visible.iter().copied().collect();
        let nearest = |mut index: usize| -> Option<usize> {
            loop {
                let parent = self.tree.nodes[index].parent?;
                if visible_set.contains(&parent) {
                    return Some(parent);
                }
                index = parent;
            }
        };
        let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
        for index in visible {
            children.entry(nearest(*index)).or_default().push(*index);
        }
        let roots = children.get(&None).cloned().unwrap_or_default();
        let multiple_roots = roots.len() > 1;
        let mut rows: Vec<Row> = Vec::with_capacity(visible.len());
        let mut stack: Vec<Frame> = Vec::new();
        for (position, root) in roots.iter().enumerate().rev() {
            stack.push(Frame {
                node: *root,
                indent: usize::from(multiple_roots),
                just_branched: multiple_roots,
                show_connector: multiple_roots,
                is_last: position == roots.len() - 1,
                gutters: Vec::new(),
                virtual_child: multiple_roots,
            });
        }
        let mut ordered: Vec<Option<Row>> = (0..visible.len()).map(|_| None).collect();
        let position_of: HashMap<usize, usize> = visible
            .iter()
            .enumerate()
            .map(|(position, index)| (*index, position))
            .collect();
        while let Some(frame) = stack.pop() {
            if let Some(position) = position_of.get(&frame.node) {
                ordered[*position] = Some(Row {
                    node: frame.node,
                    indent: frame.indent,
                    show_connector: frame.show_connector,
                    is_last: frame.is_last,
                    gutters: frame.gutters.clone(),
                    virtual_child: frame.virtual_child,
                });
            }
            let kids = children.get(&Some(frame.node)).cloned().unwrap_or_default();
            let multiple = kids.len() > 1;
            let child_indent = if multiple || (frame.just_branched && frame.indent > 0) {
                frame.indent + 1
            } else {
                frame.indent
            };
            let display_indent = if multiple_roots {
                frame.indent.saturating_sub(1)
            } else {
                frame.indent
            };
            let mut child_gutters = frame.gutters.clone();
            if frame.show_connector && !frame.virtual_child {
                child_gutters.push((display_indent.saturating_sub(1), !frame.is_last));
            }
            for (position, child) in kids.iter().enumerate().rev() {
                stack.push(Frame {
                    node: *child,
                    indent: child_indent,
                    just_branched: multiple,
                    show_connector: multiple,
                    is_last: position == kids.len() - 1,
                    gutters: child_gutters.clone(),
                    virtual_child: false,
                });
            }
        }
        rows.extend(ordered.into_iter().flatten());
        (rows, multiple_roots)
    }

    fn refresh(&mut self) {
        let previous = self
            .rows
            .get(self.selected)
            .map(|row| self.tree.nodes[row.node].entry.id)
            .or(self.leaf);
        let visible = self.visible();
        let (rows, multiple_roots) = self.arrange(&visible);
        self.rows = rows;
        self.multiple_roots = multiple_roots;
        self.selected = self.nearest_index(previous);
    }

    fn nearest_index(&self, target: Option<EntryId>) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        let Some(target) = target else {
            return self.rows.len() - 1;
        };
        if let Some(position) = self
            .rows
            .iter()
            .position(|row| self.tree.nodes[row.node].entry.id == target)
        {
            return position;
        }
        let mut index = self
            .tree
            .nodes
            .iter()
            .position(|node| node.entry.id == target);
        while let Some(current) = index {
            let Some(parent) = self.tree.nodes[current].parent else {
                break;
            };
            if let Some(position) = self.rows.iter().position(|row| {
                self.tree.nodes[row.node].entry.id == self.tree.nodes[parent].entry.id
            }) {
                return position;
            }
            index = Some(parent);
        }
        self.rows.len() - 1
    }

    fn selected_entry(&self) -> Option<&EntryRecord> {
        let row = self.rows.get(self.selected)?;
        Some(&self.tree.nodes[row.node].entry)
    }

    fn foldable(&self, index: usize) -> bool {
        let has_children = self
            .rows
            .iter()
            .any(|row| self.tree.nodes[row.node].parent == Some(index));
        if !has_children {
            return false;
        }
        match self.tree.nodes[index].parent {
            None => true,
            Some(parent) => {
                self.rows
                    .iter()
                    .filter(|row| self.tree.nodes[row.node].parent == Some(parent))
                    .count()
                    > 1
            }
        }
    }

    fn status_labels(&self) -> String {
        let filter = match self.filter {
            Filter::Default => "",
            Filter::NoTools => " [no-tools]",
            Filter::UserOnly => " [user]",
            Filter::LabeledOnly => " [labeled]",
            Filter::All => " [all]",
        };
        let times = if self.label_times {
            " [+label time]"
        } else {
            ""
        };
        format!("{filter}{times}")
    }

    fn has_text(entry: &EntryRecord) -> bool {
        let Some(Message::Assistant(assistant)) = entry.model.as_ref().and_then(|m| m.first())
        else {
            return false;
        };
        assistant.content.iter().any(|block| match block {
            AssistantBlock::Text(text) => !text.text.trim().is_empty(),
            _ => false,
        })
    }

    fn labeled(&self, index: usize, selected: bool) -> String {
        let node = &self.tree.nodes[index];
        let Some(label) = &node.label else {
            return self.describe(index, selected);
        };
        let time = match &node.label_timestamp {
            Some(timestamp) if self.label_times => {
                fg("muted", &format!("{} ", label_time(timestamp)))
            }
            _ => String::new(),
        };
        format!(
            "{}{time}{}",
            fg("warning", &format!("[{label}] ")),
            self.describe(index, selected)
        )
    }

    fn set_label(&mut self, index: usize, label: Option<String>) {
        let node = &mut self.tree.nodes[index];
        node.label_timestamp = label.as_ref().map(|_| iso8601(now_ms()));
        node.label = label.clone();
        let target = node.entry.id;
        push(&self.action, OverlayAction::Label { target, label });
    }

    fn row_text(&self, row: &Row, selected: bool, width: usize) -> String {
        let display_indent = if self.multiple_roots {
            row.indent.saturating_sub(1)
        } else {
            row.indent
        };
        let connector_allowed = row.show_connector && !row.virtual_child;
        let connector_position = display_indent.saturating_sub(1);
        let mut prefix = String::new();
        for character in 0..display_indent * 3 {
            let level = character / 3;
            let within = character % 3;
            if let Some((_, shown)) = row.gutters.iter().find(|(position, _)| *position == level) {
                prefix.push_str(if within == 0 && *shown { "│" } else { " " });
            } else if connector_allowed && level == connector_position {
                if within == 0 {
                    prefix.push(if row.is_last { '└' } else { '├' });
                } else if within == 1 {
                    let id = self.tree.nodes[row.node].entry.id;
                    if self.folded.contains(&id) {
                        prefix.push('⊞');
                    } else if self.foldable(row.node) {
                        prefix.push('⊟');
                    } else {
                        prefix.push('─');
                    }
                } else {
                    prefix.push(' ');
                }
            } else {
                prefix.push(' ');
            }
        }
        let active = self
            .tree
            .active
            .contains(&self.tree.nodes[row.node].entry.id);
        let path_marker = if active {
            fg("accent", "• ")
        } else {
            String::new()
        };
        let body = format!(
            "{}{}{}",
            fg("dim", &prefix),
            path_marker,
            self.labeled(row.node, selected)
        );
        let anchor = visible_width(&fg("dim", &prefix)) + visible_width(&path_marker);
        let viewport = width.saturating_sub(2);
        let widest = (0..self.rows.len())
            .map(|i| visible_width(&self.labeled(self.rows[i].node, false)) + 4)
            .max()
            .unwrap_or(0);
        let max_scroll = widest.saturating_sub(viewport);
        let mut scroll = 0;
        if max_scroll > 0 && selected {
            let keep = (viewport / 3).clamp(4, 20);
            if anchor > viewport.saturating_sub(keep) {
                let context = (viewport / 4).clamp(2, 12);
                scroll = max_scroll.min(anchor.saturating_sub(context));
            }
        }
        let cursor = if selected {
            fg("accent", "› ")
        } else {
            "  ".to_owned()
        };
        let line = if scroll > 0 {
            format!("{cursor}{}", slice_by_column(&body, scroll, viewport, true))
        } else {
            format!("{cursor}{body}")
        };
        truncate_to_width(&line, width, "", false)
    }

    fn describe(&self, index: usize, selected: bool) -> String {
        let entry = &self.tree.nodes[index].entry;
        let normalize = |text: &str| text.replace(['\n', '\t'], " ").trim().to_owned();
        let result = if entry.kind == pi_durable::entries::user_entry().kind() {
            let text = entry
                .model
                .as_ref()
                .and_then(|messages| messages.first())
                .map(|message| match message {
                    Message::User(user) => user_text(&user.content),
                    _ => String::new(),
                })
                .unwrap_or_default();
            format!("{}{}", fg("accent", "user: "), normalize(&text))
        } else if entry.kind == pi_durable::entries::assistant_entry().kind() {
            let text = entry
                .model
                .as_ref()
                .and_then(|messages| messages.first())
                .map(|message| match message {
                    Message::Assistant(assistant) => blocks_text(&assistant.content),
                    _ => String::new(),
                })
                .unwrap_or_default();
            let text = normalize(&text);
            if !text.is_empty() {
                format!("{}{}", fg("success", "assistant: "), text)
            } else if matches!(stop_reason(entry), Some(StopReason::Aborted)) {
                format!(
                    "{}{}",
                    fg("success", "assistant: "),
                    fg("muted", "(aborted)")
                )
            } else if let Some(error) = error_message(entry) {
                let mut text: String = error.chars().take(80).collect();
                text = normalize(&text);
                format!("{}{}", fg("success", "assistant: "), fg("error", &text))
            } else {
                format!(
                    "{}{}",
                    fg("success", "assistant: "),
                    fg("muted", "(no content)")
                )
            }
        } else if entry.kind == pi_durable::entries::tool_result_entry().kind() {
            let result = entry
                .model
                .as_ref()
                .and_then(|messages| messages.first())
                .and_then(|message| match message {
                    Message::ToolResult(result) => Some(result),
                    _ => None,
                });
            match result {
                Some(result) => match self.tool_calls.get(&result.tool_call_id) {
                    Some((name, arguments)) => fg("muted", &tool_call_text(name, arguments)),
                    None => fg("muted", &format!("[{}]", result.tool_name)),
                },
                None => fg("muted", "[tool]"),
            }
        } else if entry.kind == compaction_entry().kind() {
            fg("border", "[compaction]")
        } else if entry.kind == context_edit_entry().kind() {
            let target = entry
                .edits
                .as_ref()
                .and_then(|edits| edits.first())
                .map(|edit| match edit.action {
                    ContextAction::Omit => format!("[context omit: {}]", edit.target),
                    ContextAction::Replace { .. } => format!("[context replace: {}]", edit.target),
                })
                .unwrap_or_else(|| "[context edit]".to_owned());
            fg("dim", &target)
        } else if entry.kind == reset_entry().kind() {
            fg("dim", "[reset]")
        } else if entry.kind == LABEL_KIND {
            let label = match entry.data.as_ref().map(|data| &data["label"]) {
                Some(Value::String(label)) => label.clone(),
                Some(Value::Null) | None => "(cleared)".to_owned(),
                Some(other) => other.to_string(),
            };
            fg("dim", &format!("[label: {label}]"))
        } else if entry.kind == system_entry().kind() {
            let text = entry
                .model
                .as_ref()
                .and_then(|messages| messages.first())
                .map(|message| match message {
                    Message::System(system) => match &system.content {
                        pi_durable::ai::types::SystemContent::Text(text) => text.clone(),
                        pi_durable::ai::types::SystemContent::Blocks(_) => String::new(),
                    },
                    _ => String::new(),
                })
                .unwrap_or_default();
            fg("dim", &format!("[system]: {}", normalize(&text)))
        } else {
            fg("dim", &format!("[{}]", entry.kind))
        };
        if selected { bold(&result) } else { result }
    }
}

fn stop_reason(entry: &EntryRecord) -> Option<StopReason> {
    match entry.model.as_ref().and_then(|m| m.first()) {
        Some(Message::Assistant(assistant)) => Some(assistant.stop_reason),
        _ => None,
    }
}

fn error_message(entry: &EntryRecord) -> Option<String> {
    match entry.model.as_ref().and_then(|m| m.first()) {
        Some(Message::Assistant(assistant)) => assistant.error_message.clone(),
        _ => None,
    }
}

fn user_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => user_blocks_text(blocks),
    }
}

fn user_blocks_text(blocks: &[UserBlock]) -> String {
    let mut text = String::new();
    for block in blocks {
        if let UserBlock::Text(content) = block {
            text.push_str(&content.text);
        }
    }
    text
}

fn blocks_text(blocks: &[AssistantBlock]) -> String {
    let mut text = String::new();
    for block in blocks {
        if let AssistantBlock::Text(content) = block {
            text.push_str(&content.text);
        }
    }
    text
}

fn tool_call_text(name: &str, arguments: &serde_json::Map<String, Value>) -> String {
    let path = arguments
        .get("path")
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = shorten_path(path);
    match name {
        "read" => format!("[read: {path}]"),
        "write" => format!("[write: {path}]"),
        "edit" => format!("[edit: {path}]"),
        "bash" => {
            let raw = arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            let command: String = raw
                .replace(['\n', '\t'], " ")
                .trim()
                .chars()
                .take(50)
                .collect();
            let tail = if raw.chars().count() > 50 { "..." } else { "" };
            format!("[bash: {command}{tail}]")
        }
        "grep" => {
            let pattern = arguments
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or("");
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .map(shorten_path)
                .unwrap_or_else(|| ".".to_owned());
            format!("[grep: /{pattern}/ in {path}]")
        }
        "find" => {
            let pattern = arguments
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or("");
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .map(shorten_path)
                .unwrap_or_else(|| ".".to_owned());
            format!("[find: {pattern} in {path}]")
        }
        "ls" => {
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .map(shorten_path)
                .unwrap_or_else(|| ".".to_owned());
            format!("[ls: {path}]")
        }
        other => {
            let args = Value::Object(arguments.clone()).to_string();
            let shown: String = args.chars().take(40).collect();
            let tail = if args.chars().count() > 40 { "..." } else { "" };
            format!("[{other}: {shown}{tail}]")
        }
    }
}

fn local_time(ms: i64) -> libc::tm {
    let seconds: libc::time_t = ms.div_euclid(1000);
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    unsafe { libc::localtime_r(&seconds, &mut tm) };
    tm
}

fn label_time(timestamp: &str) -> String {
    let Some(ms) = from_iso8601(timestamp) else {
        return "aN/NaN/NaN NaN:NaN".to_owned();
    };
    let date = local_time(ms);
    let now = local_time(now_ms().try_into().unwrap_or_default());
    let time = format!("{:02}:{:02}", date.tm_hour, date.tm_min);
    let (month, day) = (date.tm_mon + 1, date.tm_mday);
    if (date.tm_year, date.tm_mon, date.tm_mday) == (now.tm_year, now.tm_mon, now.tm_mday) {
        time
    } else if date.tm_year == now.tm_year {
        format!("{month}/{day} {time}")
    } else {
        let year = (date.tm_year + 1900).to_string();
        let short = &year[year.len().saturating_sub(2)..];
        format!("{short}/{month}/{day} {time}")
    }
}

fn shorten_path(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

/// `formatHelpKeys`: the first key of each binding, arrows for the arrow keys, a shared prefix
/// written once.
fn help_keys(ids: &[&str]) -> String {
    let kb = get_keybindings();
    let keys: Vec<String> = ids
        .iter()
        .filter_map(|id| kb.keys(id).first().cloned())
        .collect();
    if keys.is_empty() {
        return String::new();
    }
    let text = if keys.len() == 1 {
        keys[0].clone()
    } else {
        let split: Vec<(String, String)> = keys
            .iter()
            .map(|key| match key.rfind('+') {
                Some(at) => (key[..at + 1].to_owned(), key[at + 1..].to_owned()),
                None => (String::new(), key.clone()),
            })
            .collect();
        let prefix = split[0].0.clone();
        if !prefix.is_empty() && split.iter().all(|(head, _)| *head == prefix) {
            format!(
                "{prefix}{}",
                split
                    .iter()
                    .map(|(_, tail)| tail.as_str())
                    .collect::<Vec<_>>()
                    .join("/")
            )
        } else {
            keys.join("/")
        }
    };
    text.replace("pageUp", "pgup")
        .replace("pageDown", "pgdn")
        .replace("up", "↑")
        .replace("down", "↓")
        .replace("left", "←")
        .replace("right", "→")
}

fn help_item(ids: &[&str], label: &str, label_first: bool) -> String {
    let keys = help_keys(ids);
    if keys.is_empty() {
        return label.to_owned();
    }
    match label_first {
        true => format!("{label} {keys}"),
        false => format!("{keys} {label}"),
    }
}

fn help_line(width: usize) -> Vec<String> {
    let items = [
        help_item(&["tui.select.up", "tui.select.down"], "move", false),
        help_item(
            &["tui.editor.cursorLeft", "tui.editor.cursorRight"],
            "page",
            false,
        ),
        help_item(
            &["app.tree.foldOrUp", "app.tree.unfoldOrDown"],
            "branch",
            false,
        ),
        help_item(&["app.tree.editLabel"], "label", false),
        help_item(&["app.tree.toggleLabelTimestamp"], "label time", false),
        help_item(
            &[
                "app.tree.filter.default",
                "app.tree.filter.noTools",
                "app.tree.filter.userOnly",
                "app.tree.filter.labeledOnly",
                "app.tree.filter.all",
            ],
            "filters",
            true,
        ),
        help_item(
            &[
                "app.tree.filter.cycleForward",
                "app.tree.filter.cycleBackward",
            ],
            "cycle",
            true,
        ),
    ];
    let line = format!("  {}", items.join(" · "));
    wrap(&line, width)
        .into_iter()
        .map(|part| fg("muted", &part))
        .collect()
}

impl Component for TreeSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![
            String::new(),
            border(width),
            format!(" {}", bold("  Session Tree")),
        ];
        lines.extend(help_line(width));
        lines.push(truncate_to_width(
            &format!(
                "  {} {}",
                fg("muted", "Type to search:"),
                fg("accent", self.search.value())
            ),
            width,
            "",
            false,
        ));
        lines.push(border(width));
        lines.push(String::new());
        if let Some((_, input)) = &mut self.label_input {
            let fitted = |line: String| truncate_to_width(&line, width, "", false);
            lines.push(fitted(format!(
                "  {}",
                fg("muted", "Label (empty to remove):")
            )));
            for line in input.render(width.saturating_sub(2)) {
                lines.push(fitted(format!("  {line}")));
            }
            lines.push(fitted(format!(
                "  {}  {}",
                key_hint("tui.select.confirm", "save"),
                key_hint("tui.select.cancel", "cancel")
            )));
        } else if self.rows.is_empty() {
            lines.push(fg("muted", "  No entries found"));
            lines.push(fg("muted", &format!("  (0/0){}", self.status_labels())));
        } else {
            let start = self
                .selected
                .saturating_sub(self.max_visible / 2)
                .min(self.rows.len().saturating_sub(self.max_visible));
            let end = (start + self.max_visible).min(self.rows.len());
            for position in start..end {
                let row = &self.rows[position];
                let selected = position == self.selected;
                lines.push(self.row_text(row, selected, width));
            }
            lines.push(truncate_to_width(
                &fg(
                    "muted",
                    &format!(
                        "  ({}/{}){}",
                        self.selected + 1,
                        self.rows.len(),
                        self.status_labels()
                    ),
                ),
                width,
                "",
                false,
            ));
        }
        lines.push(String::new());
        lines.push(border(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.search.focused = focused;
        if let Some((_, input)) = &mut self.label_input {
            input.focused = focused;
        }
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if let Some((index, input)) = &mut self.label_input {
            if kb.matches(data, "tui.select.confirm") {
                let index = *index;
                let value = input.value().trim().to_owned();
                self.label_input = None;
                self.set_label(index, Some(value).filter(|value| !value.is_empty()));
            } else if kb.matches(data, "tui.select.cancel") {
                self.label_input = None;
            } else {
                input.handle_input(data);
            }
            return;
        }
        if kb.matches(data, "tui.select.up") {
            if !self.rows.is_empty() {
                self.selected = if self.selected == 0 {
                    self.rows.len() - 1
                } else {
                    self.selected - 1
                };
            }
            return;
        }
        if kb.matches(data, "tui.select.down") {
            if !self.rows.is_empty() {
                self.selected = if self.selected == self.rows.len() - 1 {
                    0
                } else {
                    self.selected + 1
                };
            }
            return;
        }
        if kb.matches(data, "app.tree.foldOrUp") {
            if let Some(row) = self.rows.get(self.selected) {
                let index = row.node;
                let id = self.tree.nodes[index].entry.id;
                if self.foldable(index) && !self.folded.contains(&id) {
                    self.folded.insert(id);
                    self.refresh();
                    return;
                }
            }
            self.selected = self.segment_start(true);
            return;
        }
        if kb.matches(data, "app.tree.unfoldOrDown") {
            if let Some(row) = self.rows.get(self.selected) {
                let id = self.tree.nodes[row.node].entry.id;
                if self.folded.contains(&id) {
                    self.folded.remove(&id);
                    self.refresh();
                    return;
                }
            }
            self.selected = self.segment_start(false);
            return;
        }
        if kb.matches(data, "tui.select.pageUp") || kb.matches(data, "tui.editor.cursorLeft") {
            self.selected = self.selected.saturating_sub(self.max_visible);
            return;
        }
        if kb.matches(data, "tui.select.pageDown") || kb.matches(data, "tui.editor.cursorRight") {
            self.selected =
                (self.selected + self.max_visible).min(self.rows.len().saturating_sub(1));
            return;
        }
        if kb.matches(data, "tui.select.confirm") {
            if let Some(entry) = self.selected_entry() {
                push(&self.action, OverlayAction::Navigate(entry.id));
            }
            return;
        }
        if kb.matches(data, "tui.select.cancel") {
            if !self.search.value().is_empty() {
                self.search.set_value("");
                self.folded.clear();
                self.refresh();
            } else {
                push(&self.action, OverlayAction::Close);
            }
            return;
        }
        let filter = if kb.matches(data, "app.tree.filter.default") {
            Some(Filter::Default)
        } else if kb.matches(data, "app.tree.filter.noTools") {
            Some(if self.filter == Filter::NoTools {
                Filter::Default
            } else {
                Filter::NoTools
            })
        } else if kb.matches(data, "app.tree.filter.userOnly") {
            Some(if self.filter == Filter::UserOnly {
                Filter::Default
            } else {
                Filter::UserOnly
            })
        } else if kb.matches(data, "app.tree.filter.labeledOnly") {
            Some(if self.filter == Filter::LabeledOnly {
                Filter::Default
            } else {
                Filter::LabeledOnly
            })
        } else if kb.matches(data, "app.tree.filter.all") {
            Some(if self.filter == Filter::All {
                Filter::Default
            } else {
                Filter::All
            })
        } else {
            None
        };
        if let Some(filter) = filter {
            self.filter = filter;
            self.folded.clear();
            self.refresh();
            return;
        }
        let position = FILTERS.iter().position(|f| *f == self.filter).unwrap_or(0);
        if kb.matches(data, "app.tree.filter.cycleForward") {
            self.filter = FILTERS[(position + 1) % FILTERS.len()];
            self.folded.clear();
            self.refresh();
            return;
        }
        if kb.matches(data, "app.tree.filter.cycleBackward") {
            self.filter = FILTERS[(position + FILTERS.len() - 1) % FILTERS.len()];
            self.folded.clear();
            self.refresh();
            return;
        }
        if kb.matches(data, "tui.editor.deleteCharBackward") {
            if !self.search.value().is_empty() {
                let mut value = self.search.value().to_owned();
                value.pop();
                self.search.set_value(&value);
                self.folded.clear();
                self.refresh();
            }
            return;
        }
        if kb.matches(data, "app.tree.editLabel") {
            if let Some(row) = self.rows.get(self.selected) {
                let index = row.node;
                let mut input = Input::new(InputOptions::default());
                if let Some(label) = &self.tree.nodes[index].label {
                    input.set_value(label);
                }
                input.focused = self.search.focused;
                self.label_input = Some((index, input));
            }
            return;
        }
        if kb.matches(data, "app.tree.toggleLabelTimestamp") {
            self.label_times = !self.label_times;
            return;
        }
        let control = data
            .chars()
            .any(|character| (character as u32) < 32 || (character as u32) == 0x7f);
        if !control && !data.is_empty() {
            let mut value = self.search.value().to_owned();
            value.push_str(data);
            self.search.set_value(&value);
            self.folded.clear();
            self.refresh();
        }
    }
}

impl TreeSelector {
    fn segment_start(&self, up: bool) -> usize {
        let Some(row) = self.rows.get(self.selected) else {
            return self.selected;
        };
        let positions: HashMap<usize, usize> = self
            .rows
            .iter()
            .enumerate()
            .map(|(position, row)| (row.node, position))
            .collect();
        let children = |index: usize| -> Vec<usize> {
            self.rows
                .iter()
                .filter(|row| self.tree.nodes[row.node].parent == Some(index))
                .map(|row| row.node)
                .collect()
        };
        let mut current = row.node;
        if up {
            loop {
                let Some(parent) = self.tree.nodes[current].parent else {
                    return positions.get(&current).copied().unwrap_or(self.selected);
                };
                if children(parent).len() > 1
                    && let Some(position) = positions.get(&current).copied()
                    && position < self.selected
                {
                    return position;
                }
                current = parent;
            }
        }
        loop {
            let kids = children(current);
            if kids.is_empty() {
                return positions.get(&current).copied().unwrap_or(self.selected);
            }
            if kids.len() > 1 {
                return positions.get(&kids[0]).copied().unwrap_or(self.selected);
            }
            current = kids[0];
        }
    }
}
