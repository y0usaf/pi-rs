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
    ("app.session.new", &[], "Start a new session"),
    ("app.session.tree", &[], "Open session tree"),
    ("app.session.fork", &[], "Fork current session"),
    ("app.session.resume", &[], "Resume a session"),
    (
        "app.session.toggleNamedFilter",
        &["ctrl+n"],
        "Toggle named session filter",
    ),
    (
        "app.session.togglePath",
        &["ctrl+p"],
        "Toggle session path display",
    ),
    (
        "app.session.toggleSort",
        &["ctrl+s"],
        "Toggle session sort mode",
    ),
    ("app.session.rename", &["ctrl+r"], "Rename session"),
    ("app.session.delete", &["ctrl+d"], "Delete session"),
    (
        "app.session.deleteNoninvasive",
        &["ctrl+backspace"],
        "Delete session when query is empty",
    ),
    (
        "app.tree.foldOrUp",
        &["ctrl+left", "alt+left"],
        "Fold tree branch or move up",
    ),
    (
        "app.tree.unfoldOrDown",
        &["ctrl+right", "alt+right"],
        "Unfold tree branch or move down",
    ),
    (
        "app.tree.filter.default",
        &["ctrl+d"],
        "Tree filter: default view",
    ),
    (
        "app.tree.filter.noTools",
        &["ctrl+t"],
        "Tree filter: hide tool results",
    ),
    (
        "app.tree.filter.userOnly",
        &["ctrl+u"],
        "Tree filter: user messages only",
    ),
    (
        "app.tree.filter.labeledOnly",
        &["ctrl+l"],
        "Tree filter: labeled entries only",
    ),
    (
        "app.tree.filter.all",
        &["ctrl+a"],
        "Tree filter: show all entries",
    ),
    (
        "app.tree.filter.cycleForward",
        &["ctrl+o"],
        "Tree filter: cycle forward",
    ),
    (
        "app.tree.filter.cycleBackward",
        &["shift+ctrl+o"],
        "Tree filter: cycle backward",
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
