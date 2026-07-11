//! Multiline editor mechanism, ported from pi's TUI editor.
//!
//! The editor owns text editing, cursor/layout state, history, paste storage,
//! undo, kill/yank, and provider-neutral autocomplete picker mechanics.
//! Integrations remain responsible for provider policy and submissions.
use crate::{
    autocomplete::{self, AutocompleteItem, Suggestions},
    input::CURSOR_MARKER,
    kill_ring::KillRing,
    select_list::{SelectItem, SelectList, SelectListLayout, SelectListTheme},
    undo_stack::UndoStack,
    utils::{truncate_to_width, visible_width},
};
use std::collections::BTreeMap;
use unicode_segmentation::UnicodeSegmentation;

const LARGE_PASTE_LINES: usize = 10;
const LARGE_PASTE_CHARS: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    value: String,
    cursor: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    /// UTF-8 byte offset within the logical line.
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChunk {
    pub text: String,
    pub start_index: usize,
    pub end_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEffect {
    Changed(String),
    Submit(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutocompleteRequest {
    pub id: u64,
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub force: bool,
    pub explicit_tab: bool,
    /// Spec: `getAutocompleteDebounceMs` — 20ms for attachment (`@`/trigger
    /// token) contexts, otherwise 0. The integration owning the request
    /// pump honors the delay (pi debounces inside the editor).
    pub debounce_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutocompleteMode {
    Regular,
    Force,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Action {
    #[default]
    None,
    Kill,
    Yank,
    TypeWord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpDirection {
    Forward,
    Backward,
}

#[derive(Debug, Default)]
pub struct Editor {
    state: State,
    undo: UndoStack<State>,
    ring: KillRing,
    last: Action,
    pastes: BTreeMap<usize, String>,
    paste_counter: usize,
    paste_buffer: String,
    in_paste: bool,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<State>,
    last_width: usize,
    preferred_visual_col: Option<usize>,
    /// Intended absolute position before vertical movement snapped into an atomic marker.
    snapped_from_cursor: Option<usize>,
    jump_mode: Option<JumpDirection>,
    scroll_offset: usize,
    yank_range: Option<(usize, usize)>,
    padding_x: usize,
    border_open: String,
    border_close: String,
    autocomplete_max_visible: usize,
    terminal_rows: usize,
    focused: bool,
    disable_submit: bool,
    autocomplete_enabled: bool,
    autocomplete_triggers: Vec<char>,
    autocomplete_mode: Option<AutocompleteMode>,
    autocomplete_prefix: String,
    autocomplete_list: Option<SelectList>,
    autocomplete_request_id: u64,
    pending_autocomplete_request: Option<AutocompleteRequest>,
    outstanding_autocomplete_request: Option<AutocompleteRequest>,
    autocomplete_explicit_tab: bool,
    select_list_theme: SelectListTheme,
}

impl Editor {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self {
            state: State { value, cursor },
            last_width: 80,
            autocomplete_max_visible: 5,
            terminal_rows: 24,
            ..Default::default()
        }
    }

    pub fn value(&self) -> &str {
        &self.state.value
    }

    pub fn text(&self) -> &str {
        self.value()
    }

    /// Compatibility setter: replace text while preserving and clamping the
    /// old absolute cursor. Use [`Self::set_text`] for pi editor semantics.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.cancel_autocomplete();
        self.state.value = value.into();
        self.clamp_cursor();
        self.reset_vertical();
    }

    /// Normalize and replace the document, placing the cursor at its end.
    pub fn set_text(&mut self, text: impl AsRef<str>) {
        self.cancel_autocomplete();
        let normalized = normalize_text(text.as_ref());
        self.exit_history();
        self.last = Action::None;
        if self.state.value != normalized {
            self.save();
        }
        self.state.cursor = normalized.len();
        self.state.value = normalized;
        self.reset_vertical();
        self.scroll_offset = 0;
    }

    pub fn cursor(&self) -> usize {
        self.state.cursor
    }

    pub fn logical_cursor(&self) -> Cursor {
        let before = &self.state.value[..self.state.cursor];
        let line = before.bytes().filter(|byte| *byte == b'\n').count();
        let col = before
            .rfind('\n')
            .map_or(before.len(), |at| before.len() - at - 1);
        Cursor { line, col }
    }

    pub fn lines(&self) -> Vec<String> {
        self.state.value.split('\n').map(str::to_owned).collect()
    }

    pub fn expanded_text(&self) -> String {
        let mut result = self.state.value.clone();
        for (id, content) in &self.pastes {
            result = replace_paste_id(&result, *id, content);
        }
        result
    }

    pub fn padding_x(&self) -> usize {
        self.padding_x
    }

    pub fn set_padding_x(&mut self, padding: usize) {
        self.padding_x = padding;
    }

    /// Inject the border style, matching pi's `EditorTheme.borderColor`
    /// function (the coding agent passes `theme.fg("borderMuted", …)`).
    pub fn set_border_style(&mut self, open: impl Into<String>, close: impl Into<String>) {
        self.border_open = open.into();
        self.border_close = close.into();
    }

    /// Inject the autocomplete select-list styling, matching pi's
    /// `EditorTheme.selectList` (`getSelectListTheme` in the coding agent).
    pub fn set_select_list_theme(&mut self, theme: SelectListTheme) {
        self.select_list_theme = theme;
    }

    pub fn autocomplete_max_visible(&self) -> usize {
        self.autocomplete_max_visible
    }

    pub fn set_autocomplete_max_visible(&mut self, max_visible: usize) {
        self.autocomplete_max_visible = max_visible.clamp(3, 20);
    }

    pub fn set_terminal_rows(&mut self, rows: usize) {
        self.terminal_rows = rows;
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn set_disable_submit(&mut self, disabled: bool) {
        self.disable_submit = disabled;
    }

    pub fn disable_submit(&self) -> bool {
        self.disable_submit
    }

    pub fn set_autocomplete_triggers(&mut self, triggers: &[String]) {
        self.cancel_autocomplete();
        self.autocomplete_enabled = true;
        self.autocomplete_triggers = vec!['@', '#'];
        for trigger in triggers {
            let mut chars = trigger.chars();
            if let (Some(ch), None) = (chars.next(), chars.next())
                && ch != '/'
                && !ch.is_whitespace()
                && !self.autocomplete_triggers.contains(&ch)
            {
                self.autocomplete_triggers.push(ch);
            }
        }
    }

    pub fn take_autocomplete_request(&mut self) -> Option<AutocompleteRequest> {
        self.pending_autocomplete_request.take()
    }

    pub fn autocomplete_showing(&self) -> bool {
        self.autocomplete_mode.is_some() && self.autocomplete_list.is_some()
    }

    pub fn autocomplete_selected(&self) -> Option<AutocompleteItem> {
        self.autocomplete_list
            .as_ref()?
            .selected()
            .map(|item| AutocompleteItem {
                value: item.value.clone(),
                label: item.label.clone(),
                description: item.description.clone(),
            })
    }

    pub fn apply_autocomplete_suggestions(
        &mut self,
        id: u64,
        suggestions: Option<Suggestions>,
    ) -> (bool, bool) {
        let Some(request) = self.outstanding_autocomplete_request.take() else {
            return (false, false);
        };
        let cursor = self.logical_cursor();
        if request.id != id
            || request.lines != self.lines()
            || request.cursor_line != cursor.line
            || request.cursor_col != cursor.col
        {
            self.outstanding_autocomplete_request = Some(request);
            return (false, false);
        }
        let Some(suggestions) = suggestions.filter(|value| !value.items.is_empty()) else {
            self.clear_autocomplete_ui();
            return (true, false);
        };
        let single_explicit = request.force && request.explicit_tab && suggestions.items.len() == 1;
        let best = best_autocomplete_match(&suggestions.items, &suggestions.prefix);
        let items = suggestions
            .items
            .into_iter()
            .map(|item| SelectItem {
                value: item.value,
                label: item.label,
                description: item.description,
            })
            .collect();
        // Spec: `createAutocompleteList` — the slash-command menu clamps the
        // primary column into [12, 32]; everything else uses the default.
        let layout = if suggestions.prefix.starts_with('/') {
            SelectListLayout {
                min_primary_column_width: Some(12),
                max_primary_column_width: Some(32),
            }
        } else {
            SelectListLayout::default()
        };
        let mut list = SelectList::with_theme(
            items,
            self.autocomplete_max_visible,
            self.select_list_theme.clone(),
            layout,
        );
        if let Some(index) = best {
            list.set_selected_index(index);
        }
        self.autocomplete_mode = Some(if request.force {
            AutocompleteMode::Force
        } else {
            AutocompleteMode::Regular
        });
        self.autocomplete_prefix = suggestions.prefix;
        self.autocomplete_list = Some(list);
        let changed = single_explicit && self.apply_selected_autocomplete();
        (true, changed)
    }

    fn clear_autocomplete_ui(&mut self) {
        self.autocomplete_mode = None;
        self.autocomplete_prefix.clear();
        self.autocomplete_list = None;
    }

    pub fn cancel_autocomplete(&mut self) {
        self.autocomplete_request_id = self.autocomplete_request_id.wrapping_add(1);
        self.pending_autocomplete_request = None;
        self.outstanding_autocomplete_request = None;
        self.clear_autocomplete_ui();
    }

    fn request_autocomplete(&mut self, force: bool, explicit_tab: bool) {
        if !self.autocomplete_enabled {
            return;
        }
        self.autocomplete_request_id = self.autocomplete_request_id.wrapping_add(1);
        let cursor = self.logical_cursor();
        let mode = if force {
            AutocompleteMode::Force
        } else {
            AutocompleteMode::Regular
        };
        let debounce_ms = if explicit_tab || force {
            0
        } else if self.debounce_pattern_matches(self.current_line_before_cursor()) {
            20
        } else {
            0
        };
        let request = AutocompleteRequest {
            id: self.autocomplete_request_id,
            lines: self.lines(),
            cursor_line: cursor.line,
            cursor_col: cursor.col,
            force,
            explicit_tab,
            debounce_ms,
        };
        self.autocomplete_explicit_tab = explicit_tab;
        self.outstanding_autocomplete_request = Some(request.clone());
        self.pending_autocomplete_request = Some(request);
        if self.autocomplete_showing() {
            self.autocomplete_mode = Some(mode);
        }
    }

    fn refresh_autocomplete(&mut self) {
        if let Some(mode) = self.autocomplete_mode {
            self.request_autocomplete(mode == AutocompleteMode::Force, false);
        }
    }

    fn current_line_before_cursor(&self) -> &str {
        let start = self.state.value[..self.state.cursor]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        &self.state.value[start..self.state.cursor]
    }

    /// Spec: `isInSlashCommandContext` (with `isSlashMenuAllowed`).
    fn slash_context(&self) -> bool {
        self.logical_cursor().line == 0
            && self
                .current_line_before_cursor()
                .trim_start()
                .starts_with('/')
    }

    /// Spec: `isAtStartOfMessage` — slash-command auto-trigger gate.
    fn is_at_start_of_message(&self) -> bool {
        self.logical_cursor().line == 0
            && matches!(self.current_line_before_cursor().trim(), "" | "/")
    }

    /// Spec: `buildTriggerPattern` — `(?:^|[\s])[triggers][^\s]*$`.
    fn trigger_pattern_matches(&self, before: &str) -> bool {
        for (at, ch) in before.char_indices() {
            if !self.autocomplete_triggers.contains(&ch) {
                continue;
            }
            let boundary = at == 0
                || before[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            if boundary
                && !before[at + ch.len_utf8()..]
                    .chars()
                    .any(char::is_whitespace)
            {
                return true;
            }
        }
        false
    }

    /// Spec: `buildDebouncePattern` —
    /// `(?:^|[ \t])(?:@(?:"[^"]*|[^\s]*)|[non-@ triggers][^\s]*)$`.
    fn debounce_pattern_matches(&self, before: &str) -> bool {
        for (at, ch) in before.char_indices() {
            let boundary =
                at == 0 || matches!(before[..at].chars().next_back(), Some(' ') | Some('\t'));
            if !boundary {
                continue;
            }
            let rest = &before[at + ch.len_utf8()..];
            if ch == '@' {
                if let Some(quoted) = rest.strip_prefix('"') {
                    if !quoted.contains('"') {
                        return true;
                    }
                } else if !rest.chars().any(char::is_whitespace) {
                    return true;
                }
            } else if ch != '@'
                && self.autocomplete_triggers.contains(&ch)
                && !rest.chars().any(char::is_whitespace)
            {
                return true;
            }
        }
        false
    }

    /// Spec: the trigger tail of `insertCharacter`.
    fn after_insert_autocomplete(&mut self, text: &str) {
        if !self.autocomplete_enabled {
            return;
        }
        if self.autocomplete_showing() {
            self.refresh_autocomplete();
            return;
        }
        if text == "/" && self.is_at_start_of_message() {
            self.request_autocomplete(false, false);
            return;
        }
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        };
        if let Some(ch) = single
            && self.autocomplete_triggers.contains(&ch)
        {
            // Token boundary: the symbol is the line's first character or
            // follows a space/tab.
            let before = self.current_line_before_cursor();
            let before_symbol = &before[..before.len().saturating_sub(ch.len_utf8())];
            if before_symbol.is_empty()
                || matches!(before_symbol.chars().next_back(), Some(' ') | Some('\t'))
            {
                self.request_autocomplete(false, false);
            }
            return;
        }
        // Spec: `/[a-zA-Z0-9.\-_]/.test(char)` — any matching char counts.
        if text
            .chars()
            .any(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        {
            let before = self.current_line_before_cursor().to_owned();
            if self.slash_context() || self.trigger_pattern_matches(&before) {
                self.request_autocomplete(false, false);
            }
        }
    }

    /// Spec: the shared tail of `handleBackspace`/`handleForwardDelete` —
    /// update an open picker or re-trigger a completable context.
    fn after_delete_autocomplete(&mut self) {
        if !self.autocomplete_enabled {
            return;
        }
        if self.autocomplete_showing() {
            self.refresh_autocomplete();
            return;
        }
        let before = self.current_line_before_cursor().to_owned();
        if self.slash_context() || self.trigger_pattern_matches(&before) {
            self.request_autocomplete(false, false);
        }
    }

    fn apply_selected_autocomplete(&mut self) -> bool {
        let Some(item) = self.autocomplete_selected() else {
            return false;
        };
        let cursor = self.logical_cursor();
        let lines = self.lines();
        let applied = autocomplete::apply_completion(
            &lines,
            cursor.line,
            cursor.col,
            &item,
            &self.autocomplete_prefix,
        );
        let value = applied.lines.join("\n");
        let next_cursor = applied
            .lines
            .iter()
            .take(applied.cursor_line)
            .map(|line| line.len() + 1)
            .sum::<usize>()
            + applied.cursor_col.min(
                applied
                    .lines
                    .get(applied.cursor_line)
                    .map_or(0, String::len),
            );
        let changed = self.state.value != value || self.state.cursor != next_cursor;
        if changed {
            self.save();
            self.state.value = value;
            self.state.cursor = next_cursor;
            self.last = Action::None;
            self.reset_vertical();
        }
        self.cancel_autocomplete();
        changed
    }

    fn save(&mut self) {
        self.undo.push(&self.state);
    }

    fn clamp_cursor(&mut self) {
        self.state.cursor = self.state.cursor.min(self.state.value.len());
        while !self.state.value.is_char_boundary(self.state.cursor) {
            self.state.cursor = self.state.cursor.saturating_sub(1);
        }
    }

    fn reset_vertical(&mut self) {
        self.preferred_visual_col = None;
        self.snapped_from_cursor = None;
    }

    fn current_visual_line(map: &[(usize, usize)], cursor: usize) -> usize {
        map.iter()
            .enumerate()
            .find(|(index, (start, end))| {
                let is_last_segment = index + 1 == map.len() || map[index + 1].0 > *end;
                cursor >= *start && (cursor < *end || (is_last_segment && cursor == *end))
            })
            .map_or_else(|| map.len().saturating_sub(1), |(index, _)| index)
    }

    fn marker_spans(&self, text: &str) -> Vec<(usize, usize)> {
        marker_spans(text)
            .into_iter()
            .filter(|(_, _, id)| self.pastes.contains_key(id))
            .map(|(start, end, _)| (start, end))
            .collect()
    }

    fn previous_unit(&self, at: usize) -> usize {
        if at == 0 {
            return 0;
        }
        for (start, end) in self.marker_spans(&self.state.value) {
            if at > start && at <= end {
                return start;
            }
        }
        self.state.value[..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_unit(&self, at: usize) -> usize {
        if at >= self.state.value.len() {
            return self.state.value.len();
        }
        for (start, end) in self.marker_spans(&self.state.value) {
            if at >= start && at < end {
                return end;
            }
        }
        self.state.value[at..]
            .graphemes(true)
            .next()
            .map_or(self.state.value.len(), |g| at + g.len())
    }

    fn left(&mut self) {
        self.state.cursor = self.previous_unit(self.state.cursor);
        self.last = Action::None;
        self.reset_vertical();
    }

    fn right(&mut self) {
        self.state.cursor = self.next_unit(self.state.cursor);
        self.last = Action::None;
        self.reset_vertical();
    }

    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.exit_history();
        if text.chars().all(char::is_whitespace) || self.last != Action::TypeWord {
            self.save();
        }
        self.insert_raw(text);
        self.last = Action::TypeWord;
        self.reset_vertical();
        self.after_insert_autocomplete(text);
    }

    /// Insert normalized single- or multiline text as one undo operation.
    pub fn insert_text_at_cursor(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.exit_history();
        self.save();
        self.last = Action::None;
        self.insert_raw(&normalize_text(text));
        self.reset_vertical();
    }

    pub fn add_newline(&mut self) {
        // Pi's addNewLine cancels autocomplete before splitting the line.
        self.cancel_autocomplete();
        self.insert_text_at_cursor("\n");
    }

    pub fn backspace(&mut self) {
        if self.state.cursor == 0 {
            self.last = Action::None;
            self.after_delete_autocomplete();
            return;
        }
        self.exit_history();
        self.save();
        let from = self.previous_unit(self.state.cursor);
        self.state.value.drain(from..self.state.cursor);
        self.state.cursor = from;
        self.last = Action::None;
        self.reset_vertical();
        self.after_delete_autocomplete();
    }

    pub fn delete(&mut self) {
        if self.state.cursor >= self.state.value.len() {
            self.last = Action::None;
            self.after_delete_autocomplete();
            return;
        }
        self.exit_history();
        self.save();
        let to = self.next_unit(self.state.cursor);
        self.state.value.drain(self.state.cursor..to);
        self.last = Action::None;
        self.reset_vertical();
        self.after_delete_autocomplete();
    }

    pub fn undo(&mut self) {
        if let Some(state) = self.undo.pop() {
            self.state = state;
            self.last = Action::None;
            self.yank_range = None;
            self.reset_vertical();
        }
    }

    fn line_bounds(&self) -> (usize, usize) {
        let start = self.state.value[..self.state.cursor]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        let end = self.state.value[self.state.cursor..]
            .find('\n')
            .map_or(self.state.value.len(), |at| self.state.cursor + at);
        (start, end)
    }

    pub fn move_to_line_start(&mut self) {
        self.state.cursor = self.line_bounds().0;
        self.last = Action::None;
        self.reset_vertical();
    }

    pub fn move_to_line_end(&mut self) {
        self.state.cursor = self.line_bounds().1;
        self.last = Action::None;
        self.reset_vertical();
    }

    pub fn kill_to_start(&mut self) {
        if self.state.cursor == 0 {
            return;
        }
        let (line_start, _) = self.line_bounds();
        let from = if self.state.cursor == line_start {
            line_start.saturating_sub(1)
        } else {
            line_start
        };
        self.kill_range(from, self.state.cursor, true);
    }

    pub fn kill_to_end(&mut self) {
        if self.state.cursor == self.state.value.len() {
            return;
        }
        let (_, line_end) = self.line_bounds();
        let to = if self.state.cursor == line_end {
            (line_end + 1).min(self.state.value.len())
        } else {
            line_end
        };
        self.kill_range(self.state.cursor, to, false);
    }

    fn kill_range(&mut self, from: usize, to: usize, prepend: bool) {
        if from >= to {
            return;
        }
        self.exit_history();
        let accumulate = self.last == Action::Kill;
        self.save();
        let deleted = self.state.value[from..to].to_owned();
        self.ring.push(&deleted, prepend, accumulate);
        self.state.value.drain(from..to);
        self.state.cursor = from;
        self.last = Action::Kill;
        self.yank_range = None;
        self.reset_vertical();
    }

    pub fn yank(&mut self) {
        if let Some(text) = self.ring.peek().map(str::to_owned) {
            self.save();
            let start = self.state.cursor;
            self.insert_raw(&text);
            self.yank_range = Some((start, self.state.cursor));
            self.last = Action::Yank;
        }
    }

    pub fn yank_pop(&mut self) {
        if self.last != Action::Yank || self.ring.len() < 2 {
            return;
        }
        let Some((start, end)) = self.yank_range else {
            return;
        };
        if end > self.state.value.len() || start > end {
            return;
        }
        self.save();
        self.state.value.drain(start..end);
        self.state.cursor = start;
        self.ring.rotate();
        if let Some(text) = self.ring.peek().map(str::to_owned) {
            self.insert_raw(&text);
            self.yank_range = Some((start, self.state.cursor));
        }
        self.last = Action::Yank;
    }

    fn insert_raw(&mut self, text: &str) {
        self.state.value.insert_str(self.state.cursor, text);
        self.state.cursor += text.len();
    }

    pub fn delete_word_backward(&mut self) {
        if self.state.cursor == 0 {
            return;
        }
        let accumulate = self.last == Action::Kill;
        let old = self.state.cursor;
        self.move_word_left();
        let from = self.state.cursor;
        self.state.cursor = old;
        self.last = if accumulate {
            Action::Kill
        } else {
            Action::None
        };
        self.kill_range(from, old, true);
    }

    pub fn delete_word_forward(&mut self) {
        if self.state.cursor >= self.state.value.len() {
            return;
        }
        let accumulate = self.last == Action::Kill;
        let old = self.state.cursor;
        self.move_word_right();
        let to = self.state.cursor;
        self.state.cursor = old;
        self.last = if accumulate {
            Action::Kill
        } else {
            Action::None
        };
        self.kill_range(old, to, false);
    }

    pub fn move_word_left(&mut self) {
        let mut at = self.state.cursor;
        while at > 0 {
            let previous = self.previous_unit(at);
            if !self.state.value[previous..at]
                .chars()
                .all(char::is_whitespace)
            {
                break;
            }
            at = previous;
        }
        if at > 0 {
            let previous = self.previous_unit(at);
            let word = is_word(&self.state.value[previous..at]);
            at = previous;
            while at > 0 {
                let previous = self.previous_unit(at);
                if is_word(&self.state.value[previous..at]) != word
                    || self.state.value[previous..at]
                        .chars()
                        .all(char::is_whitespace)
                {
                    break;
                }
                at = previous;
            }
        }
        self.state.cursor = at;
        self.last = Action::None;
        self.reset_vertical();
    }

    pub fn move_word_right(&mut self) {
        let len = self.state.value.len();
        let mut at = self.state.cursor;
        while at < len {
            let next = self.next_unit(at);
            if !self.state.value[at..next].chars().all(char::is_whitespace) {
                break;
            }
            at = next;
        }
        if at < len {
            let next = self.next_unit(at);
            let word = is_word(&self.state.value[at..next]);
            at = next;
            while at < len {
                let next = self.next_unit(at);
                if is_word(&self.state.value[at..next]) != word
                    || self.state.value[at..next].chars().all(char::is_whitespace)
                {
                    break;
                }
                at = next;
            }
        }
        self.state.cursor = at;
        self.last = Action::None;
        self.reset_vertical();
    }

    pub fn add_to_history(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.history.first().is_some_and(|item| item == text) {
            return;
        }
        self.history.insert(0, text.to_owned());
        self.history.truncate(100);
    }

    fn navigate_history(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.history_index, up) {
            (None, true) => Some(0),
            (Some(index), true) if index + 1 < self.history.len() => Some(index + 1),
            (Some(0), false) => None,
            (Some(index), false) => Some(index - 1),
            _ => return,
        };
        if self.history_index.is_none() && next.is_some() {
            self.save();
            self.history_draft = Some(self.state.clone());
        }
        self.history_index = next;
        self.state = if let Some(index) = next {
            let value = self.history.get(index).cloned().unwrap_or_default();
            let cursor = if up { 0 } else { value.len() };
            State { value, cursor }
        } else {
            self.history_draft.take().unwrap_or_default()
        };
        self.last = Action::None;
        self.reset_vertical();
        // Pi resets the scroll before render re-adjusts it to the cursor.
        self.scroll_offset = 0;
    }

    fn exit_history(&mut self) {
        self.history_index = None;
        self.history_draft = None;
    }

    /// Process a paste directly, applying pi's normalization and large-paste
    /// marker behavior.
    pub fn paste(&mut self, text: &str) {
        let decoded = decode_paste_controls(text);
        let normalized = normalize_text(&decoded);
        let mut filtered: String = normalized
            .chars()
            .filter(|ch| *ch == '\n' || (*ch as u32) >= 32)
            .collect();
        if matches!(filtered.chars().next(), Some('/' | '~' | '.'))
            && self.state.value[..self.state.cursor]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            filtered.insert(0, ' ');
        }
        if filtered.is_empty() {
            return;
        }
        let lines = filtered.split('\n').count();
        // Pi counts UTF-16 code units (JS `String.length`) and reports the
        // total split-line count in the marker.
        let total_chars = filtered.encode_utf16().count();
        let marker = if lines > LARGE_PASTE_LINES || total_chars > LARGE_PASTE_CHARS {
            self.paste_counter += 1;
            let id = self.paste_counter;
            self.pastes.insert(id, filtered.clone());
            if lines > LARGE_PASTE_LINES {
                format!("[paste #{id} +{lines} lines]")
            } else {
                format!("[paste #{id} {total_chars} chars]")
            }
        } else {
            filtered
        };
        self.insert_text_at_cursor(&marker);
        self.cancel_autocomplete();
    }

    /// Render with the editor's configured padding, rows, and focus state
    /// (pi's `Editor.render(width)`).
    pub fn render_configured(&mut self, width: usize) -> Vec<String> {
        self.render_full(width)
    }

    /// Pi's `submitValue`: always clears the document and fires the submit
    /// effect, even when the trimmed value is empty. History is app policy
    /// (interactive-mode.ts calls `addToHistory` after routing), not part of
    /// the mechanism.
    pub fn submit(&mut self) -> Option<String> {
        self.cancel_autocomplete();
        let value = self.expanded_text().trim().to_owned();
        self.state = State::default();
        self.exit_history();
        self.undo.clear();
        self.pastes.clear();
        self.paste_counter = 0;
        self.last = Action::None;
        self.scroll_offset = 0;
        Some(value)
    }

    /// Handle an input sequence and report the mechanism effect.
    pub fn handle_effect(&mut self, data: &str) -> Option<EditorEffect> {
        let key = decode_key(data);
        if let Some(direction) = self.jump_mode {
            if matches!(key.as_deref(), Some("ctrl+]") | Some("ctrl+alt+]")) {
                self.jump_mode = None;
                return None;
            }
            if let Some(printable) = decode_printable(data).or_else(|| {
                (!data.is_empty() && !data.chars().any(char::is_control)).then(|| data.to_owned())
            }) {
                self.jump_mode = None;
                self.jump_to_char(&printable, direction);
                return None;
            }
            self.jump_mode = None;
        }
        if data.contains("\x1b[200~") {
            self.in_paste = true;
            self.paste_buffer.clear();
            let remaining = data.replacen("\x1b[200~", "", 1);
            return self.handle_paste_buffer(&remaining);
        }
        if self.in_paste {
            return self.handle_paste_buffer(data);
        }
        // Spec order: `tui.input.copy` (ctrl+c) returns to the parent before
        // the autocomplete block — it does not cancel an open picker.
        if matches!(key.as_deref(), Some("ctrl+c")) {
            return None;
        }
        if self.autocomplete_showing() {
            match key.as_deref() {
                Some("escape") => {
                    self.cancel_autocomplete();
                    return None;
                }
                Some("up") | Some("down") => {
                    if let Some(list) = &mut self.autocomplete_list {
                        list.handle(data);
                    }
                    return None;
                }
                Some("tab") => {
                    let before = self.value().to_owned();
                    self.apply_selected_autocomplete();
                    return (before != self.value())
                        .then(|| EditorEffect::Changed(self.value().to_owned()));
                }
                Some("enter") if self.autocomplete_prefix.starts_with('/') => {
                    self.apply_selected_autocomplete();
                }
                Some("enter") => {
                    let before = self.value().to_owned();
                    self.apply_selected_autocomplete();
                    return (before != self.value())
                        .then(|| EditorEffect::Changed(self.value().to_owned()));
                }
                _ => {}
            }
        }
        if matches!(key.as_deref(), Some("tab")) && !self.autocomplete_showing() {
            let slash_command = self.slash_context()
                && !self.current_line_before_cursor().trim_start().contains(' ');
            self.request_autocomplete(!slash_command, true);
            return None;
        }
        // Pi's newline condition: the newLine binding (shift+enter, which
        // "\n" and "\x1b\r" decode to under the active kitty protocol), plus
        // the raw multi-byte and modifyOtherKeys forms it accepts verbatim.
        let is_newline = matches!(key.as_deref(), Some("shift+enter"))
            || (data.len() > 1 && data.as_bytes().first() == Some(&b'\n'))
            || data == "\x1b\r"
            || data == "\x1b[13;2~"
            || (data.len() > 1 && data.contains('\x1b') && data.contains('\r'))
            || data == "\n";
        if is_newline {
            let before = self.value().to_owned();
            // shouldSubmitOnBackslashEnter only applies when the submit
            // binding includes shift+enter (non-default configuration).
            self.add_newline();
            return (before != self.value())
                .then(|| EditorEffect::Changed(self.value().to_owned()));
        }
        let before = self.value().to_owned();
        match key.as_deref() {
            // Spec: `moveCursor` re-queries an open picker after the cursor
            // moves (movement changes the text before the cursor).
            Some("left") | Some("ctrl+b") => {
                self.left();
                if self.autocomplete_showing() {
                    self.refresh_autocomplete();
                }
            }
            Some("right") | Some("ctrl+f") => {
                self.right_with_eof_preference();
                if self.autocomplete_showing() {
                    self.refresh_autocomplete();
                }
            }
            Some("up") => self.move_vertical(-1),
            Some("down") => self.move_vertical(1),
            Some("pageUp") => self.page_scroll(-1),
            Some("pageDown") => self.page_scroll(1),
            Some("home") | Some("ctrl+a") => self.move_to_line_start(),
            Some("end") | Some("ctrl+e") => self.move_to_line_end(),
            Some("backspace") | Some("shift+backspace") => self.backspace(),
            Some("delete") | Some("ctrl+d") | Some("shift+delete") => self.delete(),
            Some("ctrl+w") | Some("alt+backspace") => self.delete_word_backward(),
            Some("alt+d") | Some("alt+delete") => self.delete_word_forward(),
            Some("ctrl+u") => self.kill_to_start(),
            Some("ctrl+k") => self.kill_to_end(),
            Some("ctrl+y") => self.yank(),
            Some("alt+y") => self.yank_pop(),
            Some("ctrl+-") => self.undo(),
            Some("alt+left") | Some("ctrl+left") | Some("alt+b") => self.move_word_left(),
            Some("alt+right") | Some("ctrl+right") | Some("alt+f") => self.move_word_right(),
            Some("ctrl+]") => self.jump_mode = Some(JumpDirection::Forward),
            Some("ctrl+alt+]") => self.jump_mode = Some(JumpDirection::Backward),
            Some("enter") if self.disable_submit => {}
            Some("enter") => {
                // Workaround for terminals without shift+enter support: a
                // backslash before the cursor turns enter into a newline.
                if self.current_line_before_cursor().ends_with('\\') {
                    self.backspace();
                    self.add_newline();
                } else {
                    return self.submit().map(EditorEffect::Submit);
                }
            }
            Some("shift+space") => self.insert(" "),
            Some(key) if key.starts_with("ctrl+") => {}
            _ => {
                if let Some(text) = decode_printable(data) {
                    self.insert(&text);
                } else if !data.chars().any(char::is_control) {
                    self.insert(data);
                } else {
                    return None;
                }
            }
        }
        (before != self.value()).then(|| EditorEffect::Changed(self.value().to_owned()))
    }

    fn handle_paste_buffer(&mut self, data: &str) -> Option<EditorEffect> {
        self.paste_buffer.push_str(data);
        let end = self.paste_buffer.find("\x1b[201~")?;
        let content = self.paste_buffer[..end].to_owned();
        let remaining = self.paste_buffer[end + 6..].to_owned();
        self.paste_buffer.clear();
        self.in_paste = false;
        let before = self.value().to_owned();
        self.paste(&content);
        let first =
            (before != self.value()).then(|| EditorEffect::Changed(self.value().to_owned()));
        if !remaining.is_empty() {
            self.handle_effect(&remaining).or(first)
        } else {
            first
        }
    }

    /// Backwards-compatible input API. Submit returns its expanded submitted
    /// value; edits and movements return the current value.
    pub fn handle(&mut self, data: &str) -> Option<String> {
        match self.handle_effect(data) {
            Some(EditorEffect::Changed(value) | EditorEffect::Submit(value)) => Some(value),
            None if decode_key(data).is_some() => Some(self.value().to_owned()),
            None => None,
        }
    }

    fn jump_to_char(&mut self, needle: &str, direction: JumpDirection) {
        self.last = Action::None;
        self.reset_vertical();
        let cursor = self.state.cursor;
        let found = match direction {
            JumpDirection::Forward => {
                let start = self.next_unit(cursor);
                self.state.value[start..].find(needle).map(|at| start + at)
            }
            JumpDirection::Backward => self.state.value[..cursor].rfind(needle),
        };
        if let Some(found) = found {
            self.state.cursor = found;
        }
    }

    fn right_with_eof_preference(&mut self) {
        if self.state.cursor < self.state.value.len() {
            self.right();
            return;
        }
        let map = self.visual_map(self.last_width.max(1));
        let current = Self::current_visual_line(&map, self.state.cursor);
        if let Some((start, _)) = map.get(current) {
            self.preferred_visual_col =
                Some(visible_width(&self.state.value[*start..self.state.cursor]));
        }
        self.last = Action::None;
    }

    fn page_scroll(&mut self, direction: isize) {
        let page = 5.max(self.terminal_rows.saturating_mul(3) / 10) as isize;
        self.move_vertical(direction.saturating_mul(page));
    }
    fn visual_map(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut result = Vec::new();
        let mut absolute = 0;
        for line in self.state.value.split('\n') {
            let line_end = absolute + line.len();
            let spans = self
                .marker_spans(&self.state.value)
                .into_iter()
                .filter_map(|(start, end)| {
                    if start >= absolute && end <= line_end {
                        Some((start - absolute, end - absolute))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for chunk in word_wrap_line_atomic(line, width, &spans) {
                result.push((absolute + chunk.start_index, absolute + chunk.end_index));
            }
            absolute += line.len() + 1;
        }
        if result.is_empty() {
            result.push((0, 0));
        }
        result
    }

    fn move_vertical(&mut self, delta: isize) {
        let map = self.visual_map(self.last_width.max(1));
        let current = Self::current_visual_line(&map, self.state.cursor);
        let target = (current as isize + delta).clamp(0, map.len().saturating_sub(1) as isize);
        if delta == -1 && current == 0 {
            if self.history.is_empty() {
                self.move_to_line_start();
            } else {
                self.navigate_history(true);
            }
            return;
        }
        if delta == 1 && current + 1 >= map.len() {
            if self.history_index.is_some() {
                self.navigate_history(false);
            } else {
                self.move_to_line_end();
            }
            return;
        }
        self.move_to_visual_line(&map, current, target as usize);
        self.last = Action::None;
    }

    fn move_to_visual_line(&mut self, map: &[(usize, usize)], current: usize, target: usize) {
        let (start, end) = map[current];
        let current_col = if let Some(snapped) = self.snapped_from_cursor {
            let line = Self::current_visual_line(map, snapped);
            visible_width(&self.state.value[map[line].0..snapped.min(map[line].1)])
        } else {
            visible_width(&self.state.value[start..self.state.cursor.min(end)])
        };
        let source_is_last = current + 1 == map.len() || map[current + 1].0 > end;
        let source_max =
            visible_width(&self.state.value[start..end]).saturating_sub((!source_is_last) as usize);
        let (target_start, target_end) = map[target];
        let target_is_last = target + 1 == map.len() || map[target + 1].0 > target_end;
        let target_max = visible_width(&self.state.value[target_start..target_end])
            .saturating_sub((!target_is_last) as usize);
        let desired = self.compute_vertical_column(current_col, source_max, target_max);
        let mut byte = 0;
        let mut column = 0;
        for grapheme in self.state.value[target_start..target_end].graphemes(true) {
            let next = column + visible_width(grapheme);
            if next > desired {
                break;
            }
            byte += grapheme.len();
            column = next;
        }
        let intended = target_start + byte;
        for (marker_start, marker_end) in self.marker_spans(&self.state.value) {
            if intended < marker_start || intended >= marker_end {
                continue;
            }
            if marker_start < target_start && target > current {
                let mut next = target + 1;
                while next < map.len() && map[next].0 < marker_end {
                    next += 1;
                }
                if next < map.len() {
                    self.move_to_visual_line(map, current, next);
                    return;
                }
            }
            self.snapped_from_cursor = Some(intended);
            self.state.cursor = marker_start;
            return;
        }
        self.state.cursor = intended;
        self.snapped_from_cursor = None;
    }

    fn compute_vertical_column(
        &mut self,
        current: usize,
        source_max: usize,
        target_max: usize,
    ) -> usize {
        let cursor_in_middle = current < source_max;
        let target_too_short = target_max < current;
        if self.preferred_visual_col.is_none() || cursor_in_middle {
            if target_too_short {
                self.preferred_visual_col = Some(current);
                return target_max;
            }
            self.preferred_visual_col = None;
            return current;
        }
        let preferred = self.preferred_visual_col.unwrap_or(current);
        if target_too_short || target_max < preferred {
            return target_max;
        }
        self.preferred_visual_col = None;
        preferred
    }

    /// Backwards-compatible render entry that pins rows and focus first.
    pub fn render(&mut self, width: usize, terminal_rows: usize, focused: bool) -> Vec<String> {
        self.terminal_rows = terminal_rows;
        self.focused = focused;
        self.render_full(width)
    }

    fn border_styled(&self, text: &str) -> String {
        if self.border_open.is_empty() {
            text.to_owned()
        } else {
            format!("{}{text}{}", self.border_open, self.border_close)
        }
    }

    /// Pi's `Editor.render(width)`: horizontal borders with scroll
    /// indicators, configurable horizontal padding, wrapped rows with the
    /// fake inverse cursor, and the autocomplete list below.
    fn render_full(&mut self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let max_padding = width.saturating_sub(1) / 2;
        let padding_x = self.padding_x.min(max_padding);
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        // With padding the cursor can overflow into it; without padding one
        // column is reserved for the cursor.
        let layout_width = if padding_x > 0 {
            content_width
        } else {
            content_width.saturating_sub(1).max(1)
        };
        self.last_width = layout_width;
        let map = self.visual_map(layout_width);
        let cursor_line = Self::current_visual_line(&map, self.state.cursor);
        let max_visible = 5.max(self.terminal_rows.saturating_mul(3) / 10);
        if cursor_line < self.scroll_offset {
            self.scroll_offset = cursor_line;
        } else if cursor_line >= self.scroll_offset + max_visible {
            self.scroll_offset = cursor_line + 1 - max_visible;
        }
        self.scroll_offset = self
            .scroll_offset
            .min(map.len().saturating_sub(max_visible));
        let visible_end = (self.scroll_offset + max_visible).min(map.len());
        let left_padding = " ".repeat(padding_x);
        let mut output = Vec::new();
        if self.scroll_offset > 0 {
            let indicator = format!("─── ↑ {} more ", self.scroll_offset);
            let used = visible_width(&indicator);
            if width >= used {
                output
                    .push(self.border_styled(&format!("{indicator}{}", "─".repeat(width - used))));
            } else {
                output
                    .push(self.border_styled(&truncate_to_width(&indicator, width, "...", false)));
            }
        } else {
            output.push(self.border_styled(&"─".repeat(width)));
        }
        for (index, (start, end)) in map[self.scroll_offset..visible_end].iter().enumerate() {
            let mut text = self.state.value[*start..*end].to_owned();
            let mut line_visible_width = visible_width(&text);
            let mut cursor_in_padding = false;
            if self.scroll_offset + index == cursor_line {
                let cursor = self.state.cursor.clamp(*start, *end) - *start;
                let after = &text[cursor..];
                let marker = if self.focused { CURSOR_MARKER } else { "" };
                if after.is_empty() {
                    // Cursor at the end: an added highlighted space.
                    text = format!("{}{marker}\x1b[7m \x1b[0m", &text[..cursor]);
                    line_visible_width += 1;
                    if line_visible_width > content_width && padding_x > 0 {
                        cursor_in_padding = true;
                    }
                } else {
                    // Cursor on a grapheme: replace it with the highlighted
                    // version, keeping the visible width unchanged.
                    let unit = after.graphemes(true).next().unwrap_or(" ");
                    text = format!(
                        "{}{marker}\x1b[7m{unit}\x1b[0m{}",
                        &text[..cursor],
                        &after[unit.len().min(after.len())..]
                    );
                }
            }
            let fill = " ".repeat(content_width.saturating_sub(line_visible_width));
            let right_padding = " ".repeat(padding_x.saturating_sub(cursor_in_padding as usize));
            output.push(format!("{left_padding}{text}{fill}{right_padding}"));
        }
        let lines_below = map.len().saturating_sub(visible_end);
        if lines_below > 0 {
            let indicator = format!("─── ↓ {lines_below} more ");
            let used = visible_width(&indicator);
            output.push(self.border_styled(&format!(
                "{indicator}{}",
                "─".repeat(width.saturating_sub(used))
            )));
        } else {
            output.push(self.border_styled(&"─".repeat(width)));
        }
        if self.autocomplete_showing()
            && let Some(list) = &self.autocomplete_list
        {
            let right_padding = " ".repeat(padding_x);
            for line in list.render(content_width) {
                let fill = " ".repeat(content_width.saturating_sub(visible_width(&line)));
                output.push(format!("{left_padding}{line}{fill}{right_padding}"));
            }
        }
        output
    }
}

fn best_autocomplete_match(items: &[AutocompleteItem], prefix: &str) -> Option<usize> {
    if prefix.is_empty() {
        return None;
    }
    items
        .iter()
        .position(|item| item.value == prefix)
        .or_else(|| items.iter().position(|item| item.value.starts_with(prefix)))
}

pub fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
}

pub fn word_wrap_line(line: &str, max_width: usize) -> Vec<TextChunk> {
    if line.is_empty() || max_width == 0 {
        return vec![TextChunk {
            text: String::new(),
            start_index: 0,
            end_index: 0,
        }];
    }
    if visible_width(line) <= max_width {
        return vec![TextChunk {
            text: line.to_owned(),
            start_index: 0,
            end_index: line.len(),
        }];
    }
    let segments: Vec<(usize, &str)> = line.grapheme_indices(true).collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut current_width = 0;
    let mut wrap_opportunity: Option<(usize, usize)> = None;
    for (index, (byte, grapheme)) in segments.iter().enumerate() {
        let width = visible_width(grapheme);
        if current_width + width > max_width {
            if let Some((wrap, width_at_wrap)) = wrap_opportunity
                && current_width.saturating_sub(width_at_wrap) + width <= max_width
            {
                chunks.push(TextChunk {
                    text: line[start..wrap].to_owned(),
                    start_index: start,
                    end_index: wrap,
                });
                start = wrap;
                current_width = current_width.saturating_sub(width_at_wrap);
            } else if start < *byte {
                chunks.push(TextChunk {
                    text: line[start..*byte].to_owned(),
                    start_index: start,
                    end_index: *byte,
                });
                start = *byte;
                current_width = 0;
            }
            wrap_opportunity = None;
        }
        current_width += width;
        let next = segments.get(index + 1);
        if grapheme.chars().all(char::is_whitespace)
            && next.is_some_and(|(_, next)| !next.chars().all(char::is_whitespace))
        {
            let next_byte = next.map_or(line.len(), |(at, _)| *at);
            wrap_opportunity = Some((next_byte, current_width));
        }
    }
    chunks.push(TextChunk {
        text: line[start..].to_owned(),
        start_index: start,
        end_index: line.len(),
    });
    chunks
}

fn is_word(text: &str) -> bool {
    text.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
}

fn word_wrap_line_atomic(
    line: &str,
    max_width: usize,
    atomic_spans: &[(usize, usize)],
) -> Vec<TextChunk> {
    if atomic_spans.is_empty() || line.is_empty() || max_width == 0 {
        return word_wrap_line(line, max_width);
    }
    let mut segments = Vec::new();
    let mut at = 0;
    for &(start, end) in atomic_spans {
        for (offset, grapheme) in line[at..start].grapheme_indices(true) {
            segments.push((at + offset, grapheme, false));
        }
        segments.push((start, &line[start..end], true));
        at = end;
    }
    for (offset, grapheme) in line[at..].grapheme_indices(true) {
        segments.push((at + offset, grapheme, false));
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut current_width = 0;
    let mut wrap_opportunity: Option<(usize, usize)> = None;
    for (index, (byte, segment, atomic)) in segments.iter().enumerate() {
        let width = visible_width(segment);
        if current_width + width > max_width {
            if let Some((wrap, width_at_wrap)) = wrap_opportunity
                && current_width.saturating_sub(width_at_wrap) + width <= max_width
            {
                chunks.push(TextChunk {
                    text: line[start..wrap].to_owned(),
                    start_index: start,
                    end_index: wrap,
                });
                start = wrap;
                current_width = current_width.saturating_sub(width_at_wrap);
            } else if start < *byte {
                chunks.push(TextChunk {
                    text: line[start..*byte].to_owned(),
                    start_index: start,
                    end_index: *byte,
                });
                start = *byte;
                current_width = 0;
            }
            wrap_opportunity = None;
        }
        if *atomic && width > max_width {
            let subchunks = word_wrap_line(segment, max_width);
            for chunk in subchunks.iter().take(subchunks.len().saturating_sub(1)) {
                chunks.push(TextChunk {
                    text: chunk.text.clone(),
                    start_index: *byte + chunk.start_index,
                    end_index: *byte + chunk.end_index,
                });
            }
            if let Some(last) = subchunks.last() {
                start = *byte + last.start_index;
                current_width = visible_width(&last.text);
            }
            continue;
        }
        current_width += width;
        let next = segments.get(index + 1);
        if !*atomic
            && segment.chars().all(char::is_whitespace)
            && next
                .is_some_and(|(_, next, atomic)| *atomic || !next.chars().all(char::is_whitespace))
        {
            wrap_opportunity = Some((next.map_or(line.len(), |(at, _, _)| *at), current_width));
        }
    }
    chunks.push(TextChunk {
        text: line[start..].to_owned(),
        start_index: start,
        end_index: line.len(),
    });
    chunks
}

fn marker_spans(text: &str) -> Vec<(usize, usize, usize)> {
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(relative) = text[offset..].find("[paste #") {
        let start = offset + relative;
        let digits = start + 8;
        let Some(close_relative) = text[digits..].find(']') else {
            break;
        };
        let end = digits + close_relative + 1;
        let body = &text[digits..end - 1];
        let id_text = body.split_whitespace().next().unwrap_or("");
        if let Ok(id) = id_text.parse::<usize>() {
            result.push((start, end, id));
        }
        offset = end;
    }
    result
}

fn replace_paste_id(text: &str, id: usize, replacement: &str) -> String {
    let mut result = String::new();
    let mut copied = 0;
    for (start, end, found) in marker_spans(text) {
        if found == id {
            result.push_str(&text[copied..start]);
            result.push_str(replacement);
            copied = end;
        }
    }
    result.push_str(&text[copied..]);
    result
}

fn decode_paste_controls(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("\x1b[") {
        result.push_str(&rest[..at]);
        let sequence = &rest[at + 2..];
        let Some(end) = sequence.find('u') else {
            result.push_str(&rest[at..]);
            return result;
        };
        let body = &sequence[..end];
        if let Some((code, "5")) = body.split_once(';')
            && let Ok(code) = code.parse::<u32>()
            && let Some(ch) = char::from_u32(code)
            && ch.is_ascii_alphabetic()
        {
            result.push((ch.to_ascii_lowercase() as u8 - b'a' + 1) as char);
        } else {
            result.push_str(&rest[at..at + 3 + end]);
        }
        rest = &sequence[end + 1..];
    }
    result.push_str(rest);
    result
}

pub fn decode_key(data: &str) -> Option<String> {
    if let Some(key) = decode_protocol_key(data) {
        return Some(key);
    }
    match data {
        "\x1b" => Some("escape".to_owned()),
        "\t" => Some("tab".to_owned()),
        "\r" => Some("enter".to_owned()),
        "\n" | "\x1b\r" => Some("shift+enter".to_owned()),
        "\x7f" | "\x08" => Some("backspace".to_owned()),
        "\x1b[A" => Some("up".to_owned()),
        "\x1b[B" => Some("down".to_owned()),
        "\x1b[C" => Some("right".to_owned()),
        "\x1b[D" => Some("left".to_owned()),
        "\x1b[H" | "\x1bOH" => Some("home".to_owned()),
        "\x1b[F" | "\x1bOF" => Some("end".to_owned()),
        "\x1b[3~" => Some("delete".to_owned()),
        "\x1b\x7f" => Some("alt+backspace".to_owned()),
        "\x1bB" => Some("alt+left".to_owned()),
        "\x1bF" => Some("alt+right".to_owned()),
        "\x1b[1;5D" => Some("ctrl+left".to_owned()),
        "\x1b[1;5C" => Some("ctrl+right".to_owned()),
        "\x1b[1;3D" => Some("alt+left".to_owned()),
        "\x1b[1;3C" => Some("alt+right".to_owned()),
        "\x1b[3;3~" => Some("alt+delete".to_owned()),
        "\x1c" => Some("ctrl+\\".to_owned()),
        "\x1d" => Some("ctrl+]".to_owned()),
        "\x1f" => Some("ctrl+-".to_owned()),
        "\x00" => Some("ctrl+space".to_owned()),
        "\x1b\x1b" => Some("ctrl+alt+[".to_owned()),
        "\x1b\x1c" => Some("ctrl+alt+\\".to_owned()),
        "\x1b\x1d" => Some("ctrl+alt+]".to_owned()),
        "\x1b\x1f" => Some("ctrl+alt+-".to_owned()),
        "\x1b[5~" => Some("pageUp".to_owned()),
        "\x1b[6~" => Some("pageDown".to_owned()),
        "\x1b[Z" => Some("shift+tab".to_owned()),
        d if d.len() == 1
            && d.as_bytes()[0].is_ascii_control()
            && (1..=26).contains(&d.as_bytes()[0]) =>
        {
            Some(format!("ctrl+{}", (d.as_bytes()[0] + 96) as char))
        }
        d if d.len() == 2 && d.as_bytes()[0] == 27 && (1..=26).contains(&d.as_bytes()[1]) => {
            Some(format!("ctrl+alt+{}", (d.as_bytes()[1] + 96) as char))
        }
        d if d.len() == 2 && d.as_bytes()[0] == 27 => Some(format!("alt+{}", d.chars().nth(1)?)),
        _ => None,
    }
}

/// keys.ts "Arrow keys with modifier" / "Home/End with modifier":
/// `\x1b[1;<mod>A/B/C/D` and `\x1b[1;<mod>H/F` (with optional `:<event>`).
fn decode_modified_special(data: &str) -> Option<String> {
    let body = data.strip_prefix("\x1b[1;")?;
    let terminator = body.chars().last()?;
    let key = match terminator {
        'A' => "up",
        'B' => "down",
        'C' => "right",
        'D' => "left",
        'H' => "home",
        'F' => "end",
        _ => return None,
    };
    let mods = &body[..body.len() - 1];
    let mods = mods.split(':').next()?;
    if mods.is_empty() || !mods.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let modifier = mods.parse::<u32>().ok()?.saturating_sub(1);
    Some(format_modifiers(key.to_owned(), modifier))
}

fn decode_protocol_key(data: &str) -> Option<String> {
    if let Some(key) = decode_modified_special(data) {
        return Some(key);
    }
    let (codepoint, modifier) = if let Some(body) = data
        .strip_prefix("\x1b[27;")
        .and_then(|s| s.strip_suffix('~'))
    {
        let mut parts = body.split(';');
        let modifier = parts.next()?.parse::<u32>().ok()?.saturating_sub(1);
        (parts.next()?.parse::<u32>().ok()?, modifier)
    } else {
        let body = data.strip_prefix("\x1b[")?.strip_suffix('u')?;
        let (keys, mods) = body.split_once(';').unwrap_or((body, "1"));
        let mut key_parts = keys.split(':');
        let codepoint = key_parts.next()?.parse::<u32>().ok()?;
        let base = key_parts.nth(1).and_then(|s| s.parse::<u32>().ok());
        let modifier = mods
            .split(':')
            .next()?
            .parse::<u32>()
            .ok()?
            .saturating_sub(1);
        (
            if codepoint <= 0x7f || (57414..=57426).contains(&codepoint) {
                codepoint
            } else {
                base.unwrap_or(codepoint)
            },
            modifier,
        )
    };
    Some(format_modifiers(key_name(codepoint)?, modifier))
}

fn key_name(codepoint: u32) -> Option<String> {
    let name = match codepoint {
        27 => "escape",
        9 => "tab",
        13 | 57414 => "enter",
        32 => "space",
        127 => "backspace",
        57417 => "left",
        57418 => "right",
        57419 => "up",
        57420 => "down",
        57421 => "pageUp",
        57422 => "pageDown",
        57423 => "home",
        57424 => "end",
        57425 => "insert",
        57426 => "delete",
        _ => {
            return char::from_u32(codepoint)
                .filter(|c| !c.is_control())
                .map(|c| c.to_ascii_lowercase().to_string());
        }
    };
    Some(name.to_owned())
}

fn format_modifiers(key: String, modifier: u32) -> String {
    let effective = modifier & !(64 | 128);
    let mut prefix = String::new();
    if effective & 4 != 0 {
        prefix.push_str("ctrl+");
    }
    if effective & 1 != 0 {
        prefix.push_str("shift+");
    }
    if effective & 2 != 0 {
        prefix.push_str("alt+");
    }
    if effective & 8 != 0 {
        prefix.push_str("super+");
    }
    prefix + &key
}

pub fn decode_printable(data: &str) -> Option<String> {
    if let Some(body) = data
        .strip_prefix("\x1b[27;")
        .and_then(|s| s.strip_suffix('~'))
    {
        let mut parts = body.split(';');
        let modifier = parts.next()?.parse::<u32>().ok()?.saturating_sub(1) & !(64 | 128);
        let cp = parts.next()?.parse::<u32>().ok()?;
        return (modifier <= 1 && cp >= 32)
            .then(|| char::from_u32(cp))
            .flatten()
            .map(|c| c.to_string());
    }
    let body = data.strip_prefix("\x1b[")?.strip_suffix('u')?;
    let (keys, mods) = body.split_once(';').unwrap_or((body, "1"));
    let mut key_parts = keys.split(':');
    let cp: u32 = key_parts.next()?.parse().ok()?;
    let shifted = key_parts.next().and_then(|s| s.parse::<u32>().ok());
    let modifier = mods
        .split(':')
        .next()?
        .parse::<u32>()
        .ok()?
        .saturating_sub(1)
        & !(64 | 128);
    if modifier & !1 != 0 || cp < 32 {
        return None;
    }
    char::from_u32(if modifier & 1 != 0 {
        shifted.unwrap_or(cp)
    } else {
        cp
    })
    .map(|c| c.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn multiline_insert_normalizes_and_tracks_logical_cursor() {
        let mut editor = Editor::new("abXY");
        editor.left();
        editor.left();
        editor.insert_text_at_cursor("c\r\n\td");
        assert_eq!(editor.value(), "abc\n    dXY");
        assert_eq!(editor.logical_cursor(), Cursor { line: 1, col: 5 });
        editor.undo();
        assert_eq!((editor.value(), editor.cursor()), ("abXY", 2));
    }

    #[test]
    fn movement_and_deletion_cross_lines_by_grapheme() {
        let mut editor = Editor::new("a😀\nb");
        editor.left();
        editor.left();
        assert_eq!(editor.logical_cursor(), Cursor { line: 0, col: 5 });
        editor.backspace();
        assert_eq!(editor.value(), "a\nb");
        editor.delete();
        assert_eq!(editor.value(), "ab");
    }

    #[test]
    fn line_kills_accumulate_across_newlines() {
        let mut editor = Editor::new("one\ntwo");
        editor.kill_to_start();
        editor.kill_to_start();
        assert_eq!(editor.value(), "one");
        editor.yank();
        assert_eq!(editor.value(), "one\ntwo");
    }

    #[test]
    fn history_restores_multiline_draft() {
        let mut editor = Editor::new("draft\ntext");
        editor.add_to_history("old\nprompt");
        editor.navigate_history(true);
        assert_eq!(editor.value(), "old\nprompt");
        editor.navigate_history(false);
        assert_eq!(editor.value(), "draft\ntext");
    }

    #[test]
    fn large_paste_is_atomic_and_expands() {
        let mut editor = Editor::new("");
        let paste = (0..11)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        editor.paste(&paste);
        assert_eq!(editor.value(), "[paste #1 +11 lines]");
        assert_eq!(editor.expanded_text(), paste);
        editor.left();
        editor.delete();
        assert_eq!(editor.value(), "");
        editor.undo();
        assert_eq!(editor.value(), "[paste #1 +11 lines]");
    }

    #[test]
    fn wrapping_prefers_whitespace_and_render_marks_cursor() {
        let chunks = word_wrap_line("hello world", 7);
        assert_eq!(
            chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["hello ", "world"]
        );
        let mut editor = Editor::new("hello world");
        let rows = editor.render(8, 24, true);
        assert!(rows.iter().any(|line| line.contains(CURSOR_MARKER)));
        assert!(rows.len() >= 4);
    }

    #[test]
    fn bracketed_paste_and_submit_are_effects() {
        let mut editor = Editor::new("");
        assert_eq!(
            editor.handle_effect("\x1b[200~a\r\n\tb\x1b[201~"),
            Some(EditorEffect::Changed("a\n    b".into()))
        );
        assert_eq!(
            editor.handle_effect("\r"),
            Some(EditorEffect::Submit("a\n    b".into()))
        );
        assert_eq!(editor.value(), "");
    }

    #[test]
    fn protocol_decoding_is_preserved() {
        assert_eq!(decode_key("\x1b[97;5:1u"), Some("ctrl+a".into()));
        assert_eq!(decode_key("\x1b[57417;3u"), Some("alt+left".into()));
        assert_eq!(decode_printable("\x1b[97:65;2u"), Some("A".into()));
        assert_eq!(decode_printable("\x1b[97;5u"), None);
    }

    #[test]
    fn modified_arrow_and_home_end_sequences_decode_like_keys_ts() {
        // keys.ts: \x1b[1;<mod>A/B/C/D and \x1b[1;<mod>H/F.
        assert_eq!(decode_key("\x1b[1;3A"), Some("alt+up".into()));
        assert_eq!(decode_key("\x1b[1;3B"), Some("alt+down".into()));
        assert_eq!(decode_key("\x1b[1;5C"), Some("ctrl+right".into()));
        assert_eq!(decode_key("\x1b[1;2H"), Some("shift+home".into()));
        assert_eq!(decode_key("\x1b[1;3:1D"), Some("alt+left".into()));
        assert_eq!(decode_key("\x1b[1;xA"), None);
    }

    #[test]
    fn set_value_preserves_and_clamps_cursor() {
        let mut editor = Editor::new("éx");
        editor.left();
        editor.set_value("é");
        assert_eq!((editor.value(), editor.cursor()), ("é", 2));
        editor.set_value("a");
        assert_eq!((editor.value(), editor.cursor()), ("a", 1));
    }

    #[test]
    fn word_kills_accumulate_in_direction() {
        let mut editor = Editor::new("foo.bar baz");
        editor.delete_word_backward();
        editor.delete_word_backward();
        assert_eq!(editor.value(), "foo.");
        editor.yank();
        assert_eq!(editor.value(), "foo.bar baz");
    }
    #[test]
    fn character_jump_matches_pi_across_lines_and_cancels() {
        let mut editor = Editor::new("hello\nworld");
        for _ in 0..editor.value().len() {
            editor.left();
        }
        editor.handle_effect("\x1d");
        editor.handle_effect("o");
        assert_eq!(editor.logical_cursor(), Cursor { line: 0, col: 4 });
        editor.handle_effect("\x1d");
        editor.handle_effect("o");
        assert_eq!(editor.logical_cursor(), Cursor { line: 1, col: 1 });
        editor.handle_effect("\x1b\x1d");
        editor.handle_effect("o");
        assert_eq!(editor.logical_cursor(), Cursor { line: 0, col: 4 });
        editor.handle_effect("\x1d");
        editor.handle_effect("\x1d");
        assert_eq!(
            editor.handle_effect("x"),
            Some(EditorEffect::Changed("hellxo\nworld".into()))
        );
    }

    #[test]
    fn pending_autocomplete_does_not_swallow_submit_and_responses_are_single_use() {
        let mut editor = Editor::new("");
        editor.set_autocomplete_triggers(&[]);
        editor.handle_effect("/");
        let request = editor.take_autocomplete_request().unwrap();
        assert!(!editor.autocomplete_showing());
        assert_eq!(
            editor.handle_effect("\r"),
            Some(EditorEffect::Submit("/".into()))
        );
        let suggestions = Some(Suggestions {
            items: vec![AutocompleteItem {
                value: "model".into(),
                label: "model".into(),
                description: None,
            }],
            prefix: "/".into(),
        });
        assert_eq!(
            editor.apply_autocomplete_suggestions(request.id, suggestions.clone()),
            (false, false)
        );

        editor.handle_effect("/");
        let current = editor.take_autocomplete_request().unwrap();
        assert_eq!(
            editor.apply_autocomplete_suggestions(current.id, suggestions.clone()),
            (true, false)
        );
        assert_eq!(
            editor.apply_autocomplete_suggestions(current.id, suggestions),
            (false, false)
        );
    }

    #[test]
    fn picker_refreshes_after_equivalent_cursor_bindings() {
        let mut editor = Editor::new("");
        editor.set_autocomplete_triggers(&[]);
        editor.handle_effect("/");
        editor.handle_effect("m");
        let request = editor.take_autocomplete_request().unwrap();
        let suggestions = Suggestions {
            items: vec![AutocompleteItem {
                value: "model".into(),
                label: "model".into(),
                description: None,
            }],
            prefix: "/m".into(),
        };
        assert_eq!(
            editor.apply_autocomplete_suggestions(request.id, Some(suggestions)),
            (true, false)
        );
        editor.handle_effect("\x02");
        let refreshed = editor.take_autocomplete_request().unwrap();
        assert_eq!(refreshed.cursor_col, 1);
        assert_ne!(refreshed.id, request.id);
    }

    #[test]
    fn sticky_column_survives_short_lines_and_eof_right() {
        let mut editor = Editor::new("1234567890\n\n1234567890");
        editor.move_to_line_start();
        for _ in 0..6 {
            editor.right();
        }
        editor.move_vertical(-1);
        assert_eq!(editor.logical_cursor(), Cursor { line: 1, col: 0 });
        editor.move_vertical(-1);
        assert_eq!(editor.logical_cursor(), Cursor { line: 0, col: 6 });

        let mut wrapped = Editor::new("abcdefghij\n0123456789");
        wrapped.last_width = 4;
        wrapped.right_with_eof_preference();
        wrapped.move_vertical(-1);
        assert_eq!(wrapped.logical_cursor().col, 6);
    }

    #[test]
    fn vertical_navigation_snaps_to_atomic_marker_and_restores_column() {
        let mut editor = Editor::new("1234567890123456\n\n");
        editor.paste(&"x".repeat(2000));
        editor.insert_text_at_cursor("\n\nabcdefghijklmnop");
        editor.render(30, 24, true);
        for _ in 0..4 {
            editor.move_vertical(-1);
        }
        editor.move_to_line_start();
        for _ in 0..10 {
            editor.right();
        }
        for expected in [
            Cursor { line: 1, col: 0 },
            Cursor { line: 2, col: 0 },
            Cursor { line: 3, col: 0 },
            Cursor { line: 4, col: 10 },
        ] {
            editor.move_vertical(1);
            assert_eq!(editor.logical_cursor(), expected);
        }
    }

    #[test]
    fn vertical_navigation_skips_atomic_marker_continuation() {
        let mut editor = Editor::new("abcdefgh");
        editor.paste(&(0..100).map(|_| "line").collect::<Vec<_>>().join("\n"));
        editor.insert_text_at_cursor("ijklmnopqr\n123456789012345678");
        editor.render(20, 24, true);
        let (marker_start, marker_end) = editor.marker_spans(editor.value())[0];
        editor.move_vertical(-1);
        editor.move_to_line_start();
        for _ in 0..6 {
            editor.right();
        }
        editor.move_vertical(1);
        assert_eq!(editor.cursor(), marker_start);
        editor.move_vertical(1);
        assert_eq!(editor.cursor(), marker_end);
        editor.move_vertical(-1);
        assert_eq!(editor.cursor(), marker_start);
        editor.move_vertical(-1);
        assert_eq!(editor.cursor(), 6);

        editor.move_to_line_start();
        for _ in 0..3 {
            editor.right();
        }
        editor.move_vertical(1);
        assert_eq!(editor.cursor(), marker_start);
        editor.move_vertical(1);
        assert_eq!(editor.logical_cursor(), Cursor { line: 1, col: 3 });
    }
}
