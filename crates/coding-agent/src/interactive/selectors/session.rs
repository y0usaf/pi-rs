//! The /resume overlay and the --resume picker (upstream modes/interactive/components/session-selector.ts
//! and session-selector-search.ts): the sessions in this folder or in all of them, sorted threaded,
//! recent or by relevance, filtered to the named ones, searchable with words, "phrases" and re:
//! patterns, with delete and rename.

use pi_tui::components::input::{Input, InputOptions};
use pi_tui::fuzzy::fuzzy_match;
use pi_tui::keybindings::get_keybindings;
use pi_tui::terminal::Terminal;
use pi_tui::tui::Component;
use pi_tui::utils::{truncate_to_width, visible_width};

use super::{
    ActionSlot, OverlayAction, border, fit, key_display, key_hint, push, scroll_info, visible_range,
};
use crate::interactive::theme::{bg, bold, fg};
use crate::session_list::{self, SessionEntry};

/// Upstream's SortMode.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortMode {
    Threaded,
    Recent,
    Relevance,
}

impl SortMode {
    fn label(self) -> &'static str {
        match self {
            SortMode::Threaded => "Threaded",
            SortMode::Recent => "Recent",
            SortMode::Relevance => "Fuzzy",
        }
    }

    fn next(self) -> Self {
        match self {
            SortMode::Threaded => SortMode::Recent,
            SortMode::Recent => SortMode::Relevance,
            SortMode::Relevance => SortMode::Threaded,
        }
    }
}

/// One row of the list: a session and where it sits in the fork tree.
struct Row {
    index: usize,
    depth: usize,
    is_last: bool,
    ancestor_continues: Vec<bool>,
}

fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

/// formatSessionDate: an age as the picker shows it.
fn format_age(modified: std::time::SystemTime) -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(modified)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let minutes = seconds / 60;
    let hours = seconds / 3_600;
    let days = seconds / 86_400;
    if minutes < 1 {
        "now".to_owned()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if hours < 24 {
        format!("{hours}h")
    } else if days < 7 {
        format!("{days}d")
    } else if days < 30 {
        format!("{}w", days / 7)
    } else if days < 365 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

/// normalizeWhitespaceLower.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

enum Token {
    Fuzzy(String),
    Phrase(String),
}

enum Query {
    Tokens(Vec<Token>),
    /// None when the pattern did not parse, which matches nothing as upstream does.
    Regex(Option<regex::Regex>),
}

/// parseSearchQuery: re:<pattern>, words, and "phrases"; unbalanced quotes fall back to words.
fn parse_query(query: &str) -> Query {
    let trimmed = query.trim();
    if let Some(pattern) = trimmed.strip_prefix("re:") {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Query::Regex(None);
        }
        return Query::Regex(
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .ok(),
        );
    }
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    let mut quoted = false;
    let mut unclosed = false;
    fn flush(buffer: &mut String, quoted: bool, tokens: &mut Vec<Token>) {
        let value = buffer.trim().to_owned();
        buffer.clear();
        if value.is_empty() {
            return;
        }
        tokens.push(if quoted {
            Token::Phrase(value)
        } else {
            Token::Fuzzy(value)
        });
    }
    for ch in trimmed.chars() {
        if ch == '"' {
            flush(&mut buffer, quoted, &mut tokens);
            quoted = !quoted;
            continue;
        }
        if !quoted && ch.is_whitespace() {
            flush(&mut buffer, false, &mut tokens);
            continue;
        }
        buffer.push(ch);
    }
    if quoted {
        unclosed = true;
    }
    if unclosed {
        return Query::Tokens(
            trimmed
                .split_whitespace()
                .map(|word| Token::Fuzzy(word.to_owned()))
                .collect(),
        );
    }
    flush(&mut buffer, quoted, &mut tokens);
    Query::Tokens(tokens)
}

const TIER_NAME: f64 = 0.0;
const TIER_TEXT: f64 = 1_000_000.0;
const TIER_FUZZY: f64 = 2_000_000.0;

