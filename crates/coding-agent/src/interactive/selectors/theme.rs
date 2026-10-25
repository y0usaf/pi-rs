//! The /settings theme item (upstream components/settings-selector.ts ThemeSubmenu): one theme for
//! every terminal, or a light/dark pair that follows the terminal's appearance. Highlighting a
//! theme previews it.

use std::cell::RefCell;
use std::rc::Rc;

use pi_tui::components::select_list::{SelectItem, SelectListLayoutOptions};
use pi_tui::components::settings_list::{
    SettingItem, SettingsList, SettingsListOptions, SubmenuDone,
};
use pi_tui::tui::{Component, ComponentRef, component};

use super::settings::{ChangeFn, SelectSubmenu, SubmenuCallbacks};
use super::{ActionSlot, OverlayAction, fit, push, settings_list_theme};
use crate::interactive::theme::{self, SYSTEM_THEME, bold, fg};

/// The row that switches the item into automatic mode, and the value it stands for.
const AUTOMATIC: &str = "/";

fn layout() -> SelectListLayoutOptions {
    SelectListLayoutOptions {
        min_primary_column_width: Some(12),
        max_primary_column_width: Some(32),
        truncate_primary: None,
    }
}

fn theme_items(themes: &[String], current: &str) -> Vec<SelectItem> {
    themes
        .iter()
        .map(|name| SelectItem {
            value: name.clone(),
            label: format!("{}{name}", if name == current { "✓ " } else { "  " }),
            description: (name == SYSTEM_THEME)
                .then(|| "Theme created from your terminal's colors".to_string()),
        })
        .collect()
}

/// The system theme comes first, then automatic mode, then the remaining themes.
fn single_mode_items(themes: &[String], current: &str) -> Vec<SelectItem> {
    let items = theme_items(themes, current);
    let mut system = Vec::new();
    let mut rest = Vec::new();
    for item in items {
        if item.value == SYSTEM_THEME {
            system.push(item);
        } else {
            rest.push(item);
        }
    }
    system.push(SelectItem {
        value: AUTOMATIC.to_string(),
        label: "  automatic".to_string(),
        description: Some("Use separate themes for light and dark terminal appearance".to_string()),
    });
    system.extend(rest);
    system
}

fn preferred(themes: &[String], preferred: Option<&str>, fallback: &str) -> String {
    if let Some(preferred) = preferred
        && themes.iter().any(|name| name == preferred)
    {
        return preferred.to_string();
    }
    if themes.iter().any(|name| name == fallback) {
        return fallback.to_string();
    }
    themes
        .first()
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Single,
    Automatic,
}

enum Event {
    /// Preview a theme name, or the pair when it is "/".
    Preview(String),
    /// Keep a theme name, or the pair when it is "/", and save it.
    Apply(String),
    Cancel,
    Single,
    Automatic,
    Light(String),
    Dark(String),
}

type Events = Rc<RefCell<Vec<Event>>>;

/// The /settings theme row's submenu.
pub struct ThemeSubmenu {
    original: String,
    mode: Mode,
    single: String,
    light: String,
    dark: String,
    themes: Vec<String>,
    events: Events,
    action: ActionSlot,
    done: SubmenuDone,
    active: ComponentRef,
}

impl ThemeSubmenu {
    pub fn new(setting: &str, themes: Vec<String>, action: ActionSlot, done: SubmenuDone) -> Self {
        let pair = theme::parse_auto(Some(setting));
        let single = preferred(themes.as_slice(), Some(setting), SYSTEM_THEME);
        let (light, dark) = match &pair {
            Some((light, dark)) => (light.clone(), dark.clone()),
            None => (single.clone(), single.clone()),
        };
        let mode = if pair.is_some() {
            Mode::Automatic
        } else {
            Mode::Single
        };
        let events: Events = Rc::new(RefCell::new(Vec::new()));
        let mut submenu = ThemeSubmenu {
            original: setting.to_string(),
            mode,
            single,
            light,
            dark,
            themes,
            events: events.clone(),
            action,
            done,
            active: component(Panel::new(Vec::new())),
        };
        submenu.rebuild();
        submenu
    }

