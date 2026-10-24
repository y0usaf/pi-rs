//! The /model overlay (upstream `components/model-selector.ts`): a fuzzy search over the
//! available models, a scope toggle for the scoped set, and Enter to select or save a default.

use pi_durable::ai::types::Model;
use pi_tui::components::input::{Input, InputOptions};
use pi_tui::fuzzy::fuzzy_filter;
use pi_tui::keybindings::get_keybindings;
use pi_tui::tui::Component;

use super::{
    ActionSlot, OverlayAction, border, distinct_model_name, fit, key_display, key_hint,
    model_selector_search_text, models_are_equal, push, scroll_info, visible_range,
};
use crate::interactive::theme::fg;

const MAX_VISIBLE: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    Scoped,
}

pub struct ModelSelector {
    all: Vec<Model>,
    scoped: Vec<Model>,
    scope: Scope,
    filtered: Vec<usize>,
    selected: usize,
    current: Option<Model>,
    default: Option<(String, String)>,
    search: Input,
    action: ActionSlot,
}

impl ModelSelector {
    /// `all` is the available snapshot, `scoped` the session's scoped models, `default` the
    /// saved default as (provider, id).
    pub fn new(
        all: Vec<Model>,
        scoped: Vec<Model>,
        current: Option<Model>,
        default: Option<(String, String)>,
        initial_search: Option<&str>,
        action: ActionSlot,
    ) -> Self {
        let mut search = Input::new(InputOptions::default());
        if let Some(initial) = initial_search {
            search.set_value(initial);
        }
        let scope = if scoped.is_empty() {
            Scope::All
        } else {
            Scope::Scoped
        };
        let mut selector = ModelSelector {
            all,
            scoped,
            scope,
            filtered: Vec::new(),
            selected: 0,
            current,
            default,
            search,
            action,
        };
        selector.sort_models();
        selector.refilter();
        selector
    }

