//! The overlays the builtin slash commands open (upstream `modes/interactive/components`):
//! the model, thinking level, scoped model, settings and project trust selectors, on
//! `pi_tui`'s select list and settings list.

pub mod model;
pub mod scoped_models;
pub mod session;
pub mod settings;
pub mod thinking;
pub mod tree;
pub mod trust;

use pi_durable::ai::types::ModelThinkingLevel;
use pi_tui::components::select_list::SelectListTheme;
use pi_tui::components::settings_list::{SelectableStyle, SettingsListTheme, Style};
use pi_tui::keybindings::get_keybindings;
use pi_tui::utils::{truncate_to_width, visible_width, wrap_text_with_ansi};
use std::cell::RefCell;
use std::rc::Rc;

use super::theme::{fg, fg_fn};

/// What a selector asks the loop to do once the user picks something.
pub enum OverlayAction {
    /// Close the selector and give the editor focus back.
    Close,
    SetModel {
        provider: String,
        id: String,
        persist: bool,
    },
    SetThinking {
        level: ModelThinkingLevel,
        persist: bool,
    },
    /// `None` means every model is enabled.
    SetEnabledModels {
        ids: Option<Vec<String>>,
        persist: bool,
    },
    SetTrust {
        updates: Vec<TrustUpdate>,
        trusted: bool,
    },
    /// A settings row changed; `id` is the row's id.
    Setting { id: String, value: String },
    /// The session picker chose a file: switch the loop to it.
    Resume { path: String },
    /// The session tree picked an entry to move the session's leaf to.
    Navigate(pi_durable::ids::EntryId),
}

/// One project trust decision to write, `decision: None` clearing the entry.
pub struct TrustUpdate {
    pub path: String,
    pub decision: Option<bool>,
}

/// A selector's action slot, shared with the loop.
pub type ActionSlot = Rc<RefCell<Option<OverlayAction>>>;

pub fn action_slot() -> ActionSlot {
    Rc::new(RefCell::new(None))
}

pub fn push(slot: &ActionSlot, action: OverlayAction) {
    *slot.borrow_mut() = Some(action);
}

/// `DynamicBorder`: a horizontal rule the width of the screen.
pub fn border(width: usize) -> String {
    fg("border", &"─".repeat(width.max(1)))
}

/// Truncate every line to the screen width: the renderer rejects a wider line.
pub fn fit(lines: Vec<String>, width: usize) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| truncate_to_width(&line, width, "", false))
        .collect()
}

/// Word-wrap a styled line to the screen width.
pub fn wrap(line: &str, width: usize) -> Vec<String> {
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }
    wrap_text_with_ansi(line, width)
}

fn capitalize(part: &str) -> String {
    let mut chars = part.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `formatKeyText` over one key.
fn format_key(key: &str, capitalize_parts: bool) -> String {
    key.split('/')
        .map(|k| {
            k.split('+')
                .map(|part| {
                    let part = if cfg!(target_os = "macos") && part.eq_ignore_ascii_case("alt") {
                        "option"
                    } else {
                        part
                    };
                    if capitalize_parts {
                        capitalize(part)
                    } else {
                        part.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// `keyDisplayText`: the keys bound to `id`, capitalised.
pub fn key_display(id: &str) -> String {
    get_keybindings()
        .keys(id)
        .iter()
        .map(|k| format_key(k, true))
        .collect::<Vec<_>>()
        .join("/")
}

/// `keyHint`: dim keys, muted description.
pub fn key_hint(id: &str, description: &str) -> String {
    let keys = get_keybindings()
        .keys(id)
        .iter()
        .map(|k| format_key(k, false))
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "{}{}",
        fg("dim", &keys),
        fg("muted", &format!(" {description}"))
    )
}

/// `rawKeyHint`, for keys that are not a named binding.
pub fn raw_key_hint(key: &str, description: &str) -> String {
    format!(
        "{}{}",
        fg("dim", &format_key(key, false)),
        fg("muted", &format!(" {description}"))
    )
}

/// `getSelectListTheme`.
pub fn select_list_theme() -> SelectListTheme {
    SelectListTheme {
        selected_prefix: fg_fn("accent"),
        selected_text: fg_fn("accent"),
        description: fg_fn("muted"),
        scroll_info: fg_fn("muted"),
        no_match: fg_fn("muted"),
    }
}

/// `getSettingsListTheme`.
pub fn settings_list_theme() -> SettingsListTheme {
    let label: SelectableStyle = Rc::new(|text: &str, selected: bool| {
        if selected {
            fg("accent", text)
        } else {
            text.to_string()
        }
    });
    let value: SelectableStyle = Rc::new(|text: &str, selected: bool| {
        if selected {
            fg("accent", text)
        } else {
            fg("muted", text)
        }
    });
    let description: Style = Rc::new(|text: &str| fg("dim", text));
    let hint: Style = Rc::new(|text: &str| fg("dim", text));
    SettingsListTheme {
        label,
        value,
        description,
        cursor: fg("accent", "→ "),
        hint,
    }
}

/// The slice of a list to draw, centred on the selection (upstream's `updateList`).
pub fn visible_range(selected: usize, len: usize, max_visible: usize) -> (usize, usize) {
    let start = selected
        .saturating_sub(max_visible / 2)
        .min(len.saturating_sub(max_visible));
    (start, (start + max_visible).min(len))
}

/// The `(selected+1/total)` line drawn when the list scrolls.
pub fn scroll_info(selected: usize, len: usize) -> String {
    fg("muted", &format!("  ({}/{len})", selected + 1))
}

/// `getModelSearchText`: what the model selector matches a query against.
pub fn model_selector_search_text(id: &str, provider: &str, name: &str) -> String {
    let name = if name.is_empty() {
        String::new()
    } else {
        format!(" {name}")
    };
    format!("{provider} {provider}/{id} {provider} {id}{name}")
}

/// `getModelSearchText`: what the scoped-models selector matches a query against.
pub fn model_search_text(id: &str, provider: &str, name: &str) -> String {
    let name = if name.is_empty() {
        String::new()
    } else {
        format!(" {name}")
    };
    format!("{id} {provider} {provider}/{id} {provider} {id}{name}")
}

fn split_model_label(value: &str) -> Vec<String> {
    value
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// `getDistinctModelName`: the model's name when it says more than its id, else `None`.
pub fn distinct_model_name(id: &str, name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let id_words = split_model_label(id);
    let compact_id = id_words.concat();
    let covered = |word: &str| {
        id_words.iter().any(|w| w == word)
            || (word.chars().count() > 1 && compact_id.contains(word))
    };
    if split_model_label(name).iter().all(|w| covered(w)) {
        None
    } else {
        Some(name.to_string())
    }
}

/// `modelsAreEqual`.
pub fn models_are_equal(
    a: &pi_durable::ai::types::Model,
    b: &pi_durable::ai::types::Model,
) -> bool {
    a.provider == b.provider && a.id == b.id
}