fn literal_score(entry: &SessionEntry, value: &str) -> Option<f64> {
    let name = entry.name.as_deref().unwrap_or_default().to_lowercase();
    if name.contains(value) {
        return Some(TIER_NAME + fuzzy_match(value, &name).score);
    }
    if entry.id.to_lowercase().contains(value)
        || entry.all_text.to_lowercase().contains(value)
        || entry.cwd.to_lowercase().contains(value)
    {
        return Some(TIER_TEXT);
    }
    None
}

fn fuzzy_score(entry: &SessionEntry, value: &str) -> Option<f64> {
    let value = value.to_lowercase();
    if let Some(score) = literal_score(entry, &value) {
        return Some(score);
    }
    let mut best: Option<f64> = None;
    for field in [entry.name.clone().unwrap_or_default(), entry.cwd.clone()] {
        let found = fuzzy_match(&value, &field);
        if found.matches && best.is_none_or(|b| found.score < b) {
            best = Some(found.score);
        }
    }
    best.map(|score| TIER_FUZZY + score)
}

fn phrase_score(entry: &SessionEntry, phrase: &str) -> Option<f64> {
    let name = normalize(entry.name.as_deref().unwrap_or_default());
    if name.contains(phrase) {
        return Some(TIER_NAME + fuzzy_match(phrase, &name).score);
    }
    if entry.id.to_lowercase().contains(phrase)
        || normalize(&entry.all_text).contains(phrase)
        || normalize(&entry.cwd).contains(phrase)
    {
        return Some(TIER_TEXT);
    }
    None
}

fn regex_score(entry: &SessionEntry, pattern: &regex::Regex) -> Option<f64> {
    if entry
        .name
        .as_deref()
        .is_some_and(|name| pattern.is_match(name))
    {
        return Some(TIER_NAME);
    }
    if pattern.is_match(&entry.id)
        || pattern.is_match(&entry.all_text)
        || pattern.is_match(&entry.cwd)
    {
        return Some(TIER_TEXT);
    }
    None
}

fn match_score(entry: &SessionEntry, query: &Query) -> Option<f64> {
    match query {
        Query::Regex(None) => None,
        Query::Regex(Some(pattern)) => regex_score(entry, pattern),
        Query::Tokens(tokens) => {
            let mut total = 0.0;
            for token in tokens {
                let score = match token {
                    Token::Fuzzy(value) => fuzzy_score(entry, value)?,
                    Token::Phrase(value) => {
                        let phrase = normalize(value);
                        if phrase.is_empty() {
                            continue;
                        }
                        phrase_score(entry, &phrase)?
                    }
                };
                total += score;
            }
            Some(total)
        }
    }
}

/// filterAndSortSessions: the entries a query keeps, in the order the sort mode puts them.
fn filter_and_sort(
    entries: &[SessionEntry],
    query: &str,
    sort: SortMode,
    named_only: bool,
) -> Vec<usize> {
    let candidates: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !named_only || entry.name.is_some())
        .map(|(index, _)| index)
        .collect();
    if query.trim().is_empty() {
        return candidates;
    }
    let parsed = parse_query(query);
    if let Query::Regex(None) = parsed {
        return Vec::new();
    }
    if sort == SortMode::Recent {
        return candidates
            .into_iter()
            .filter(|index| match_score(&entries[*index], &parsed).is_some())
            .collect();
    }
    let mut scored: Vec<(f64, usize)> = candidates
        .into_iter()
        .filter_map(|index| match_score(&entries[index], &parsed).map(|score| (score, index)))
        .collect();
    scored.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(entries[b.1].modified.cmp(&entries[a.1].modified))
    });
    scored.into_iter().map(|(_, index)| index).collect()
}

