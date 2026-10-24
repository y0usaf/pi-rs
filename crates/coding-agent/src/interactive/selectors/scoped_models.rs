//! The /scoped-models overlay (upstream components/scoped-models-selector.ts): which models are
//! enabled, in what order. Changes are session-only until Ctrl+S writes them to settings.

use pi_durable::ai::types::Model;
use pi_tui::components::input::{Input, InputOptions};
use pi_tui::fuzzy::fuzzy_filter;
use pi_tui::keybindings::get_keybindings;
use pi_tui::keys::matches_key;
use pi_tui::tui::Component;

use super::{
    ActionSlot, OverlayAction, border, distinct_model_name, fit, key_display, model_search_text,
    push, scroll_info, visible_range, wrap,
};
use crate::interactive::theme::{bold, fg, strikethrough};

const MAX_VISIBLE: usize = 8;

fn is_enabled(enabled: &Option<Vec<String>>, id: &str) -> bool {
    match enabled {
        None => true,
        Some(ids) => ids.iter().any(|i| i == id),
    }
}

fn normalize_enabled(result: Vec<String>, all: &[String]) -> Option<Vec<String>> {
    let covers_all =
        result.len() == all.len() && result.iter().all(|id| all.iter().any(|a| a == id));
    if covers_all { None } else { Some(result) }
}

fn toggle(enabled: Option<Vec<String>>, all: &[String], id: &str) -> Option<Vec<String>> {
    let Some(mut ids) = enabled else {
        return Some(all.iter().filter(|a| a.as_str() != id).cloned().collect());
    };
    if let Some(index) = ids.iter().position(|i| i == id) {
        ids.remove(index);
        return Some(ids);
    }
    ids.push(id.to_string());
    normalize_enabled(ids, all)
}

fn enable_all(
    enabled: Option<Vec<String>>,
    all: &[String],
    targets: Option<&[String]>,
) -> Option<Vec<String>> {
    let mut ids = enabled?;
    for id in targets.unwrap_or(all) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    normalize_enabled(ids, all)
}

fn clear_all(
    enabled: Option<Vec<String>>,
    all: &[String],
    targets: Option<&[String]>,
) -> Option<Vec<String>> {
    match enabled {
        None => match targets {
            Some(targets) => Some(
                all.iter()
                    .filter(|a| !targets.contains(a))
                    .cloned()
                    .collect(),
            ),
            None => Some(Vec::new()),
        },
        Some(ids) => {
            let targets: Vec<String> = targets.map(|t| t.to_vec()).unwrap_or_else(|| ids.clone());
            Some(ids.into_iter().filter(|id| !targets.contains(id)).collect())
        }
    }
}

fn move_id(enabled: Option<Vec<String>>, id: &str, delta: i64) -> Option<Vec<String>> {
    let mut ids = enabled?;
    let index = ids.iter().position(|i| i == id)?;
    let new_index = index as i64 + delta;
    if new_index < 0 || new_index >= ids.len() as i64 {
        return Some(ids);
    }
    let new_index = new_index as usize;
    ids.swap(index, new_index);
    Some(ids)
}

fn sorted_ids(enabled: &Option<Vec<String>>, all: &[String]) -> Vec<String> {
    match enabled {
        None => all.to_vec(),
        Some(ids) => {
            let mut out = ids.clone();
            out.extend(all.iter().filter(|id| !ids.contains(id)).cloned());
            out
        }
    }
}

#[derive(Clone)]
struct Item {
    full_id: String,
    model: Option<Model>,
    enabled: bool,
}

pub struct ScopedModelsSelector {
    models: Vec<Model>,
    all_ids: Vec<String>,
    enabled: Option<Vec<String>>,
    items: Vec<Item>,
    selected: usize,
    search: Input,
    dirty: bool,
    action: ActionSlot,
}

impl ScopedModelsSelector {
    pub fn new(models: Vec<Model>, enabled: Option<Vec<String>>, action: ActionSlot) -> Self {
        let all_ids: Vec<String> = models
            .iter()
            .map(|m| format!("{}/{}", m.provider, m.id))
            .collect();
        let mut selector = ScopedModelsSelector {
            models,
            all_ids,
            enabled,
            items: Vec::new(),
            selected: 0,
            search: Input::new(InputOptions::default()),
            dirty: false,
            action,
        };
        selector.refresh();
        selector
    }

