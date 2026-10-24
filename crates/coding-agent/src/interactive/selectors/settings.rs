//! The /settings overlay (upstream components/settings-selector.ts and settings-submenu.ts):
//! rows that cycle a value or open a submenu, on pi_tui's settings list. The rows are the
//! settings this mode applies today or reads on the next start; the rest of upstream's list has
//! no pi-rs equivalent yet.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use pi_durable::ai::thinking::{level_name, supported_thinking_levels};
use pi_durable::ai::types::{Model, ModelThinkingLevel};
use pi_tui::components::input::{Input, InputOptions};
use pi_tui::components::select_list::{
    SelectItem, SelectList, SelectListLayoutOptions, SelectListTheme,
};
use pi_tui::components::settings_list::{
    SettingItem, SettingsList, SettingsListOptions, SubmenuDone,
};
use pi_tui::fuzzy::fuzzy_filter;
use pi_tui::keybindings::get_keybindings;
use pi_tui::tui::{Component, ComponentRef, component};

use super::{
    ActionSlot, OverlayAction, border, fit, key_display, push, select_list_theme,
    settings_list_theme, wrap,
};
use crate::interactive::theme::{bold, fg};
use crate::run_config::parse_level;

fn layout(min: usize, max: usize) -> SelectListLayoutOptions {
    SelectListLayoutOptions {
        min_primary_column_width: Some(min),
        max_primary_column_width: Some(max),
        truncate_primary: None,
    }
}

fn level_description(level: ModelThinkingLevel) -> &'static str {
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

const TRUST_LABELS: [(&str, &str); 3] = [
    ("ask", "Ask"),
    ("always", "Always trust"),
    ("never", "Never trust"),
];

fn trust_label(value: &str) -> &'static str {
    TRUST_LABELS
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, label)| *label)
        .unwrap_or("Ask")
}

fn trust_value(label: &str) -> &'static str {
    TRUST_LABELS
        .iter()
        .find(|(_, l)| *l == label)
        .map(|(v, _)| *v)
        .unwrap_or("ask")
}

/// The selections carried between a stepped submenu's steps, by step key.
pub type StepContext = BTreeMap<String, String>;

type TitleFn = Rc<dyn Fn(&StepContext) -> String>;
type OptionsFn = Rc<dyn Fn(&StepContext) -> Vec<SelectItem>>;
type PreselectFn = Rc<dyn Fn(&StepContext) -> Option<String>>;
type CompleteFn = Box<dyn FnMut(&StepContext)>;
type SetLevelFn = Box<dyn FnMut(&str, &str, ModelThinkingLevel)>;
type RemoveLevelFn = Box<dyn FnMut(&str, &str)>;

/// What a submenu does when the user picks a value or backs out.
pub struct SubmenuCallbacks {
    pub on_select: Box<dyn FnMut(&str)>,
    pub on_cancel: Box<dyn FnMut()>,
}

/// Upstream SelectSubmenu: a titled select list, optionally type-to-filter.
pub struct SelectSubmenu {
    title: String,
    description: String,
    all: Vec<SelectItem>,
    list: SelectList,
    theme: SelectListTheme,
    search: Option<Input>,
    layout: SelectListLayoutOptions,
    on_select: Box<dyn FnMut(&str)>,
    on_cancel: Box<dyn FnMut()>,
    hint: &'static str,
    selected_slot: Rc<RefCell<Option<String>>>,
    cancel_slot: Rc<RefCell<bool>>,
}

impl SelectSubmenu {
    pub fn new(
        title: &str,
        description: &str,
        options: Vec<SelectItem>,
        current_value: Option<&str>,
        searchable: bool,
        layout_options: SelectListLayoutOptions,
        callbacks: SubmenuCallbacks,
    ) -> Self {
        let theme = select_list_theme();
        let mut submenu = SelectSubmenu {
            title: title.to_string(),
            description: description.to_string(),
            all: options,
            list: SelectList::new(Vec::new(), 10, theme.clone(), layout_options.clone()),
            theme,
            search: searchable.then(|| Input::new(InputOptions::default())),
            layout: layout_options,
            on_select: callbacks.on_select,
            on_cancel: callbacks.on_cancel,
            hint: if searchable {
                "  Type to filter · Enter to select · Esc to go back"
            } else {
                "  Enter to select · Esc to go back"
            },
            selected_slot: Rc::new(RefCell::new(None)),
            cancel_slot: Rc::new(RefCell::new(false)),
        };
        submenu.rebuild(current_value.map(str::to_string));
        submenu
    }