    fn pair(&self) -> String {
        format!("{}/{}", self.light, self.dark)
    }

    /// The theme the pair resolves to on this terminal.
    fn active_automatic(&self) -> String {
        match theme::terminal_appearance() {
            theme::Appearance::Light => self.light.clone(),
            theme::Appearance::Dark => self.dark.clone(),
        }
    }

    fn rebuild(&mut self) {
        self.active = match self.mode {
            Mode::Single => self.single_menu(),
            Mode::Automatic => self.automatic_menu(),
        };
    }

    fn single_menu(&mut self) -> ComponentRef {
        let events = self.events.clone();
        let select = events.clone();
        let cancel = events.clone();
        let preview = events;
        component(SelectSubmenu::new(
            "Theme",
            "Select a theme, or choose automatic to follow terminal appearance.",
            single_mode_items(&self.themes, &self.single),
            Some(&self.single),
            false,
            layout(),
            SubmenuCallbacks {
                on_select: Box::new(move |value: &str| {
                    let event = if value == AUTOMATIC {
                        Event::Automatic
                    } else {
                        Event::Apply(value.to_string())
                    };
                    select.borrow_mut().push(event);
                }),
                on_cancel: Box::new(move || cancel.borrow_mut().push(Event::Cancel)),
                on_change: Some(Box::new(move |value: &str| {
                    preview.borrow_mut().push(Event::Preview(value.to_string()));
                })),
            },
        ))
    }

    fn automatic_menu(&mut self) -> ComponentRef {
        let themes = self.themes.clone();
        let events = self.events.clone();
        let light_done = events.clone();
        let light_preview = events.clone();
        let dark_done = events.clone();
        let dark_preview = events.clone();
        let cancelled = events.clone();
        let changed = events;
        let light_item = SettingItem::new(
            "light-theme",
            "Light theme",
            Some("Theme to use in automatic mode when the terminal is light"),
            &self.light,
        )
        .submenu(Box::new(move |current: &str, done: SubmenuDone| {
            let pick = light_done.clone();
            let preview = light_preview.clone();
            let close = done.clone();
            theme_select(
                "Light Theme",
                "Select the theme to use for light terminal appearance",
                &themes,
                current,
                Box::new(move |value: &str| {
                    pick.borrow_mut().push(Event::Light(value.to_string()));
                    close.close_with(value);
                }),
                Box::new(move |value: &str| {
                    preview.borrow_mut().push(Event::Preview(value.to_string()));
                }),
            )
        }));
        let themes = self.themes.clone();
        let dark_item = SettingItem::new(
            "dark-theme",
            "Dark theme",
            Some("Theme to use in automatic mode when the terminal is dark"),
            &self.dark,
        )
        .submenu(Box::new(move |current: &str, done: SubmenuDone| {
            let pick = dark_done.clone();
            let preview = dark_preview.clone();
            let close = done.clone();
            theme_select(
                "Dark Theme",
                "Select the theme to use for dark terminal appearance",
                &themes,
                current,
                Box::new(move |value: &str| {
                    pick.borrow_mut().push(Event::Dark(value.to_string()));
                    close.close_with(value);
                }),
                Box::new(move |value: &str| {
                    preview.borrow_mut().push(Event::Preview(value.to_string()));
                }),
            )
        }));
        let mut list = SettingsList::new(
            vec![
                light_item,
                dark_item,
                SettingItem::new(
                    "apply",
                    "Apply",
                    Some("Save and go back"),
                    "save and go back",
                )
                .values(&["save and go back"]),
                SettingItem::new(
                    "single-mode",
                    "Change mode",
                    Some("Switch to one theme for light and dark"),
                    "switch to single theme",
                )
                .values(&["switch to single theme"]),
            ],
            10,
            settings_list_theme(),
            SettingsListOptions::default(),
        );
        list.on_change = Some(Box::new(move |id: &str, _value: &str| {
            let event = match id {
                "apply" => Event::Apply(AUTOMATIC.to_string()),
                "single-mode" => Event::Single,
                _ => return,
            };
            changed.borrow_mut().push(event);
        }));
        list.on_cancel = Some(Box::new(move || cancelled.borrow_mut().push(Event::Cancel)));
        let header = Panel::new(vec![
            fg("accent", &bold("Automatic Theme")),
            String::new(),
            fg(
                "muted",
                "Choose themes for terminal light and dark appearance.",
            ),
            fg("muted", "Light/dark detection requires terminal support."),
            String::new(),
        ]);
        component(Nested {
            header: Rc::new(RefCell::new(header)),
            body: component(list),
        })
    }