    fn model_by_id(&self, full_id: &str) -> Option<&Model> {
        self.models
            .iter()
            .find(|m| format!("{}/{}", m.provider, m.id) == full_id)
    }

    fn build_items(&self, query: &str) -> Vec<Item> {
        let items: Vec<Item> = sorted_ids(&self.enabled, &self.all_ids)
            .into_iter()
            .map(|full_id| Item {
                model: self.model_by_id(&full_id).cloned(),
                enabled: is_enabled(&self.enabled, &full_id),
                full_id,
            })
            .collect();
        if query.is_empty() {
            return items;
        }
        fuzzy_filter(&items, query, |item| match &item.model {
            Some(model) => model_search_text(&model.id, &model.provider, &model.name),
            None => item.full_id.clone(),
        })
    }

    fn refresh(&mut self) {
        let query = self.search.value().to_string();
        self.items = self.build_items(&query);
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
    }

    fn notify(&self, persist: bool) {
        push(
            &self.action,
            OverlayAction::SetEnabledModels {
                ids: self.enabled.clone(),
                persist,
            },
        );
    }

    fn footer_text(&self) -> String {
        let enabled_count = match &self.enabled {
            None => self.all_ids.len(),
            Some(ids) => ids.iter().filter(|id| self.all_ids.contains(id)).count(),
        };
        let unavailable = match &self.enabled {
            None => 0,
            Some(ids) => ids.iter().filter(|id| !self.all_ids.contains(id)).count(),
        };
        let count_text = if self.enabled.is_none() {
            "all enabled".to_string()
        } else if unavailable > 0 {
            format!(
                "{enabled_count}/{} enabled · {unavailable} unavailable",
                self.all_ids.len()
            )
        } else {
            format!("{enabled_count}/{} enabled", self.all_ids.len())
        };
        let parts = [
            format!("{} toggle", key_display("tui.select.confirm")),
            format!("{} all", key_display("app.models.enableAll")),
            format!("{} clear", key_display("app.models.clearAll")),
            format!("{} provider", key_display("app.models.toggleProvider")),
            format!(
                "{}/{} reorder",
                key_display("app.models.reorderUp"),
                key_display("app.models.reorderDown")
            ),
            format!("{} save", key_display("app.models.save")),
            count_text,
        ];
        let line = fg("dim", &format!("  {} ", parts.join(" · ")));
        if self.dirty {
            format!("{line}{}", fg("warning", "(unsaved)"))
        } else {
            line
        }
    }

    fn rows(&self, out: &mut Vec<String>) {
        if self.items.is_empty() {
            out.push(fg("muted", "  No matching models"));
            return;
        }
        let (start, end) = visible_range(self.selected, self.items.len(), MAX_VISIBLE);
        for i in start..end {
            let item = &self.items[i];
            let is_selected = i == self.selected;
            let prefix = if is_selected {
                fg("accent", "→ ")
            } else {
                "  ".to_string()
            };
            let id = match &item.model {
                Some(model) => model.id.clone(),
                None => strikethrough(&item.full_id),
            };
            let id = if is_selected { fg("accent", &id) } else { id };
            let name = match &item.model {
                Some(model) => match distinct_model_name(&model.id, &model.name) {
                    Some(name) => fg("muted", &format!(" ({name})")),
                    None => String::new(),
                },
                None => String::new(),
            };
            let provider = fg(
                "muted",
                &match &item.model {
                    Some(model) => format!(" [{}]", model.provider),
                    None => " [unavailable]".to_string(),
                },
            );
            let status = if item.model.is_some() && item.enabled {
                fg("accent", "✓ ")
            } else {
                "  ".to_string()
            };
            out.push(format!("{prefix}{status}{id}{name}{provider}"));
        }
        if start > 0 || end < self.items.len() {
            out.push(scroll_info(self.selected, self.items.len()));
        }
        if let Some(item) = self.items.get(self.selected) {
            out.push(String::new());
            out.push(fg(
                "muted",
                &match &item.model {
                    Some(model) => format!("  Model Name: {}", model.name),
                    None => "  Model unavailable".to_string(),
                },
            ));
        }
    }
}