    fn rebuild(&mut self, preselect: Option<String>) {
        let query = self
            .search
            .as_ref()
            .map(|s| s.value().to_string())
            .unwrap_or_default();
        let items = if query.is_empty() {
            self.all.clone()
        } else {
            fuzzy_filter(&self.all, &query, |item| {
                format!(
                    "{} {}",
                    item.label,
                    item.description.as_deref().unwrap_or("")
                )
            })
        };
        let preselect_index =
            preselect.and_then(|value| items.iter().position(|item| item.value == value));
        let mut list = SelectList::new(items, 10, self.theme.clone(), self.layout.clone());
        let selected = self.selected_slot.clone();
        list.on_select = Some(Box::new(move |item: &SelectItem| {
            *selected.borrow_mut() = Some(item.value.clone());
        }));
        let cancelled = self.cancel_slot.clone();
        list.on_cancel = Some(Box::new(move || {
            *cancelled.borrow_mut() = true;
        }));
        if let Some(index) = preselect_index {
            list.set_selected_index(index);
        }
        self.list = list;
    }
}

impl Component for SelectSubmenu {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![fg("accent", &bold(&self.title))];
        if !self.description.is_empty() {
            lines.push(String::new());
            lines.extend(wrap(&fg("muted", &self.description), width));
        }
        if let Some(search) = self.search.as_mut() {
            lines.push(String::new());
            lines.extend(search.render(width));
        }
        lines.push(String::new());
        lines.extend(self.list.render(width));
        lines.push(String::new());
        lines.push(fg("dim", self.hint));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        if self.search.is_some() {
            let kb = get_keybindings();
            let is_nav = kb.matches(data, "tui.select.up")
                || kb.matches(data, "tui.select.down")
                || kb.matches(data, "tui.select.confirm")
                || kb.matches(data, "tui.select.cancel");
            if is_nav {
                self.list.handle_input(data);
            } else {
                if let Some(search) = self.search.as_mut() {
                    search.handle_input(data);
                }
                let selected = self.list.selected_item().map(|i| i.value.clone());
                self.rebuild(selected);
            }
        } else {
            self.list.handle_input(data);
        }
        if let Some(value) = self.selected_slot.borrow_mut().take() {
            (self.on_select)(&value);
        } else if std::mem::take(&mut *self.cancel_slot.borrow_mut()) {
            (self.on_cancel)();
        }
    }
}

/// One step of a SteppedSubmenu.
#[derive(Clone)]
pub struct StepDef {
    pub key: String,
    pub title: TitleFn,
    pub description: TitleFn,
    pub options: OptionsFn,
    pub preselect: Option<PreselectFn>,
    pub searchable: bool,
    pub layout: Option<(usize, usize)>,
}

enum StepEvent {
    Select(String),
    Cancel,
}

type StepEvents = Rc<RefCell<Vec<StepEvent>>>;

fn build_step(
    steps: &[StepDef],
    index: usize,
    context: &BTreeMap<String, String>,
    events: StepEvents,
) -> ComponentRef {
    let step = &steps[index];
    let total = steps.len();
    let title = (step.title)(context);
    let description = (step.description)(context);
    let label = if total > 1 {
        format!("Step {}/{total} · {description}", index + 1)
    } else {
        description
    };
    let options = (step.options)(context);
    let preselect = step.preselect.as_ref().and_then(|f| f(context));
    let layout_options = match step.layout {
        Some((min, max)) => layout(min, max),
        None => layout(12, 32),
    };
    let select_events = events.clone();
    let cancel_events = events;
    component(SelectSubmenu::new(
        &title,
        &label,
        options,
        preselect.as_deref(),
        step.searchable,
        layout_options,
        SubmenuCallbacks {
            on_select: Box::new(move |value: &str| {
                select_events
                    .borrow_mut()
                    .push(StepEvent::Select(value.to_string()));
            }),
            on_cancel: Box::new(move || cancel_events.borrow_mut().push(StepEvent::Cancel)),
        },
    ))
}