/// buildSessionTree then flattenSessionTree: parents above the sessions forked from them.
fn threaded_order(entries: &[SessionEntry]) -> Vec<Row> {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let parent = entry
            .parent
            .as_ref()
            .and_then(|parent| entries.iter().position(|other| &other.path == parent));
        match parent {
            Some(parent) if parent != index => children[parent].push(index),
            _ => roots.push(index),
        }
    }
    fn latest(
        modified: &[std::time::SystemTime],
        children: &[Vec<usize>],
        index: usize,
    ) -> std::time::SystemTime {
        children[index]
            .iter()
            .map(|child| latest(modified, children, *child))
            .fold(modified[index], std::time::SystemTime::max)
    }
    fn walk(
        children: &[Vec<usize>],
        index: usize,
        depth: usize,
        ancestor_continues: &[bool],
        is_last: bool,
        rows: &mut Vec<Row>,
    ) {
        rows.push(Row {
            index,
            depth,
            is_last,
            ancestor_continues: ancestor_continues.to_vec(),
        });
        let count = children[index].len();
        for (position, child) in children[index].iter().enumerate() {
            let child_is_last = position + 1 == count;
            let mut ancestors = ancestor_continues.to_vec();
            ancestors.push(if depth > 0 { !is_last } else { false });
            walk(children, *child, depth + 1, &ancestors, child_is_last, rows);
        }
    }
    let modified: Vec<std::time::SystemTime> = entries.iter().map(|entry| entry.modified).collect();
    let ranks: Vec<std::time::SystemTime> = (0..entries.len())
        .map(|index| latest(&modified, &children, index))
        .collect();
    let by_activity = |a: &usize, b: &usize| ranks[*b].cmp(&ranks[*a]);
    roots.sort_by(by_activity);
    for list in children.iter_mut() {
        list.sort_by(by_activity);
    }
    let mut rows = Vec::new();
    let root_count = roots.len();
    for (position, root) in roots.iter().enumerate() {
        walk(
            &children,
            *root,
            0,
            &[],
            position + 1 == root_count,
            &mut rows,
        );
    }
    rows
}

/// The picker: which folder scope it shows, how it sorts, and its search or rename input.
pub struct SessionSelector {
    /// The session directory of the folder pi-rs runs in.
    session_dir: String,
    agent_dir: String,
    /// The session the loop has open, which the picker refuses to delete.
    current: Option<String>,
    scope_all: bool,
    sort: SortMode,
    named_only: bool,
    show_path: bool,
    current_folder: Vec<SessionEntry>,
    everywhere: Vec<SessionEntry>,
    /// The entries the filter kept, in the order the sort mode puts them: rows index into this.
    visible: Vec<SessionEntry>,
    rows: Vec<Row>,
    selected: usize,
    search: Input,
    confirming_delete: Option<String>,
    status: Option<(String, bool)>,
    rename: Option<(String, Input)>,
    action: ActionSlot,
}

impl SessionSelector {
    pub fn new(
        session_dir: &str,
        agent_dir: &str,
        current: Option<&str>,
        action: ActionSlot,
    ) -> Self {
        let mut selector = SessionSelector {
            session_dir: session_dir.to_owned(),
            agent_dir: agent_dir.to_owned(),
            current: current.map(str::to_owned),
            scope_all: false,
            sort: SortMode::Threaded,
            named_only: false,
            show_path: false,
            current_folder: session_list::sessions(session_dir),
            everywhere: Vec::new(),
            visible: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            search: Input::new(InputOptions::default()),
            confirming_delete: None,
            status: None,
            rename: None,
            action,
        };
        selector.search.focused = true;
        selector.refilter();
        selector
    }

    fn entries(&self) -> &[SessionEntry] {
        if self.scope_all {
            &self.everywhere
        } else {
            &self.current_folder
        }
    }

