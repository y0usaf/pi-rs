//! A list of settings, each cycling through values or opening a submenu (upstream
//! components/settings-list.ts).

use crate::components::input::{Input, InputOptions};
use crate::fuzzy::fuzzy_filter;
use crate::keybindings::get_keybindings;
use crate::tui::{Component, ComponentRef};
use crate::utils::{truncate_to_width, visible_width, wrap_text_with_ansi};
use std::cell::RefCell;
use std::rc::Rc;

/// How a submenu reports that it is finished: the value it selected, if any, and the setting to
/// move the cursor to afterwards.
#[derive(Default)]
pub struct SubmenuOutcome {
    pub closed: bool,
    pub value: Option<String>,
    pub navigate_to: Option<String>,
}

#[derive(Clone, Default)]
pub struct SubmenuDone(Rc<RefCell<SubmenuOutcome>>);

impl SubmenuDone {
    pub fn close(&self) {
        self.0.borrow_mut().closed = true;
    }
    pub fn close_with(&self, value: &str) {
        let mut o = self.0.borrow_mut();
        o.closed = true;
        o.value = Some(value.to_string());
    }
    pub fn close_with_navigate(&self, value: &str, navigate_to: &str) {
        let mut o = self.0.borrow_mut();
        o.closed = true;
        o.value = Some(value.to_string());
        o.navigate_to = Some(navigate_to.to_string());
    }
    fn take(&self) -> Option<SubmenuOutcome> {
        let mut o = self.0.borrow_mut();
        if !o.closed {
            return None;
        }
        Some(std::mem::take(&mut *o))
    }
}

/// Opens a submenu for a setting, given its current value.
pub type SubmenuFactory = Box<dyn FnMut(&str, SubmenuDone) -> ComponentRef>;

pub struct SettingItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub current_value: String,
    pub values: Option<Vec<String>>,
    pub submenu: Option<SubmenuFactory>,
}

impl SettingItem {
    pub fn new(id: &str, label: &str, description: Option<&str>, current_value: &str) -> Self {
        SettingItem {
            id: id.into(),
            label: label.into(),
            description: description.map(Into::into),
            current_value: current_value.into(),
            values: None,
            submenu: None,
        }
    }
    pub fn values(mut self, values: &[&str]) -> Self {
        self.values = Some(values.iter().map(|v| v.to_string()).collect());
        self
    }
    pub fn submenu(mut self, f: SubmenuFactory) -> Self {
        self.submenu = Some(f);
        self
    }
}

pub type Style = Rc<dyn Fn(&str) -> String>;
pub type SelectableStyle = Rc<dyn Fn(&str, bool) -> String>;

pub struct SettingsListTheme {
    pub label: SelectableStyle,
    pub value: SelectableStyle,
    pub description: Style,
    pub cursor: String,
    pub hint: Style,
}

#[derive(Default)]
pub struct SettingsListOptions {
    pub enable_search: bool,
}

/// Called with the setting's id and its new value.
pub type ChangeCallback = Box<dyn FnMut(&str, &str)>;
/// Called when the list is dismissed.
pub type CancelCallback = Box<dyn FnMut()>;

pub struct SettingsList {
    items: Vec<SettingItem>,
    filtered: Vec<usize>,
    theme: SettingsListTheme,
    selected_index: usize,
    max_visible: usize,
    pub on_change: Option<ChangeCallback>,
    pub on_cancel: Option<CancelCallback>,
    search_input: Option<Input>,
    search_enabled: bool,
    submenu_component: Option<ComponentRef>,
    submenu_done: Option<SubmenuDone>,
    submenu_item_index: Option<usize>,
}

impl SettingsList {
    pub fn new(
        items: Vec<SettingItem>,
        max_visible: usize,
        theme: SettingsListTheme,
        options: SettingsListOptions,
    ) -> Self {
        let filtered = (0..items.len()).collect();
        SettingsList {
            items,
            filtered,
            theme,
            selected_index: 0,
            max_visible,
            on_change: None,
            on_cancel: None,
            search_input: options
                .enable_search
                .then(|| Input::new(InputOptions::default())),
            search_enabled: options.enable_search,
            submenu_component: None,
            submenu_done: None,
            submenu_item_index: None,
        }
    }