/// Upstream SteppedSubmenu: a select list per step, Esc going back one step.
pub struct SteppedSubmenu {
    steps: Vec<StepDef>,
    context: BTreeMap<String, String>,
    index: usize,
    active: ComponentRef,
    events: StepEvents,
    on_complete: CompleteFn,
    summary: Box<dyn FnMut() -> String>,
    done: SubmenuDone,
    loop_steps: bool,
}

impl SteppedSubmenu {
    pub fn new(
        steps: Vec<StepDef>,
        on_complete: CompleteFn,
        summary: Box<dyn FnMut() -> String>,
        done: SubmenuDone,
        loop_steps: bool,
    ) -> Self {
        let events: StepEvents = Rc::new(RefCell::new(Vec::new()));
        let context = BTreeMap::new();
        let active = build_step(&steps, 0, &context, events.clone());
        SteppedSubmenu {
            steps,
            context,
            index: 0,
            active,
            events,
            on_complete,
            summary,
            done,
            loop_steps,
        }
    }

    fn rebuild(&mut self) {
        self.active = build_step(&self.steps, self.index, &self.context, self.events.clone());
    }

    fn finish(&mut self) {
        let summary = (self.summary)();
        self.done.close_with(&summary);
    }
}

impl Component for SteppedSubmenu {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.active.borrow_mut().render(width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        self.active.borrow_mut().handle_input(data);
        let events: Vec<StepEvent> = self.events.borrow_mut().drain(..).collect();
        for event in events {
            match event {
                StepEvent::Select(value) => {
                    let key = self.steps[self.index].key.clone();
                    self.context.insert(key, value);
                    if self.index + 1 < self.steps.len() {
                        self.index += 1;
                        self.rebuild();
                    } else {
                        (self.on_complete)(&self.context);
                        if self.loop_steps {
                            self.context.clear();
                            self.index = 0;
                            self.rebuild();
                        } else {
                            self.finish();
                        }
                    }
                }
                StepEvent::Cancel => {
                    if self.index > 0 {
                        let key = self.steps[self.index].key.clone();
                        self.context.remove(&key);
                        self.index -= 1;
                        self.rebuild();
                    } else {
                        self.finish();
                    }
                }
            }
        }
    }
}

/// What the settings overlay draws, read from the session and the settings manager.
pub struct SettingsConfig {
    pub hide_thinking_block: bool,
    pub output_pad: u8,
    pub editor_padding_x: usize,
    pub autocomplete_max_visible: usize,
    pub show_hardware_cursor: bool,
    pub clear_on_shrink: bool,
    pub default_project_trust: String,
    pub thinking_level: ModelThinkingLevel,
    pub model_thinking_levels: BTreeMap<String, String>,
    pub available_models: Vec<Model>,
    pub current_model: Option<(String, String)>,
    pub default_model: Option<(String, String)>,
}

/// What a settings row does that the row's own value cannot say: the per-model thinking
/// overrides, written from the submenu's two-step result.
pub struct SettingsCallbacks {
    pub model_thinking_level: SetLevelFn,
    pub model_thinking_level_remove: RemoveLevelFn,
}

pub type Callbacks = Rc<RefCell<SettingsCallbacks>>;
/// Reads the saved per-model thinking overrides, for the row's summary.
pub type OverridesReader = Rc<dyn Fn() -> BTreeMap<String, String>>;