    fn preview(&self, setting: &str) {
        let resolved = if setting == AUTOMATIC {
            self.pair()
        } else {
            setting.to_string()
        };
        push(&self.action, OverlayAction::PreviewTheme(resolved));
    }

    fn handle_events(&mut self) {
        let events: Vec<Event> = self.events.borrow_mut().drain(..).collect();
        for event in events {
            match event {
                Event::Preview(value) => {
                    if self.mode == Mode::Single && value == AUTOMATIC {
                        self.preview(AUTOMATIC);
                    } else {
                        self.preview(&value);
                    }
                }
                // Closing with a value hands it to the settings row, which applies and saves it.
                Event::Apply(value) if value == AUTOMATIC => {
                    self.done.close_with(&self.pair());
                }
                Event::Apply(value) => {
                    self.done.close_with(&value);
                }
                Event::Cancel => {
                    push(
                        &self.action,
                        OverlayAction::PreviewTheme(self.original.clone()),
                    );
                    self.done.close();
                }
                Event::Single => {
                    self.mode = Mode::Single;
                    self.single = self.active_automatic();
                    self.preview(&self.single.clone());
                    self.rebuild();
                }
                Event::Automatic => {
                    self.mode = Mode::Automatic;
                    self.preview(AUTOMATIC);
                    self.rebuild();
                }
                Event::Light(value) => {
                    self.light = value;
                    self.preview(AUTOMATIC);
                }
                Event::Dark(value) => {
                    self.dark = value;
                    self.preview(AUTOMATIC);
                }
            }
        }
    }
}

/// A titled select list for one theme of the pair.
fn theme_select(
    title: &str,
    description: &str,
    themes: &[String],
    current: &str,
    on_select: Box<dyn FnMut(&str)>,
    on_change: ChangeFn,
) -> ComponentRef {
    component(SelectSubmenu::new(
        title,
        description,
        theme_items(themes, current),
        Some(current),
        false,
        layout(),
        SubmenuCallbacks {
            on_select,
            on_cancel: Box::new(|| {}),
            on_change: Some(on_change),
        },
    ))
}

impl Component for ThemeSubmenu {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.active.borrow_mut().render(width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn invalidate(&mut self) {
        self.active.borrow_mut().invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.active.borrow_mut().handle_input(data);
        self.handle_events();
    }
}

/// Lines drawn as they are, above whatever the component below draws.
pub struct Panel {
    lines: Vec<String>,
}

impl Panel {
    pub fn new(lines: Vec<String>) -> Self {
        Panel { lines }
    }
}

impl Component for Panel {
    fn render(&mut self, width: usize) -> Vec<String> {
        fit(self.lines.clone(), width)
    }
    fn accepts_input(&self) -> bool {
        false
    }
}

/// A header above a component that takes the input.
pub struct Nested {
    header: Rc<RefCell<Panel>>,
    body: ComponentRef,
}

impl Component for Nested {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = self.header.borrow_mut().render(width);
        lines.extend(self.body.borrow_mut().render(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn invalidate(&mut self) {
        self.body.borrow_mut().invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.body.borrow_mut().handle_input(data);
    }
}
