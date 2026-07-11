//! Pi-compatible settings list selection, cycling, fuzzy search, and rendering.

use std::sync::{Arc, Mutex};

use crate::{
    component::Component,
    editor::decode_key,
    fuzzy::fuzzy_filter,
    input::Input,
    utils::{truncate_to_width, visible_width, wrap_text_with_ansi},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub current_value: String,
    pub values: Vec<String>,
    pub submenu: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsListAction {
    Changed { id: String, value: String },
    Submenu { id: String, current_value: String },
    Cancel,
    None,
}

type SelectedStyle = Arc<dyn Fn(&str, bool) -> String + Send + Sync>;
type TextStyle = Arc<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Clone)]
pub struct SettingsTheme {
    pub label: SelectedStyle,
    pub value: SelectedStyle,
    pub description: TextStyle,
    pub cursor: String,
    pub hint: TextStyle,
}

impl Default for SettingsTheme {
    fn default() -> Self {
        Self {
            label: Arc::new(|text, _| text.to_owned()),
            value: Arc::new(|text, _| text.to_owned()),
            description: Arc::new(str::to_owned),
            cursor: "→ ".to_owned(),
            hint: Arc::new(str::to_owned),
        }
    }
}

struct State {
    items: Vec<SettingItem>,
    filtered: Vec<usize>,
    selected: usize,
}

pub struct SettingsList {
    state: Mutex<State>,
    query: Input,
    max_visible: usize,
    theme: SettingsTheme,
    search: bool,
}

impl SettingsList {
    pub fn new(
        items: Vec<SettingItem>,
        max_visible: usize,
        theme: SettingsTheme,
        search: bool,
    ) -> Self {
        let filtered = (0..items.len()).collect();
        Self {
            state: Mutex::new(State {
                items,
                filtered,
                selected: 0,
            }),
            query: Input::default(),
            max_visible,
            theme,
            search,
        }
    }

    pub fn selected(&self) -> Option<SettingItem> {
        let state = self.state.lock().ok()?;
        let index = if self.search {
            *state.filtered.get(state.selected)?
        } else {
            state.selected
        };
        state.items.get(index).cloned()
    }

    pub fn update_value(&self, id: &str, value: impl Into<String>) {
        if let Ok(mut state) = self.state.lock()
            && let Some(item) = state.items.iter_mut().find(|item| item.id == id)
        {
            item.current_value = value.into();
        }
    }

    pub fn set_query(&self, query: &str) {
        self.query.set_value(query);
        self.apply_filter();
    }

    pub fn query(&self) -> String {
        self.query.value()
    }

    pub fn select_id(&self, id: &str) {
        if let Ok(mut state) = self.state.lock() {
            let item_index = state.items.iter().position(|item| item.id == id);
            if let Some(item_index) = item_index {
                state.selected = if self.search {
                    state
                        .filtered
                        .iter()
                        .position(|index| *index == item_index)
                        .unwrap_or(0)
                } else {
                    item_index
                };
            }
        }
    }

    pub fn move_up(&self) {
        if let Ok(mut state) = self.state.lock() {
            let count = if self.search {
                state.filtered.len()
            } else {
                state.items.len()
            };
            if count > 0 {
                state.selected = if state.selected == 0 {
                    count - 1
                } else {
                    state.selected - 1
                };
            }
        }
    }

    pub fn move_down(&self) {
        if let Ok(mut state) = self.state.lock() {
            let count = if self.search {
                state.filtered.len()
            } else {
                state.items.len()
            };
            if count > 0 {
                state.selected = (state.selected + 1) % count;
            }
        }
    }

    pub fn activate(&self) -> SettingsListAction {
        let Ok(mut state) = self.state.lock() else {
            return SettingsListAction::None;
        };
        let Some(index) = (if self.search {
            state.filtered.get(state.selected).copied()
        } else {
            Some(state.selected)
        }) else {
            return SettingsListAction::None;
        };
        let Some(item) = state.items.get_mut(index) else {
            return SettingsListAction::None;
        };
        if item.submenu {
            return SettingsListAction::Submenu {
                id: item.id.clone(),
                current_value: item.current_value.clone(),
            };
        }
        if item.values.is_empty() {
            return SettingsListAction::None;
        }
        let next = item
            .values
            .iter()
            .position(|value| value == &item.current_value)
            .map_or(0, |current| (current + 1) % item.values.len());
        item.current_value = item.values[next].clone();
        SettingsListAction::Changed {
            id: item.id.clone(),
            value: item.current_value.clone(),
        }
    }

    pub fn handle_input(&self, data: &str) -> SettingsListAction {
        match decode_key(data).as_deref() {
            Some("up") => self.move_up(),
            Some("down") => self.move_down(),
            Some("enter") => return self.activate(),
            Some("escape") | Some("ctrl+c") => return SettingsListAction::Cancel,
            _ if data == " " => return self.activate(),
            _ if self.search => {
                let sanitized = data.replace(' ', "");
                if sanitized.is_empty() {
                    return SettingsListAction::None;
                }
                self.query.handle_input(&sanitized);
                self.apply_filter();
            }
            _ => return SettingsListAction::None,
        }
        SettingsListAction::None
    }

