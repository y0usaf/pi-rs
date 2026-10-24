//! Upstream `core/keybindings.ts`: the app's own bindings on top of pi-tui's, installed so a
//! selector can ask for them by name and a user override applies to both.

use pi_tui::keybindings::{
    KeybindingDefinition, KeybindingsManager, get_keybindings, set_keybindings, tui_keybindings,
};

const APP_DEFS: &[(&str, &[&str], &str)] = &[
    ("app.thinking.cycle", &["shift+tab"], "Cycle thinking level"),
    ("app.thinking.save", &["ctrl+s"], "Save thinking level"),
    ("app.thinking.toggle", &["ctrl+t"], "Toggle thinking blocks"),
    ("app.models.save", &["ctrl+s"], "Save model selection"),
    ("app.models.enableAll", &["ctrl+a"], "Enable all models"),
    ("app.models.clearAll", &["ctrl+x"], "Clear all models"),
    (
        "app.models.toggleProvider",
        &["ctrl+p"],
        "Toggle all models for provider",
    ),
    (
        "app.models.reorderUp",
        &["alt+up"],
        "Move model up in order",
    ),
    (
        "app.models.reorderDown",
        &["alt+down"],
        "Move model down in order",
    ),
];

/// The app bindings, in upstream's declaration order.
pub fn app_keybindings() -> Vec<(String, KeybindingDefinition)> {
    APP_DEFS
        .iter()
        .map(|(id, keys, description)| {
            (
                id.to_string(),
                KeybindingDefinition {
                    default_keys: keys.iter().map(|k| k.to_string()).collect(),
                    description: Some(description.to_string()),
                },
            )
        })
        .collect()
}

/// Replace the global keybindings with pi-tui's plus the app's, keeping any user overrides.
pub fn install() {
    let mut definitions = tui_keybindings();
    definitions.extend(app_keybindings());
    let user = get_keybindings().user_bindings();
    set_keybindings(KeybindingsManager::new(definitions, user));
}