    fn refilter(&mut self) {
        let query = self.search.value().to_owned();
        let kept = filter_and_sort(self.entries(), &query, self.sort, self.named_only);
        let entries = self.entries();
        self.visible = kept.iter().map(|index| entries[*index].clone()).collect();
        self.rows = if self.sort == SortMode::Threaded && query.trim().is_empty() {
            threaded_order(&self.visible)
        } else {
            (0..self.visible.len())
                .map(|index| Row {
                    index,
                    depth: 0,
                    is_last: true,
                    ancestor_continues: Vec::new(),
                })
                .collect()
        };
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn selected_entry(&self) -> Option<&SessionEntry> {
        let row = self.rows.get(self.selected)?;
        self.visible.get(row.index)
    }

    fn reload(&mut self) {
        self.current_folder = session_list::sessions(&self.session_dir);
        if self.scope_all {
            self.everywhere = session_list::all(&self.agent_dir);
        }
        self.refilter();
    }

    fn toggle_scope(&mut self) {
        self.scope_all = !self.scope_all;
        if self.scope_all && self.everywhere.is_empty() {
            self.everywhere = session_list::all(&self.agent_dir);
        }
        self.refilter();
    }

    fn confirm_delete(&mut self) {
        let Some(path) = self.confirming_delete.take() else {
            return;
        };
        match session_list::delete(&path) {
            Ok(true) => {
                self.status = Some(("Session moved to trash".to_owned(), false));
                self.reload();
            }
            Ok(false) => {
                self.status = Some(("Session deleted".to_owned(), false));
                self.reload();
            }
            Err(error) => self.status = Some((format!("Failed to delete: {error}"), true)),
        }
    }

    fn start_delete(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if self.current.as_deref() == Some(entry.path.as_str()) {
            self.status = Some((
                "Cannot delete the currently active session".to_owned(),
                true,
            ));
            return;
        }
        self.confirming_delete = Some(entry.path.clone());
    }

    fn tree_prefix(&self, row: &Row) -> String {
        if row.depth == 0 {
            return String::new();
        }
        let mut prefix = String::new();
        for continues in &row.ancestor_continues {
            prefix.push_str(if *continues { "\u{2502}  " } else { "   " });
        }
        prefix.push_str(if row.is_last {
            "\u{2514}\u{2500} "
        } else {
            "\u{251c}\u{2500} "
        });
        prefix
    }

    fn draw_row(&self, row: &Row, width: usize, selected: bool) -> String {
        let entry = &self.visible[row.index];
        let confirming = self.confirming_delete.as_deref() == Some(entry.path.as_str());
        let current = self.current.as_deref() == Some(entry.path.as_str());
        let prefix = self.tree_prefix(row);
        let display = entry
            .name
            .clone()
            .unwrap_or_else(|| entry.first_message.clone());
        let display: String = display
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect();
        let display = display.trim().to_owned();
        let age = format_age(entry.modified);
        let mut right = format!("{} {age}", entry.message_count);
        if self.scope_all && !entry.cwd.is_empty() {
            right = format!("{} {right}", shorten_home(&entry.cwd));
        }
        if self.show_path {
            right = format!("{} {right}", shorten_home(&entry.path));
        }
        let cursor = if selected {
            fg("accent", "\u{203a} ")
        } else {
            "  ".to_owned()
        };
        let reserved = visible_width(&right) + 2;
        let available = width
            .saturating_sub(2 + visible_width(&prefix) + reserved)
            .max(10);
        let message = truncate_to_width(&display, available, "\u{2026}", false);
        let mut styled = if confirming {
            fg("error", &message)
        } else if current {
            fg("accent", &message)
        } else if entry.name.is_some() {
            fg("warning", &message)
        } else {
            message
        };
        if selected {
            styled = bold(&styled);
        }
        let left = format!("{cursor}{}{styled}", fg("dim", &prefix));
        let spacing = width
            .saturating_sub(visible_width(&left) + visible_width(&right))
            .max(1);
        let right = fg(if confirming { "error" } else { "dim" }, &right);
        format!("{left}{}{right}", " ".repeat(spacing))
    }

    fn draw_header(&self, width: usize) -> Vec<String> {
        let title = if self.scope_all {
            "Resume Session (All)"
        } else {
            "Resume Session (Current Folder)"
        };
        let scope = if self.scope_all {
            format!(
                "{}{}",
                fg("muted", "\u{25cb} Current Folder | "),
                fg("accent", "\u{25c9} All")
            )
        } else {
            format!(
                "{}{}",
                fg("accent", "\u{25c9} Current Folder"),
                fg("muted", " | \u{25cb} All")
            )
        };
        let named = format!(
            "{}{}",
            fg("muted", "Name: "),
            fg("accent", if self.named_only { "Named" } else { "All" })
        );
        let sort = format!(
            "{}{}",
            fg("muted", "Sort: "),
            fg("accent", self.sort.label())
        );
        let right = truncate_to_width(&format!("{scope}  {named}  {sort}"), width, "", false);
        let available = width.saturating_sub(visible_width(&right) + 1);
        let left = truncate_to_width(&bold(title), available, "", false);
        let spacing = width.saturating_sub(visible_width(&left) + visible_width(&right));
        let mut lines = vec![format!("{left}{}{right}", " ".repeat(spacing))];
        if self.confirming_delete.is_some() {
            lines.push(fg(
                "error",
                &truncate_to_width(
                    &format!(
                        "Delete session? {} \u{b7} {}",
                        key_hint("tui.select.confirm", "confirm"),
                        key_hint("tui.select.cancel", "cancel")
                    ),
                    width,
                    "\u{2026}",
                    false,
                ),
            ));
            lines.push(String::new());
        } else if let Some((message, error)) = &self.status {
            lines.push(fg(
                if *error { "error" } else { "accent" },
                &truncate_to_width(message, width, "\u{2026}", false),
            ));
            lines.push(String::new());
        } else {
            let sep = fg("muted", " \u{b7} ");
            lines.push(
                [
                    key_hint("tui.input.tab", "scope"),
                    fg("muted", "re:<pattern> regex \u{b7} \"phrase\" exact"),
                ]
                .join(&sep),
            );
            let path_state = if self.show_path { "(on)" } else { "(off)" };
            lines.push(
                [
                    key_hint("app.session.toggleSort", "sort"),
                    key_hint("app.session.toggleNamedFilter", "named"),
                    key_hint("app.session.delete", "delete"),
                    key_hint("app.session.togglePath", &format!("path {path_state}")),
                    key_hint("app.session.rename", "rename"),
                ]
                .join(&sep),
            );
        }
        lines
    }

    fn rows_drawn(&mut self, width: usize) -> Vec<String> {
        let (start, end) = visible_range(self.selected, self.rows.len(), 10);
        let mut lines = Vec::new();
        for position in start..end {
            let line = self.draw_row(&self.rows[position], width, position == self.selected);
            lines.push(if position == self.selected {
                bg("selectedBg", &line)
            } else {
                line
            });
        }
        if start > 0 || end < self.rows.len() {
            lines.push(scroll_info(self.selected, self.rows.len()));
        }
        lines
    }
}

impl Component for SessionSelector {
    fn accepts_input(&self) -> bool {
        true
    }