    fn apply_filter(&self) {
        let query = self.query.value();
        if let Ok(mut state) = self.state.lock() {
            state.filtered = fuzzy_filter((0..state.items.len()).collect(), &query, |index| {
                state.items[*index].label.clone()
            });
            state.selected = 0;
        }
    }

    fn hint(&self) -> &'static str {
        if self.search {
            "  Type to search · Enter/Space to change · Esc to cancel"
        } else {
            "  Enter/Space to change · Esc to cancel"
        }
    }

    fn add_hint_line(&self, lines: &mut Vec<String>, width: usize) {
        lines.push(String::new());
        lines.push(truncate_to_width(
            &(self.theme.hint)(self.hint()),
            width,
            "...",
            false,
        ));
    }
}

impl Component for SettingsList {
    fn render(&self, width: usize) -> Vec<String> {
        let Ok(state) = self.state.lock() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        if self.search {
            lines.extend(self.query.render(width));
            lines.push(String::new());
        }
        if state.items.is_empty() {
            lines.push((self.theme.hint)("  No settings available"));
            if self.search {
                self.add_hint_line(&mut lines, width);
            }
            return lines;
        }
        let display = if self.search {
            state.filtered.clone()
        } else {
            (0..state.items.len()).collect::<Vec<_>>()
        };
        if display.is_empty() {
            lines.push(truncate_to_width(
                &(self.theme.hint)("  No matching settings"),
                width,
                "...",
                false,
            ));
            self.add_hint_line(&mut lines, width);
            return lines;
        }
        let start = state
            .selected
            .saturating_sub(self.max_visible / 2)
            .min(display.len().saturating_sub(self.max_visible));
        let end = (start + self.max_visible).min(display.len());
        let max_label = state
            .items
            .iter()
            .map(|item| visible_width(&item.label))
            .max()
            .unwrap_or(0)
            .min(30);
        for (position, item_index) in display.iter().enumerate().take(end).skip(start) {
            let Some(item) = state.items.get(*item_index) else {
                continue;
            };
            let selected = position == state.selected;
            let prefix = if selected { &self.theme.cursor } else { "  " };
            let prefix_width = visible_width(prefix);
            let padded = format!(
                "{}{}",
                item.label,
                " ".repeat(max_label.saturating_sub(visible_width(&item.label)))
            );
            let label = (self.theme.label)(&padded, selected);
            let used_width = prefix_width + max_label + 2;
            let value = truncate_to_width(
                &item.current_value,
                width.saturating_sub(used_width + 2),
                "",
                false,
            );
            let value = (self.theme.value)(&value, selected);
            lines.push(truncate_to_width(
                &format!("{prefix}{label}  {value}"),
                width,
                "...",
                false,
            ));
        }
        if start > 0 || end < display.len() {
            lines.push((self.theme.hint)(&truncate_to_width(
                &format!("  ({}/{})", state.selected + 1, display.len()),
                width.saturating_sub(2),
                "",
                false,
            )));
        }
        if let Some(description) = display
            .get(state.selected)
            .and_then(|index| state.items.get(*index))
            .and_then(|item| item.description.as_deref())
        {
            lines.push(String::new());
            for line in wrap_text_with_ansi(description, width.saturating_sub(4)) {
                lines.push((self.theme.description)(&format!("  {line}")));
            }
        }
        self.add_hint_line(&mut lines, width);
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, label: &str, value: &str) -> SettingItem {
        SettingItem {
            id: id.into(),
            label: label.into(),
            description: None,
            current_value: value.into(),
            values: vec!["off".into(), "on".into()],
            submenu: false,
        }
    }

    #[test]
    fn cycles_and_wraps() {
        let settings = SettingsList::new(
            vec![item("a", "Alpha", "off"), item("b", "Beta", "on")],
            1,
            SettingsTheme::default(),
            false,
        );
        assert_eq!(
            settings.activate(),
            SettingsListAction::Changed {
                id: "a".into(),
                value: "on".into()
            }
        );
        settings.move_up();
        assert_eq!(settings.selected().map(|item| item.id), Some("b".into()));
    }

    #[test]
    fn filter_and_reference_layout() {
        let settings = SettingsList::new(
            vec![item("a", "Alpha", "off"), item("b", "Beta", "on")],
            2,
            SettingsTheme::default(),
            true,
        );
        settings.set_query("bt");
        let rendered = settings.render(30);
        assert!(rendered[0].starts_with("> "));
        assert!(rendered[0].contains('b'));
        assert_eq!(rendered[2], "→ Beta   on");
    }

    #[test]
    fn submenu_activation_does_not_change_value() {
        let mut submenu = item("theme", "Theme", "dark");
        submenu.submenu = true;
        let settings = SettingsList::new(vec![submenu], 1, SettingsTheme::default(), true);
        assert_eq!(
            settings.handle_input("\r"),
            SettingsListAction::Submenu {
                id: "theme".into(),
                current_value: "dark".into()
            }
        );
        assert_eq!(
            settings.selected().map(|item| item.current_value),
            Some("dark".into())
        );
    }
}