fn model_key(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

fn overrides_summary(overrides: &BTreeMap<String, String>) -> String {
    if overrides.is_empty() {
        "none".to_string()
    } else {
        format!("{} configured", overrides.len())
    }
}

const CLEAR_OVERRIDE: &str = "__clear__";

fn per_model_thinking_item(
    config: &SettingsConfig,
    callbacks: &Callbacks,
    reader: &OverridesReader,
) -> SettingItem {
    let overrides = config.model_thinking_levels.clone();
    let current_key = config
        .current_model
        .as_ref()
        .map(|(p, id)| format!("{p}/{id}"));
    let default_key = config
        .default_model
        .as_ref()
        .map(|(p, id)| format!("{p}/{id}"));
    let global_level = config.thinking_level;
    let sorted = {
        let mut sorted = config.available_models.clone();
        let current = current_key.clone();
        let default = default_key.clone();
        sorted.sort_by(|a, b| {
            let (ka, kb) = (model_key(a), model_key(b));
            if Some(&ka) == current.as_ref() {
                return std::cmp::Ordering::Less;
            }
            if Some(&kb) == current.as_ref() {
                return std::cmp::Ordering::Greater;
            }
            if Some(&ka) == default.as_ref() {
                return std::cmp::Ordering::Less;
            }
            if Some(&kb) == default.as_ref() {
                return std::cmp::Ordering::Greater;
            }
            a.provider.cmp(&b.provider)
        });
        sorted
    };
    let models_by_key: BTreeMap<String, Model> =
        sorted.iter().map(|m| (model_key(m), m.clone())).collect();

    let model_step = {
        let sorted = sorted.clone();
        let overrides = overrides.clone();
        let preselect = current_key.clone().or_else(|| default_key.clone());
        StepDef {
            key: "model".to_string(),
            title: Rc::new(|_: &BTreeMap<String, String>| "Per-Model Thinking Level".to_string()),
            description: Rc::new(|_: &BTreeMap<String, String>| {
                "Select a model to configure".to_string()
            }),
            options: Rc::new(move |_: &BTreeMap<String, String>| {
                let mut items: Vec<SelectItem> = sorted
                    .iter()
                    .map(|model| {
                        let key = model_key(model);
                        SelectItem {
                            value: key.clone(),
                            label: format!(
                                "{} {}",
                                model.id,
                                fg("muted", &format!("[{}]", model.provider))
                            ),
                            description: overrides.get(&key).cloned(),
                        }
                    })
                    .collect();
                if items.is_empty() {
                    items.push(SelectItem {
                        value: "__none__".to_string(),
                        label: "No models available".to_string(),
                        description: Some(
                            "Log in to a provider or configure an API key first".to_string(),
                        ),
                    });
                }
                items
            }),
            preselect: Some(Rc::new(move |_: &BTreeMap<String, String>| {
                preselect.clone()
            })),
            searchable: true,
            layout: Some((12, 46)),
        }
    };
    let level_step = {
        let title_models = models_by_key.clone();
        let option_models = models_by_key.clone();
        let option_overrides = overrides.clone();
        let preselect_overrides = overrides.clone();
        StepDef {
            key: "level".to_string(),
            title: Rc::new(
                move |context: &BTreeMap<String, String>| match context.get("model") {
                    Some(key) => match title_models.get(key) {
                        Some(model) => {
                            format!("Thinking Level for {} [{}]", model.id, model.provider)
                        }
                        None => format!("Thinking Level for {key}"),
                    },
                    None => "Thinking Level".to_string(),
                },
            ),
            description: Rc::new(|_: &BTreeMap<String, String>| {
                "Select default thinking level for this model".to_string()
            }),
            options: Rc::new(move |context: &BTreeMap<String, String>| {
                let Some(model) = context.get("model").and_then(|k| option_models.get(k)) else {
                    return Vec::new();
                };
                let levels = if model.reasoning {
                    supported_thinking_levels(model)
                } else {
                    vec![ModelThinkingLevel::Off]
                };
                let active = context.get("model").and_then(|k| option_overrides.get(k));
                let mut items: Vec<SelectItem> = levels
                    .iter()
                    .map(|&level| {
                        let name = level_name(level).to_string();
                        SelectItem {
                            value: name.clone(),
                            label: format!(
                                "{}{name}",
                                if active == Some(&name) { "✓ " } else { "  " }
                            ),
                            description: Some(level_description(level).to_string()),
                        }
                    })
                    .collect();
                if active.is_some() {
                    items.push(SelectItem {
                        value: CLEAR_OVERRIDE.to_string(),
                        label: "  (clear override)".to_string(),
                        description: Some(format!(
                            "Revert to global default ({})",
                            level_name(global_level)
                        )),
                    });
                }
                items
            }),
            preselect: Some(Rc::new(move |context: &BTreeMap<String, String>| {
                context
                    .get("model")
                    .and_then(|k| preselect_overrides.get(k))
                    .cloned()
            })),
            searchable: false,
            layout: None,
        }
    };

    let mut item = SettingItem::new(
        "model-thinking",
        "Default thinking level per model",
        Some(&format!(
            "Override the default thinking level for specific models. {} cycles in-session.",
            key_display("app.thinking.cycle")
        )),
        &overrides_summary(&overrides),
    );
    let cbs = callbacks.clone();
    let summary_reader = reader.clone();
    item.submenu = Some(Box::new(move |_current: &str, done: SubmenuDone| {
        let complete_cbs = cbs.clone();
        let summary = summary_reader.clone();
        component(SteppedSubmenu::new(
            vec![model_step.clone(), level_step.clone()],
            Box::new(move |context: &BTreeMap<String, String>| {
                let Some(model) = context.get("model") else {
                    return;
                };
                let Some(level) = context.get("level") else {
                    return;
                };
                let Some((provider, id)) = model.split_once('/') else {
                    return;
                };
                let mut cbs = complete_cbs.borrow_mut();
                if level == CLEAR_OVERRIDE {
                    (cbs.model_thinking_level_remove)(provider, id);
                } else if let Some(parsed) = parse_level(level) {
                    (cbs.model_thinking_level)(provider, id, parsed);
                }
            }),
            Box::new(move || overrides_summary(&summary())),
            done,
            true,
        ))
    }));
    item
}

/// The /settings overlay.
pub struct SettingsSelector {
    list: SettingsList,
}

impl SettingsSelector {
    pub fn new(
        config: &SettingsConfig,
        callbacks: Callbacks,
        reader: OverridesReader,
        action: ActionSlot,
    ) -> Self {
        let mut items: Vec<SettingItem> = Vec::new();

        items.push(
            SettingItem::new(
                "hide-thinking",
                "Hide thinking",
                Some("Hide thinking blocks in the transcript"),
                if config.hide_thinking_block {
                    "true"
                } else {
                    "false"
                },
            )
            .values(&["true", "false"]),
        );
        items.push(
            SettingItem::new(
                "default-project-trust",
                "Default project trust",
                Some("Trust level for new projects that need it"),
                trust_label(&config.default_project_trust),
            )
            .values(&["Ask", "Always trust", "Never trust"]),
        );
        items.push(per_model_thinking_item(config, &callbacks, &reader));
        items.push(
            SettingItem::new(
                "show-hardware-cursor",
                "Show hardware cursor",
                Some("Use the terminal cursor instead of Pi's drawn cursor"),
                if config.show_hardware_cursor {
                    "true"
                } else {
                    "false"
                },
            )
            .values(&["true", "false"]),
        );
        items.push(
            SettingItem::new(
                "editor-padding",
                "Editor padding",
                Some("Horizontal padding for input editor (0-3)"),
                &config.editor_padding_x.to_string(),
            )
            .values(&["0", "1", "2", "3"]),
        );
        items.push(
            SettingItem::new(
                "output-padding",
                "Output padding",
                Some("Horizontal padding for messages, tool output, and command output"),
                &config.output_pad.to_string(),
            )
            .values(&["0", "1"]),
        );
        items.push(
            SettingItem::new(
                "autocomplete-max-visible",
                "Autocomplete max items",
                Some("Max visible items in autocomplete dropdown (3-20)"),
                &config.autocomplete_max_visible.to_string(),
            )
            .values(&["3", "5", "7", "10", "15", "20"]),
        );
        items.push(
            SettingItem::new(
                "clear-on-shrink",
                "Clear on shrink",
                Some("Clear empty rows when content shrinks (may cause flicker)"),
                if config.clear_on_shrink {
                    "true"
                } else {
                    "false"
                },
            )
            .values(&["true", "false"]),
        );

        let mut list = SettingsList::new(
            items,
            10,
            settings_list_theme(),
            SettingsListOptions {
                enable_search: true,
            },
        );
        let change_action = action.clone();
        list.on_change = Some(Box::new(move |id: &str, value: &str| {
            push(
                &change_action,
                OverlayAction::Setting {
                    id: id.to_string(),
                    value: if id == "default-project-trust" {
                        trust_value(value).to_string()
                    } else {
                        value.to_string()
                    },
                },
            );
        }));
        let cancel_action = action.clone();
        list.on_cancel = Some(Box::new(move || {
            push(&cancel_action, OverlayAction::Close);
        }));
        SettingsSelector { list }
    }
}

impl Component for SettingsSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![border(width)];
        lines.extend(self.list.render(width));
        lines.push(border(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn invalidate(&mut self) {
        self.list.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.list.handle_input(data);
    }
}
