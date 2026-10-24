//! Autocomplete: triggers, requests (synchronous here) and applying completions.

use super::edit::trim_start;
use super::text::{DEFAULT_TRIGGER_CHARACTERS, matches_trigger};
use super::{AutocompleteMode, Editor};
use crate::autocomplete::{AutocompleteItem, AutocompleteSuggestions};
use crate::components::select_list::{SelectItem, SelectList, SelectListLayoutOptions};
use crate::utils::{is_whitespace_char, js_trim};

impl Editor {
    pub(crate) fn set_autocomplete_trigger_characters(&mut self, chars: Vec<String>) {
        let mut next: Vec<String> = DEFAULT_TRIGGER_CHARACTERS
            .iter()
            .map(|s| s.to_string())
            .collect();
        for c in chars {
            if c.chars().map(char::len_utf16).sum::<usize>() != 1
                || c == "/"
                || is_whitespace_char(&c)
                || next.contains(&c)
            {
                continue;
            }
            next.push(c);
        }
        self.autocomplete_trigger_characters = next;
    }

    pub(crate) fn matches_trigger(&self, before: &str) -> bool {
        matches_trigger(before, &self.autocomplete_trigger_characters, false)
    }

    fn is_slash_menu_allowed(&self) -> bool {
        self.state.cursor_line == 0
    }

    pub(crate) fn is_at_start_of_message(&self) -> bool {
        if !self.is_slash_menu_allowed() {
            return false;
        }
        let b = self.text_before_cursor();
        let t = js_trim(&b);
        t.is_empty() || t == "/"
    }

    pub(crate) fn is_in_slash_command_context(&self, before: &str) -> bool {
        self.is_slash_menu_allowed() && trim_start(before).starts_with('/')
    }

    pub(crate) fn try_trigger_autocomplete(&mut self, explicit_tab: bool) {
        self.request_autocomplete(false, explicit_tab);
    }

    pub(crate) fn handle_tab_completion(&mut self) {
        if self.autocomplete_provider.is_none() {
            return;
        }
        let before = self.text_before_cursor();
        if self.is_in_slash_command_context(&before) && !trim_start(&before).contains(' ') {
            self.request_autocomplete(false, true);
        } else {
            self.request_autocomplete(true, true);
        }
    }

    fn request_autocomplete(&mut self, force: bool, explicit_tab: bool) {
        let Some(p) = self.autocomplete_provider.clone() else {
            return;
        };
        let (cl, cc) = (self.state.cursor_line, self.state.cursor_col);
        if force && !p.should_trigger_file_completion(&self.state.lines, cl, cc) {
            return;
        }
        let suggestions = p.get_suggestions(&self.state.lines, cl, cc, force);
        let Some(s) = suggestions.filter(|s| !s.items.is_empty()) else {
            self.cancel_autocomplete();
            self.render_requested();
            return;
        };
        if force && explicit_tab && s.items.len() == 1 {
            let item = s.items[0].clone();
            self.push_undo_snapshot();
            self.last_action = None;
            self.apply_completion(&item, &s.prefix);
            self.emit_change();
            self.render_requested();
            return;
        }
        self.apply_autocomplete_suggestions(
            s,
            if force {
                AutocompleteMode::Force
            } else {
                AutocompleteMode::Regular
            },
        );
        self.render_requested();
    }

    /// Replace text and cursor with the provider's completion of `item`.
    pub(crate) fn apply_completion(&mut self, item: &AutocompleteItem, prefix: &str) {
        let Some(p) = self.autocomplete_provider.clone() else {
            return;
        };
        let r = p.apply_completion(
            &self.state.lines,
            self.state.cursor_line,
            self.state.cursor_col,
            item,
            prefix,
        );
        self.state.lines = r.lines;
        self.state.cursor_line = r.cursor_line;
        self.set_cursor_col(r.cursor_col);
    }

    fn apply_autocomplete_suggestions(
        &mut self,
        s: AutocompleteSuggestions,
        mode: AutocompleteMode,
    ) {
        let layout = if s.prefix.starts_with('/') {
            SelectListLayoutOptions {
                min_primary_column_width: Some(12),
                max_primary_column_width: Some(32),
                truncate_primary: None,
            }
        } else {
            SelectListLayoutOptions::default()
        };
        let items: Vec<SelectItem> = s.items.iter().map(to_select_item).collect();
        let mut list = SelectList::new(
            items,
            self.autocomplete_max_visible,
            self.theme.select_list.clone(),
            layout,
        );
        let slot = self.autocomplete_picked.clone();
        list.on_select = Some(Box::new(move |it: &SelectItem| {
            *slot.borrow_mut() = Some(it.clone())
        }));
        if let Some(i) = best_match_index(&s.items, &s.prefix) {
            list.set_selected_index(i);
        }
        self.autocomplete_prefix = s.prefix;
        self.autocomplete_list = Some(list);
        self.autocomplete_state = Some(mode);
    }

    /// Apply an item the autocomplete list selected through `on_select` (upstream `list.onSelect`).
    pub(crate) fn apply_picked(&mut self) {
        let Some(it) = self.autocomplete_picked.borrow_mut().take() else {
            return;
        };
        if self.autocomplete_provider.is_none() {
            return;
        }
        self.push_undo_snapshot();
        self.last_action = None;
        let prefix = self.autocomplete_prefix.clone();
        self.apply_completion(&from_select_item(&it), &prefix);
        self.cancel_autocomplete();
        self.emit_change();
    }

    /// The selected autocomplete item, if the list is open.
    pub(crate) fn selected_autocomplete_item(&self) -> Option<AutocompleteItem> {
        self.autocomplete_list
            .as_ref()?
            .selected_item()
            .map(from_select_item)
    }

    pub(crate) fn cancel_autocomplete(&mut self) {
        self.autocomplete_state = None;
        self.autocomplete_list = None;
        self.autocomplete_prefix.clear();
    }

    pub(crate) fn update_autocomplete(&mut self) {
        let Some(mode) = self.autocomplete_state else {
            return;
        };
        if self.autocomplete_provider.is_none() {
            return;
        }
        self.request_autocomplete(mode == AutocompleteMode::Force, false);
    }
}

fn to_select_item(i: &AutocompleteItem) -> SelectItem {
    SelectItem {
        value: i.value.clone(),
        label: i.label.clone(),
        description: i.description.clone(),
    }
}

fn from_select_item(i: &SelectItem) -> AutocompleteItem {
    AutocompleteItem {
        value: i.value.clone(),
        label: i.label.clone(),
        description: i.description.clone(),
    }
}

/// Exact value match wins, else the first value starting with the prefix.
fn best_match_index(items: &[AutocompleteItem], prefix: &str) -> Option<usize> {
    if prefix.is_empty() {
        return None;
    }
    items
        .iter()
        .position(|i| i.value == prefix)
        .or_else(|| items.iter().position(|i| i.value.starts_with(prefix)))
}