    fn render(&mut self, width: usize) -> Vec<String> {
        if let Some((_, input)) = self.rename.as_mut() {
            let mut lines = vec![
                border(width),
                String::new(),
                format!(" {}", bold("Rename Session")),
                String::new(),
            ];
            lines.extend(input.render(width));
            lines.push(String::new());
            lines.push(fg(
                "muted",
                &format!(
                    "{} to save \u{b7} {} to cancel",
                    key_display("tui.select.confirm"),
                    key_display("tui.select.cancel")
                ),
            ));
            lines.push(String::new());
            lines.push(border(width));
            return fit(lines, width);
        }
        let mut lines = vec![border(width), String::new()];
        lines.extend(self.draw_header(width));
        lines.push(String::new());
        lines.extend(self.search.render(width));
        lines.push(String::new());
        if self.rows.is_empty() {
            let message = if self.named_only {
                let key = key_display("app.session.toggleNamedFilter");
                if self.scope_all {
                    format!("  No named sessions found. Press {key} to show all.")
                } else {
                    format!(
                        "  No named sessions in current folder. Press {key} to show all, or Tab to view all."
                    )
                }
            } else if self.scope_all {
                "  No sessions found".to_owned()
            } else {
                "  No sessions in current folder. Press Tab to view all.".to_owned()
            };
            lines.push(fg(
                "muted",
                &truncate_to_width(&message, width, "\u{2026}", false),
            ));
        } else {
            lines.extend(self.rows_drawn(width));
        }
        lines.push(String::new());
        lines.push(border(width));
        fit(lines, width)
    }

