//! Upstream `core/keybindings.ts`: the app's own bindings on top of pi-tui's, installed so a
//! selector can ask for them by name and a user override applies to both.

use pi_tui::keybindings::{
    KeybindingDefinition, KeybindingsConfig, KeybindingsManager, set_keybindings, tui_keybindings,
};

const APP_DEFS: &[(&str, &[&str], &str)] = &[
    ("app.interrupt", &["escape"], "Cancel or abort"),
    ("app.clear", &["ctrl+c"], "Clear editor"),
    ("app.exit", &["ctrl+d"], "Exit when editor is empty"),
    ("app.tools.expand", &["ctrl+o"], "Toggle tool output"),
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

/// Replace the global keybindings with pi-tui's plus the app's, with the user's overrides.
pub fn install(agent_dir: &str) {
    let mut definitions = tui_keybindings();
    definitions.extend(app_keybindings());
    set_keybindings(KeybindingsManager::new(definitions, load(agent_dir)));
}

/// Legacy keybinding names, as upstream renamed them.
const MIGRATIONS: &[(&str, &str)] = &[
    ("cursorUp", "tui.editor.cursorUp"),
    ("cursorDown", "tui.editor.cursorDown"),
    ("cursorLeft", "tui.editor.cursorLeft"),
    ("cursorRight", "tui.editor.cursorRight"),
    ("cursorWordLeft", "tui.editor.cursorWordLeft"),
    ("cursorWordRight", "tui.editor.cursorWordRight"),
    ("cursorLineStart", "tui.editor.cursorLineStart"),
    ("cursorLineEnd", "tui.editor.cursorLineEnd"),
    ("jumpForward", "tui.editor.jumpForward"),
    ("jumpBackward", "tui.editor.jumpBackward"),
    ("pageUp", "tui.editor.pageUp"),
    ("pageDown", "tui.editor.pageDown"),
    ("deleteCharBackward", "tui.editor.deleteCharBackward"),
    ("deleteCharForward", "tui.editor.deleteCharForward"),
    ("deleteWordBackward", "tui.editor.deleteWordBackward"),
    ("deleteWordForward", "tui.editor.deleteWordForward"),
    ("deleteToLineStart", "tui.editor.deleteToLineStart"),
    ("deleteToLineEnd", "tui.editor.deleteToLineEnd"),
    ("yank", "tui.editor.yank"),
    ("yankPop", "tui.editor.yankPop"),
    ("undo", "tui.editor.undo"),
    ("newLine", "tui.input.newLine"),
    ("submit", "tui.input.submit"),
    ("tab", "tui.input.tab"),
    ("copy", "tui.input.copy"),
    ("selectUp", "tui.select.up"),
    ("selectDown", "tui.select.down"),
    ("selectPageUp", "tui.select.pageUp"),
    ("selectPageDown", "tui.select.pageDown"),
    ("selectConfirm", "tui.select.confirm"),
    ("selectCancel", "tui.select.cancel"),
    ("interrupt", "app.interrupt"),
    ("clear", "app.clear"),
    ("exit", "app.exit"),
    ("suspend", "app.suspend"),
    ("cycleThinkingLevel", "app.thinking.cycle"),
    ("cycleModelForward", "app.model.cycleForward"),
    ("cycleModelBackward", "app.model.cycleBackward"),
    ("selectModel", "app.model.select"),
    ("expandTools", "app.tools.expand"),
    ("toggleThinking", "app.thinking.toggle"),
    ("toggleSessionNamedFilter", "app.session.toggleNamedFilter"),
    ("externalEditor", "app.editor.external"),
    ("followUp", "app.message.followUp"),
    ("dequeue", "app.message.dequeue"),
    ("pasteImage", "app.clipboard.pasteImage"),
    ("newSession", "app.session.new"),
    ("tree", "app.session.tree"),
    ("fork", "app.session.fork"),
    ("resume", "app.session.resume"),
    ("treeFoldOrUp", "app.tree.foldOrUp"),
    ("treeUnfoldOrDown", "app.tree.unfoldOrDown"),
    ("treeEditLabel", "app.tree.editLabel"),
    ("treeToggleLabelTimestamp", "app.tree.toggleLabelTimestamp"),
    ("toggleSessionPath", "app.session.togglePath"),
    ("toggleSessionSort", "app.session.toggleSort"),
    ("renameSession", "app.session.rename"),
    ("deleteSession", "app.session.delete"),
    ("deleteSessionNoninvasive", "app.session.deleteNoninvasive"),
];

/// The named base keys upstream's schema accepts.
const NAMED_KEYS: &[&str] = &[
    "escape",
    "esc",
    "enter",
    "return",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "clear",
    "home",
    "end",
    "pageUp",
    "pageDown",
    "up",
    "down",
    "left",
    "right",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

/// The symbol base keys upstream's schema accepts: the backtick and the ASCII punctuation around
/// it, less the plus that separates modifiers.
const SYMBOL_KEYS: &[&str] = &[
    "\u{60}", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/", "!", "@", "#", "$", "%", "^", "&",
    "*", "(", ")", "_", "+", "|", "~", "{", "}", ":", "<", ">", "?",
];

/// The shape upstream's KeyIdSchema accepts: up to four distinct modifiers, then a base key.
fn is_key_id(id: &str) -> bool {
    let mut key = id;
    let mut seen: Vec<&str> = Vec::new();
    while let Some((modifier, rest)) = key.split_once('+') {
        if !matches!(modifier, "ctrl" | "shift" | "alt" | "super") || seen.contains(&modifier) {
            return false;
        }
        seen.push(modifier);
        key = rest;
    }
    if seen.len() > 4 {
        return false;
    }
    if key == "+" {
        return false;
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.is_ascii_lowercase() || c.is_ascii_digit(),
        _ => NAMED_KEYS.contains(&key) || SYMBOL_KEYS.contains(&key),
    }
}

fn keys_of(value: &serde_json::Value) -> Option<Vec<String>> {
    let one = |value: &serde_json::Value| value.as_str().map(str::to_owned);
    match value {
        serde_json::Value::String(_) => one(value).map(|key| vec![key]),
        serde_json::Value::Array(items) => items.iter().map(one).collect(),
        _ => None,
    }
}

/// keybindings.json: binding ids to keys, legacy names migrated, values the schema rejects dropped.
pub fn config(json: &serde_json::Value) -> KeybindingsConfig {
    let serde_json::Value::Object(entries) = json else {
        return Vec::new();
    };
    let mut config: KeybindingsConfig = Vec::new();
    for (name, value) in entries {
        if name == "$schema" {
            continue;
        }
        let id = MIGRATIONS
            .iter()
            .find(|(legacy, _)| legacy == name)
            .map(|(_, id)| *id)
            .unwrap_or(name);
        if config.iter().any(|(seen, _)| seen == id) {
            continue;
        }
        let Some(keys) = keys_of(value) else {
            continue;
        };
        if !keys.iter().all(|key| is_key_id(key)) {
            continue;
        }
        config.push((id.to_string(), Some(keys)));
    }
    config
}

/// The user's bindings from the agent directory's keybindings.json, or none when it is absent or
/// unreadable.
pub fn load(agent_dir: &str) -> KeybindingsConfig {
    let path = crate::paths::join(agent_dir, "keybindings.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match crate::json::parse(text) {
        Ok(json) => config(&json),
        Err(_) => Vec::new(),
    }
}