impl Component for ScopedModelsSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![
            border(width),
            String::new(),
            fg("accent", &bold("Model Configuration")),
            fg(
                "muted",
                &format!(
                    "Session-only. {} to save to settings.",
                    key_display("app.models.save")
                ),
            ),
            String::new(),
        ];
        lines.extend(self.search.render(width));
        lines.push(String::new());
        self.rows(&mut lines);
        lines.push(String::new());
        lines.extend(wrap(&self.footer_text(), width));
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
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        let len = self.items.len();
        if kb.matches(data, "tui.select.up") {
            if len == 0 {
                return;
            }
            self.selected = if self.selected == 0 {
                len - 1
            } else {
                self.selected - 1
            };
            return;
        }
        if kb.matches(data, "tui.select.down") {
            if len == 0 {
                return;
            }
            self.selected = if self.selected == len - 1 {
                0
            } else {
                self.selected + 1
            };
            return;
        }
        let reorder_up = kb.matches(data, "app.models.reorderUp");
        let reorder_down = kb.matches(data, "app.models.reorderDown");
        if reorder_up || reorder_down {
            if self.enabled.is_none() {
                return;
            }
            let Some(item) = self.items.get(self.selected) else {
                return;
            };
            let id = item.full_id.clone();
            if !is_enabled(&self.enabled, &id) {
                return;
            }
            let delta = if reorder_up { -1 } else { 1 };
            let position = self
                .enabled
                .as_ref()
                .and_then(|ids| ids.iter().position(|i| *i == id));
            let Some(position) = position else {
                return;
            };
            let target = position as i64 + delta;
            let ids_len = self.enabled.as_ref().map(|ids| ids.len()).unwrap_or(0) as i64;
            if target < 0 || target >= ids_len {
                return;
            }
            self.enabled = move_id(self.enabled.clone(), &id, delta);
            self.dirty = true;
            self.selected = (self.selected as i64 + delta) as usize;
            self.refresh();
            self.notify(false);
            return;
        }
        if kb.matches(data, "tui.select.confirm") {
            let Some(item) = self.items.get(self.selected) else {
                return;
            };
            let id = item.full_id.clone();
            self.enabled = toggle(self.enabled.clone(), &self.all_ids, &id);
            self.dirty = true;
            self.refresh();
            self.notify(false);
            return;
        }
        let query = self.search.value().to_string();
        let filtered_ids: Option<Vec<String>> = if query.is_empty() {
            None
        } else {
            Some(self.items.iter().map(|i| i.full_id.clone()).collect())
        };
        if kb.matches(data, "app.models.enableAll") {
            self.enabled = enable_all(self.enabled.clone(), &self.all_ids, filtered_ids.as_deref());
            self.dirty = true;
            self.refresh();
            self.notify(false);
            return;
        }
        if kb.matches(data, "app.models.clearAll") {
            self.enabled = clear_all(self.enabled.clone(), &self.all_ids, filtered_ids.as_deref());
            self.dirty = true;
            self.refresh();
            self.notify(false);
            return;
        }
        if kb.matches(data, "app.models.toggleProvider") {
            let Some(item) = self.items.get(self.selected) else {
                return;
            };
            let Some(model) = item.model.clone() else {
                return;
            };
            let provider_ids: Vec<String> = self
                .models
                .iter()
                .filter(|m| m.provider == model.provider)
                .map(|m| format!("{}/{}", m.provider, m.id))
                .collect();
            let all_enabled = provider_ids.iter().all(|id| is_enabled(&self.enabled, id));
            self.enabled = if all_enabled {
                clear_all(self.enabled.clone(), &self.all_ids, Some(&provider_ids))
            } else {
                enable_all(self.enabled.clone(), &self.all_ids, Some(&provider_ids))
            };
            self.dirty = true;
            self.refresh();
            self.notify(false);
            return;
        }
        if kb.matches(data, "app.models.save") {
            self.notify(true);
            self.dirty = false;
            return;
        }
        if matches_key(data, "ctrl+c") {
            if !self.search.value().is_empty() {
                self.search.set_value("");
                self.refresh();
            } else {
                push(&self.action, OverlayAction::Close);
            }
            return;
        }
        if matches_key(data, "escape") {
            push(&self.action, OverlayAction::Close);
            return;
        }
        self.search.handle_input(data);
        self.refresh();
    }
}
