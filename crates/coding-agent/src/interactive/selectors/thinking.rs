//! The /thinking overlay (upstream `components/thinking-selector.ts`): the levels the current
//! model accepts, type-to-filter, Enter to set for the session, Ctrl+S to save the default.

use pi_durable::ai::thinking::level_name;
use pi_durable::ai::types::ModelThinkingLevel;
use pi_tui::components::input::{Input, InputOptions};
use pi_tui::components::select_list::{
    SelectItem, SelectList, SelectListLayoutOptions, SelectListTheme,
};
use pi_tui::fuzzy::fuzzy_filter;
use pi_tui::keybindings::get_keybindings;
use pi_tui::tui::Component;

use super::{ActionSlot, OverlayAction, border, fit, key_display, push, select_list_theme};
use crate::interactive::theme::fg;
use crate::run_config::parse_level;

const MAX_VISIBLE: usize = 10;

fn description(level: ModelThinkingLevel) -> &'static str {
    match level {
        ModelThinkingLevel::Off => "No reasoning",
        ModelThinkingLevel::Minimal => "Very brief reasoning (~1k tokens)",
        ModelThinkingLevel::Low => "Light reasoning (~2k tokens)",
        ModelThinkingLevel::Medium => "Moderate reasoning (~8k tokens)",
        ModelThinkingLevel::High => "Deep reasoning (~16k tokens)",
        ModelThinkingLevel::Xhigh => "Extra-high reasoning (~32k tokens)",
        ModelThinkingLevel::Max => "Maximum reasoning",
    }
}

pub struct ThinkingSelector {
    all: Vec<SelectItem>,
    list: SelectList,
    theme: SelectListTheme,
    search: Input,
    action: ActionSlot,
}

impl ThinkingSelector {
    pub fn new(
        current: ModelThinkingLevel,
        available: Vec<ModelThinkingLevel>,
        default: Option<ModelThinkingLevel>,
        action: ActionSlot,
    ) -> Self {
        let all: Vec<SelectItem> = available
            .iter()
            .map(|&level| SelectItem {
                value: level_name(level).to_string(),
                label: format!(
                    "{}{}",
                    if level == current { "✓ " } else { "  " },
                    level_name(level)
                ),
                description: Some(if Some(level) == default {
                    format!("{} · default", description(level))
                } else {
                    description(level).to_string()
                }),
            })
            .collect();
        let theme = select_list_theme();
        let mut selector = ThinkingSelector {
            all,
            list: SelectList::new(
                Vec::new(),
                MAX_VISIBLE,
                theme.clone(),
                SelectListLayoutOptions {
                    min_primary_column_width: Some(12),
                    max_primary_column_width: Some(32),
                    truncate_primary: None,
                },
            ),
            theme,
            search: Input::new(InputOptions::default()),
            action,
        };
        selector.rebuild(None);
        selector
    }

    fn filtered(&self, query: &str) -> Vec<SelectItem> {
        if query.is_empty() {
            return self.all.clone();
        }
        fuzzy_filter(&self.all, query, |item| {
            format!(
                "{} {}",
                item.value,
                item.description.as_deref().unwrap_or("")
            )
        })
    }

    fn rebuild(&mut self, preselect: Option<String>) {
        let query = self.search.value().to_string();
        let items = self.filtered(&query);
        let preselect_index =
            preselect.and_then(|value| items.iter().position(|item| item.value == value));
        let action = self.action.clone();
        let mut list = SelectList::new(
            items,
            MAX_VISIBLE,
            self.theme.clone(),
            SelectListLayoutOptions {
                min_primary_column_width: Some(12),
                max_primary_column_width: Some(32),
                truncate_primary: None,
            },
        );
        list.on_select = Some(Box::new(move |item: &SelectItem| {
            if let Some(level) = parse_level(&item.value) {
                push(
                    &action,
                    OverlayAction::SetThinking {
                        level,
                        persist: false,
                    },
                );
            }
        }));
        let cancel = self.action.clone();
        list.on_cancel = Some(Box::new(move || {
            push(&cancel, OverlayAction::Close);
        }));
        if let Some(index) = preselect_index {
            list.set_selected_index(index);
        }
        self.list = list;
    }
}

impl Component for ThinkingSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![
            border(width),
            String::new(),
            "Thinking Level".to_string(),
            String::new(),
            format!(
                "{} cycles thinking levels in-session",
                key_display("app.thinking.cycle")
            ),
            String::new(),
        ];
        lines.extend(self.search.render(width));
        lines.push(String::new());
        lines.extend(self.list.render(width));
        lines.push(String::new());
        lines.push(fg(
            "dim",
            &format!(
                "  {} to select · {} to set as default · {} to cancel",
                key_display("tui.select.confirm"),
                key_display("app.thinking.save"),
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
        if kb.matches(data, "app.thinking.save") {
            if let Some(item) = self.list.selected_item()
                && let Some(level) = parse_level(&item.value)
            {
                push(
                    &self.action,
                    OverlayAction::SetThinking {
                        level,
                        persist: true,
                    },
                );
            }
            return;
        }
        let is_nav = kb.matches(data, "tui.select.up")
            || kb.matches(data, "tui.select.down")
            || kb.matches(data, "tui.select.confirm")
            || kb.matches(data, "tui.select.cancel");
        if is_nav {
            self.list.handle_input(data);
            return;
        }
        self.search.handle_input(data);
        let selected = self.list.selected_item().map(|i| i.value.clone());
        self.rebuild(selected);
    }
}