    fn handle_input(&mut self, data: &str) {
        let bindings = get_keybindings();
        if let Some((path, input)) = self.rename.as_mut() {
            if bindings.matches(data, "tui.select.cancel") {
                self.rename = None;
                return;
            }
            if bindings.matches(data, "tui.select.confirm") {
                let name = input.value().trim().to_owned();
                let path = path.clone();
                self.rename = None;
                if !name.is_empty() {
                    match session_list::rename(&path, &name) {
                        Ok(()) => {
                            self.status = Some((format!("Session name set: {name}"), false));
                            self.reload();
                        }
                        Err(error) => self.status = Some((format!("Error: {error}"), true)),
                    }
                }
                return;
            }
            input.handle_input(data);
            return;
        }
        if self.confirming_delete.is_some() {
            if bindings.matches(data, "tui.select.confirm") {
                self.confirm_delete();
            } else if bindings.matches(data, "tui.select.cancel") {
                self.confirming_delete = None;
            }
            return;
        }
        if bindings.matches(data, "tui.input.tab") {
            self.toggle_scope();
            return;
        }
        if bindings.matches(data, "app.session.toggleSort") {
            self.sort = self.sort.next();
            self.refilter();
            return;
        }
        if bindings.matches(data, "app.session.toggleNamedFilter") {
            self.named_only = !self.named_only;
            self.refilter();
            return;
        }
        if bindings.matches(data, "app.session.togglePath") {
            self.show_path = !self.show_path;
            return;
        }
        if bindings.matches(data, "app.session.delete") {
            self.start_delete();
            return;
        }
        if bindings.matches(data, "app.session.rename") {
            if let Some(entry) = self.selected_entry() {
                let (path, name) = (entry.path.clone(), entry.name.clone());
                let mut input = Input::new(InputOptions::default());
                input.set_value(name.as_deref().unwrap_or_default());
                input.focused = true;
                self.rename = Some((path, input));
            }
            return;
        }
        if bindings.matches(data, "app.session.deleteNoninvasive") {
            if self.search.value().is_empty() {
                self.start_delete();
            } else {
                self.search.handle_input(data);
                self.refilter();
            }
            return;
        }
        if bindings.matches(data, "tui.select.up") {
            self.selected = self.selected.saturating_sub(1);
        } else if bindings.matches(data, "tui.select.down") {
            self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
        } else if bindings.matches(data, "tui.select.pageUp") {
            self.selected = self.selected.saturating_sub(10);
        } else if bindings.matches(data, "tui.select.pageDown") {
            self.selected = (self.selected + 10).min(self.rows.len().saturating_sub(1));
        } else if bindings.matches(data, "tui.select.confirm") {
            if let Some(entry) = self.selected_entry() {
                let path = entry.path.clone();
                push(&self.action, OverlayAction::Resume { path });
            }
        } else if bindings.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Close);
        } else {
            self.search.handle_input(data);
            self.refilter();
        }
    }
}

/// Show the picker on its own, before the loop starts (--resume): the chosen session's path, or
/// None when the user cancels with Escape (upstream cli/session-picker.ts).
pub fn select_session(session_dir: &str, agent_dir: &str) -> Result<Option<String>, String> {
    use pi_tui::terminal::ProcessTerminal;
    use pi_tui::tui::component;
    use pi_tui::tui_main_screen::{TuiMainScreen, as_ref};

    super::super::keybindings::install(agent_dir);
    let slot: ActionSlot = super::action_slot();
    let selector = component(SessionSelector::new(
        session_dir,
        agent_dir,
        None,
        slot.clone(),
    ));
    let mut tui = TuiMainScreen::new(ProcessTerminal::new());
    let reference = as_ref(&selector);
    tui.add_child(reference.clone());
    tui.set_focus(Some(reference));
    tui.start();
    let mut chosen = None;
    loop {
        let timeout = tui.tick(0).map_or(50, |due| due.min(50));
        let events = tui.terminal.poll(Some(timeout));
        tui.handle_events(events);
        if let Some(action) = slot.borrow_mut().take() {
            match action {
                OverlayAction::Resume { path } => {
                    chosen = Some(path);
                    break;
                }
                OverlayAction::Close => break,
                _ => {}
            }
        }
        tui.request_render(false);
    }
    tui.stop(false);
    tui.terminal.drain_input(500, 50);
    Ok(chosen)
}