    fn sort_models(&mut self) {
        let (current, default) = (self.current.clone(), self.default.clone());
        self.all.sort_by(|a, b| {
            let a_current = current.as_ref().is_some_and(|c| models_are_equal(c, a));
            let b_current = current.as_ref().is_some_and(|c| models_are_equal(c, b));
            if a_current != b_current {
                return if a_current {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            let is_default = |m: &Model| {
                default
                    .as_ref()
                    .is_some_and(|(p, id)| *p == m.provider && *id == m.id)
            };
            match (is_default(a), is_default(b)) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.provider.cmp(&b.provider),
            }
        });
    }

    fn active(&self) -> &[Model] {
        match self.scope {
            Scope::All => &self.all,
            Scope::Scoped => &self.scoped,
        }
    }

    fn is_default(&self, model: &Model) -> bool {
        self.default
            .as_ref()
            .is_some_and(|(provider, id)| *provider == model.provider && *id == model.id)
    }

    fn is_current(&self, model: &Model) -> bool {
        self.current
            .as_ref()
            .is_some_and(|c| models_are_equal(c, model))
    }

    fn refilter(&mut self) {
        let query = self.search.value().to_string();
        let indices: Vec<usize> = (0..self.active().len()).collect();
        if query.is_empty() {
            self.filtered = indices;
            self.selected = self
                .filtered
                .iter()
                .position(|&i| {
                    self.current
                        .as_ref()
                        .is_some_and(|c| models_are_equal(c, &self.active()[i]))
                })
                .unwrap_or_else(|| self.selected.min(self.filtered.len().saturating_sub(1)));
            return;
        }
        let active = self.active();
        let default = self.default.clone();
        let is_default = |m: &Model| {
            default
                .as_ref()
                .is_some_and(|(p, id)| *p == m.provider && *id == m.id)
        };
        let mut filtered = fuzzy_filter(&indices, &query, |&i| {
            let model = &active[i];
            let badge = if is_default(model) { " default" } else { "" };
            format!(
                "{}{badge}",
                model_selector_search_text(&model.id, &model.provider, &model.name)
            )
        });
        if "default".starts_with(&query.trim().to_lowercase()) && !query.trim().is_empty() {
            let defaults: Vec<usize> = (0..active.len())
                .filter(|&i| is_default(&active[i]))
                .collect();
            if !defaults.is_empty() {
                let rest: Vec<usize> = filtered
                    .into_iter()
                    .filter(|i| !defaults.contains(i))
                    .collect();
                filtered = defaults.into_iter().chain(rest).collect();
            }
        }
        self.filtered = filtered;
        self.selected = 0;
    }

    fn set_scope(&mut self, scope: Scope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.refilter();
    }

    fn rows(&self, out: &mut Vec<String>) {
        let len = self.filtered.len();
        let (start, end) = visible_range(self.selected, len, MAX_VISIBLE);
        for i in start..end {
            let model = &self.active()[self.filtered[i]];
            let is_selected = i == self.selected;
            let cursor = if is_selected {
                fg("accent", "→ ")
            } else {
                "  ".to_string()
            };
            let marker = if self.is_current(model) {
                fg("accent", "✓ ")
            } else {
                "  ".to_string()
            };
            let id = if is_selected {
                fg("accent", &model.id)
            } else {
                model.id.clone()
            };
            let name = match distinct_model_name(&model.id, &model.name) {
                Some(name) => fg("muted", &format!(" ({name})")),
                None => String::new(),
            };
            let provider = fg("muted", &format!("[{}]", model.provider));
            let badge = if self.is_default(model) {
                fg("muted", " · default")
            } else {
                String::new()
            };
            out.push(format!("{cursor}{marker}{id}{name} {provider}{badge}"));
        }
        if start > 0 || end < len {
            out.push(scroll_info(self.selected, len));
        }
        if len == 0 {
            out.push(fg("muted", "  No matching models"));
        } else {
            out.push(String::new());
            let selected = &self.active()[self.filtered[self.selected]];
            out.push(fg("muted", &format!("  Model Name: {}", selected.name)));
        }
    }

    fn scope_line(&self) -> String {
        let all = if self.scope == Scope::All {
            fg("accent", "all")
        } else {
            fg("muted", "all")
        };
        let scoped = if self.scope == Scope::Scoped {
            fg("accent", "scoped")
        } else {
            fg("muted", "scoped")
        };
        format!(
            "{}{all}{}{scoped}",
            fg("muted", "Scope: "),
            fg("muted", " | ")
        )
    }

    fn choose(&self, persist: bool) {
        let Some(&index) = self.filtered.get(self.selected) else {
            return;
        };
        let model = &self.active()[index];
        push(
            &self.action,
            OverlayAction::SetModel {
                provider: model.provider.clone(),
                id: model.id.clone(),
                persist,
            },
        );
    }
}

impl Component for ModelSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![border(width), String::new()];
        if self.scoped.is_empty() {
            lines.push(fg(
                "warning",
                "Only showing models from configured providers. Use /login to add providers.",
            ));
        } else {
            lines.push(self.scope_line());
            lines.push(key_hint("tui.input.tab", "scope") + &fg("muted", " (all/scoped)"));
        }
        lines.push(String::new());
        lines.extend(self.search.render(width));
        lines.push(String::new());
        self.rows(&mut lines);
        lines.push(String::new());
        lines.push(fg(
            "dim",
            &format!(
                "  {} to select · {} to set as default · {} to cancel",
                key_display("tui.select.confirm"),
                key_display("app.models.save"),
                key_display("tui.select.cancel")
            ),
        ));
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
        if kb.matches(data, "tui.input.tab") {
            if !self.scoped.is_empty() {
                let next = if self.scope == Scope::All {
                    Scope::Scoped
                } else {
                    Scope::All
                };
                self.set_scope(next);
            }
            return;
        }
        if kb.matches(data, "tui.select.up") {
            let len = self.filtered.len();
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
            let len = self.filtered.len();
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
        if kb.matches(data, "tui.select.confirm") {
            self.choose(false);
            return;
        }
        if kb.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Close);
            return;
        }
        if kb.matches(data, "app.models.save") {
            self.choose(true);
            return;
        }
        self.search.handle_input(data);
        self.refilter();
    }
}