    pub fn set_value(&mut self, id: &str, value: &str) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.current_value = value.to_string();
        }
    }

    pub fn value(&self, id: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.current_value.as_str())
    }

    pub fn select_item(&mut self, id: &str) {
        let display = self.display_indices();
        if let Some(index) = display.iter().position(|&i| self.items[i].id == id) {
            self.selected_index = index;
        }
    }

    fn display_indices(&self) -> Vec<usize> {
        if self.search_enabled {
            self.filtered.clone()
        } else {
            (0..self.items.len()).collect()
        }
    }

    fn render_main(&mut self, width: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        if let Some(input) = self.search_input.as_mut() {
            lines.extend(input.render(width));
            lines.push(String::new());
        }
        if self.items.is_empty() {
            lines.push((self.theme.hint)("  No settings available"));
            if self.search_enabled {
                self.add_hint_line(&mut lines, width);
            }
            return lines;
        }
        let display = self.display_indices();
        if display.is_empty() {
            lines.push(truncate_to_width(
                &(self.theme.hint)("  No matching settings"),
                width,
                "",
                false,
            ));
            self.add_hint_line(&mut lines, width);
            return lines;
        }
        let (start, end) = self.visible_range(display.len());
        let max_label_width = self
            .items
            .iter()
            .map(|item| visible_width(&item.label))
            .max()
            .unwrap_or(0)
            .min(36);
        for (i, &index) in display.iter().enumerate().take(end).skip(start) {
            let item = &self.items[index];
            let is_selected = i == self.selected_index;
            let prefix = if is_selected {
                self.theme.cursor.clone()
            } else {
                "  ".to_string()
            };
            let pad = max_label_width.saturating_sub(visible_width(&item.label));
            let label_padded = format!("{}{}", item.label, " ".repeat(pad));
            let label_text = (self.theme.label)(&label_padded, is_selected);
            let separator = "  ";
            let used = visible_width(&prefix) + max_label_width + visible_width(separator);
            let value_max_width = width.saturating_sub(used).saturating_sub(2);
            let value_text = (self.theme.value)(
                &truncate_to_width(&item.current_value, value_max_width, "", false),
                is_selected,
            );
            lines.push(truncate_to_width(
                &format!("{prefix}{label_text}{separator}{value_text}"),
                width,
                "",
                false,
            ));
        }
        if start > 0 || end < display.len() {
            let scroll = format!("  ({}/{})", self.selected_index + 1, display.len());
            lines.push((self.theme.hint)(&truncate_to_width(
                &scroll,
                width.saturating_sub(2),
                "",
                false,
            )));
        }
        if let Some(description) = self.items[display[self.selected_index]].description.clone() {
            lines.push(String::new());
            for line in wrap_text_with_ansi(&description, width.saturating_sub(4)) {
                lines.push((self.theme.description)(&format!("  {line}")));
            }
        }
        self.add_hint_line(&mut lines, width);
        lines
    }

    fn visible_range(&self, len: usize) -> (usize, usize) {
        let start = self
            .selected_index
            .saturating_sub(self.max_visible / 2)
            .min(len.saturating_sub(self.max_visible));
        (start, (start + self.max_visible).min(len))
    }

    fn activate(&mut self) {
        let display = self.display_indices();
        let Some(&index) = display.get(self.selected_index) else {
            return;
        };
        let item = &mut self.items[index];
        if let Some(open) = item.submenu.as_mut() {
            let done = SubmenuDone::default();
            let current = item.current_value.clone();
            self.submenu_component = Some(open(&current, done.clone()));
            self.submenu_done = Some(done);
            self.submenu_item_index = Some(self.selected_index);
            return;
        }
        if let Some(values) = item.values.clone() {
            if values.is_empty() {
                return;
            }
            let next = match values.iter().position(|v| *v == item.current_value) {
                Some(i) => values[(i + 1) % values.len()].clone(),
                None => values[0].clone(),
            };
            item.current_value = next.clone();
            let id = item.id.clone();
            if let Some(change) = self.on_change.as_mut() {
                change(&id, &next);
            }
        }
    }

    fn finish_submenu(&mut self) {
        let Some(done) = self.submenu_done.clone() else {
            return;
        };
        let Some(outcome) = done.take() else {
            return;
        };
        self.submenu_component = None;
        self.submenu_done = None;
        let item_index = self.submenu_item_index.take();
        if let Some(value) = &outcome.value
            && let Some(index) = item_index
        {
            self.items[index].current_value = value.clone();
            let id = self.items[index].id.clone();
            if let Some(change) = self.on_change.as_mut() {
                change(&id, value);
            }
        }
        if let Some(navigate_to) = outcome.navigate_to {
            self.select_item(&navigate_to);
            self.activate();
        } else if let Some(index) = item_index {
            self.selected_index = index;
        }
    }

    fn apply_filter(&mut self, query: &str) {
        let indices: Vec<usize> = (0..self.items.len()).collect();
        self.filtered = fuzzy_filter(&indices, query, |&i| self.items[i].label.clone());
        self.selected_index = 0;
    }

    fn add_hint_line(&self, lines: &mut Vec<String>, width: usize) {
        lines.push(String::new());
        let hint = if self.search_enabled {
            "  Type to search · Enter/Space to change · Esc to cancel"
        } else {
            "  Enter/Space to change · Esc to cancel"
        };
        lines.push(truncate_to_width(
            &(self.theme.hint)(hint),
            width,
            "",
            false,
        ));
    }
}

impl Component for SettingsList {
    fn render(&mut self, width: usize) -> Vec<String> {
        if let Some(submenu) = self.submenu_component.clone() {
            return submenu.borrow_mut().render(width);
        }
        self.render_main(width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn invalidate(&mut self) {
        if let Some(submenu) = self.submenu_component.as_ref() {
            submenu.borrow_mut().invalidate();
        }
    }

    fn children(&self) -> Option<Vec<ComponentRef>> {
        self.submenu_component.clone().map(|c| vec![c])
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(submenu) = self.submenu_component.clone() {
            submenu.borrow_mut().handle_input(data);
            self.finish_submenu();
            return;
        }
        let kb = get_keybindings();
        let len = self.display_indices().len();
        if kb.matches(data, "tui.select.up") {
            if len == 0 {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                len - 1
            } else {
                self.selected_index - 1
            };
        } else if kb.matches(data, "tui.select.down") {
            if len == 0 {
                return;
            }
            self.selected_index = if self.selected_index == len - 1 {
                0
            } else {
                self.selected_index + 1
            };
        } else if kb.matches(data, "tui.select.confirm")
            || (data == " "
                && (!self.search_enabled
                    || self
                        .search_input
                        .as_ref()
                        .is_none_or(|i| i.value().is_empty())))
        {
            self.activate();
        } else if kb.matches(data, "tui.select.cancel") {
            if let Some(cancel) = self.on_cancel.as_mut() {
                cancel();
            }
        } else if self.search_enabled
            && let Some(input) = self.search_input.as_mut()
        {
            input.handle_input(data);
            let query = input.value().to_string();
            self.apply_filter(&query);
        }
    }
}
