//! Named keybindings with user overrides (upstream `keybindings.ts`).

use crate::keys::matches_key;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// A keybinding's defaults and description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingDefinition {
    pub default_keys: Vec<String>,
    pub description: Option<String>,
}

/// Definitions by id, in declaration order.
pub type KeybindingDefinitions = Vec<(String, KeybindingDefinition)>;
/// User overrides in insertion order: id to keys (`None` means "use the default").
pub type KeybindingsConfig = Vec<(String, Option<Vec<String>>)>;

/// Two or more user bindings claiming one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub key: String,
    pub keybindings: Vec<String>,
}

const TUI_DEFS: &[(&str, &[&str], &str)] = &[
    ("tui.editor.cursorUp", &["up"], "Move cursor up"),
    ("tui.editor.cursorDown", &["down"], "Move cursor down"),
    (
        "tui.editor.historyPrevious",
        &[],
        "Select previous prompt history entry",
    ),
    (
        "tui.editor.historyNext",
        &[],
        "Select next prompt history entry",
    ),
    (
        "tui.editor.cursorLeft",
        &["left", "ctrl+b"],
        "Move cursor left",
    ),
    (
        "tui.editor.cursorRight",
        &["right", "ctrl+f"],
        "Move cursor right",
    ),
    (
        "tui.editor.cursorWordLeft",
        &["alt+left", "ctrl+left", "alt+b"],
        "Move cursor word left",
    ),
    (
        "tui.editor.cursorWordRight",
        &["alt+right", "ctrl+right", "alt+f"],
        "Move cursor word right",
    ),
    (
        "tui.editor.cursorLineStart",
        &["home", "ctrl+a"],
        "Move to line start",
    ),
    (
        "tui.editor.cursorLineEnd",
        &["end", "ctrl+e"],
        "Move to line end",
    ),
    (
        "tui.editor.jumpForward",
        &["ctrl+]"],
        "Jump forward to character",
    ),
    (
        "tui.editor.jumpBackward",
        &["ctrl+alt+]"],
        "Jump backward to character",
    ),
    ("tui.editor.pageUp", &["pageUp", "ctrl+pageUp"], "Page up"),
    (
        "tui.editor.pageDown",
        &["pageDown", "ctrl+pageDown"],
        "Page down",
    ),
    (
        "tui.editor.deleteCharBackward",
        &["backspace"],
        "Delete character backward",
    ),
    (
        "tui.editor.deleteCharForward",
        &["delete", "ctrl+d"],
        "Delete character forward",
    ),
    (
        "tui.editor.deleteWordBackward",
        &["ctrl+w", "alt+backspace"],
        "Delete word backward",
    ),
    (
        "tui.editor.deleteWordForward",
        &["alt+d", "alt+delete"],
        "Delete word forward",
    ),
    (
        "tui.editor.deleteToLineStart",
        &["ctrl+u"],
        "Delete to line start",
    ),
    (
        "tui.editor.deleteToLineEnd",
        &["ctrl+k"],
        "Delete to line end",
    ),
    ("tui.editor.yank", &["ctrl+y"], "Yank"),
    ("tui.editor.yankPop", &["alt+y"], "Yank pop"),
    ("tui.editor.undo", &["ctrl+-"], "Undo"),
    (
        "tui.input.newLine",
        &["shift+enter", "ctrl+j"],
        "Insert newline",
    ),
    ("tui.input.submit", &["enter"], "Submit input"),
    ("tui.input.tab", &["tab"], "Tab / autocomplete"),
    ("tui.input.copy", &["ctrl+c"], "Copy selection"),
    ("tui.select.up", &["up"], "Move selection up"),
    ("tui.select.down", &["down"], "Move selection down"),
    ("tui.select.pageUp", &["pageUp"], "Selection page up"),
    ("tui.select.pageDown", &["pageDown"], "Selection page down"),
    ("tui.select.confirm", &["enter"], "Confirm selection"),
    (
        "tui.select.cancel",
        &["escape", "ctrl+c"],
        "Cancel selection",
    ),
    (
        "tui.altScreen.pageUp",
        &["pageUp"],
        "Scroll viewport up one page",
    ),
    (
        "tui.altScreen.pageDown",
        &["pageDown"],
        "Scroll viewport down one page",
    ),
    (
        "tui.altScreen.halfPageUp",
        &[],
        "Scroll viewport up half a page",
    ),
    (
        "tui.altScreen.halfPageDown",
        &[],
        "Scroll viewport down half a page",
    ),
    ("tui.altScreen.lineUp", &[], "Scroll viewport up one line"),
    (
        "tui.altScreen.lineDown",
        &[],
        "Scroll viewport down one line",
    ),
    (
        "tui.altScreen.previousPrompt",
        &["ctrl+shift+up", "ctrl+up"],
        "Jump to previous semantic prompt",
    ),
    (
        "tui.altScreen.nextPrompt",
        &["ctrl+shift+down", "ctrl+down"],
        "Jump to next semantic prompt",
    ),
    (
        "tui.altScreen.search",
        &["ctrl+shift+f"],
        "Search the primary scroll view",
    ),
    (
        "tui.altScreen.searchNext",
        &["enter", "ctrl+g"],
        "Select the next search match",
    ),
    (
        "tui.altScreen.searchPrevious",
        &["shift+enter", "ctrl+shift+g"],
        "Select the previous search match",
    ),
    (
        "tui.altScreen.searchClose",
        &["escape"],
        "Close transcript search",
    ),
    (
        "tui.altScreen.top",
        &["ctrl+home"],
        "Scroll viewport to top",
    ),
    (
        "tui.altScreen.bottom",
        &["ctrl+end"],
        "Scroll viewport to bottom",
    ),
];

/// pi-tui's own keybindings.
pub fn tui_keybindings() -> KeybindingDefinitions {
    TUI_DEFS
        .iter()
        .map(|(id, keys, d)| {
            (
                id.to_string(),
                KeybindingDefinition {
                    default_keys: keys.iter().map(|k| k.to_string()).collect(),
                    description: Some(d.to_string()),
                },
            )
        })
        .collect()
}

fn normalize(keys: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in keys {
        if !out.contains(k) {
            out.push(k.clone());
        }
    }
    out
}

/// Resolves keybinding ids to keys, applying user overrides.
#[derive(Debug, Clone)]
pub struct KeybindingsManager {
    definitions: KeybindingDefinitions,
    user: KeybindingsConfig,
    keys: BTreeMap<String, Vec<String>>,
    conflicts: Vec<KeybindingConflict>,
}

impl KeybindingsManager {
    pub fn new(definitions: KeybindingDefinitions, user: KeybindingsConfig) -> Self {
        let mut m = KeybindingsManager {
            definitions,
            user,
            keys: BTreeMap::new(),
            conflicts: Vec::new(),
        };
        m.rebuild();
        m
    }

    fn definition(&self, id: &str) -> Option<&KeybindingDefinition> {
        self.definitions
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, d)| d)
    }

    fn rebuild(&mut self) {
        self.keys.clear();
        self.conflicts.clear();
        let mut claims: Vec<(String, Vec<String>)> = Vec::new();
        for (id, keys) in &self.user {
            if self.definition(id).is_none() {
                continue;
            }
            for k in normalize(keys.as_deref().unwrap_or(&[])) {
                match claims.iter_mut().find(|(key, _)| *key == k) {
                    Some((_, ids)) => {
                        if !ids.contains(id) {
                            ids.push(id.clone())
                        }
                    }
                    None => claims.push((k, vec![id.clone()])),
                }
            }
        }
        for (key, ids) in claims {
            if ids.len() > 1 {
                self.conflicts.push(KeybindingConflict {
                    key,
                    keybindings: ids,
                });
            }
        }
        for (id, def) in &self.definitions {
            let keys = match self.user.iter().find(|(u, _)| u == id).map(|(_, k)| k) {
                Some(Some(k)) => normalize(k),
                _ => normalize(&def.default_keys),
            };
            self.keys.insert(id.clone(), keys);
        }
    }

    /// Whether `data` is any key bound to `id`.
    pub fn matches(&self, data: &str, id: &str) -> bool {
        self.keys
            .get(id)
            .is_some_and(|ks| ks.iter().any(|k| matches_key(data, k)))
    }

    pub fn keys(&self, id: &str) -> Vec<String> {
        self.keys.get(id).cloned().unwrap_or_default()
    }

    pub fn get_definition(&self, id: &str) -> Option<KeybindingDefinition> {
        self.definition(id).cloned()
    }

    pub fn conflicts(&self) -> Vec<KeybindingConflict> {
        self.conflicts.clone()
    }

    pub fn set_user_bindings(&mut self, user: KeybindingsConfig) {
        self.user = user;
        self.rebuild();
    }

    pub fn user_bindings(&self) -> KeybindingsConfig {
        self.user.clone()
    }

    /// Every id with its resolved keys.
    pub fn resolved_bindings(&self) -> Vec<(String, Vec<String>)> {
        self.definitions
            .iter()
            .map(|(id, _)| (id.clone(), self.keys(id)))
            .collect()
    }
}

thread_local! {
    static GLOBAL: RefCell<Option<Rc<KeybindingsManager>>> = const { RefCell::new(None) };
}

/// Replace the global keybindings.
pub fn set_keybindings(kb: KeybindingsManager) {
    GLOBAL.with(|g| *g.borrow_mut() = Some(Rc::new(kb)));
}

/// The global keybindings (pi-tui's defaults unless set).
pub fn get_keybindings() -> Rc<KeybindingsManager> {
    GLOBAL.with(|g| {
        g.borrow_mut()
            .get_or_insert_with(|| {
                Rc::new(KeybindingsManager::new(
                    tui_keybindings(),
                    KeybindingsConfig::new(),
                ))
            })
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn user(pairs: &[(&str, &[&str])]) -> KeybindingsConfig {
        pairs
            .iter()
            .map(|(id, k)| (id.to_string(), Some(s(k))))
            .collect()
    }

    #[test]
    fn ctrl_j_is_a_default_newline_alias() {
        let kb = KeybindingsManager::new(tui_keybindings(), vec![]);
        assert_eq!(kb.keys("tui.input.newLine"), s(&["shift+enter", "ctrl+j"]));
        assert!(kb.matches("\n", "tui.input.newLine"));
        assert!(kb.matches("\x1b[106;5u", "tui.input.newLine"));
    }

    #[test]
    fn editor_viewport_navigation() {
        let kb = KeybindingsManager::new(tui_keybindings(), vec![]);
        assert_eq!(
            kb.keys("tui.editor.cursorLineStart"),
            s(&["home", "ctrl+a"])
        );
        assert_eq!(kb.keys("tui.editor.cursorLineEnd"), s(&["end", "ctrl+e"]));
        assert_eq!(kb.keys("tui.editor.pageUp"), s(&["pageUp", "ctrl+pageUp"]));
        assert_eq!(
            kb.keys("tui.editor.pageDown"),
            s(&["pageDown", "ctrl+pageDown"])
        );
        assert!(kb.keys("tui.editor.historyPrevious").is_empty());
        assert!(kb.keys("tui.editor.historyNext").is_empty());
    }

    #[test]
    fn alt_screen_navigation() {
        let kb = KeybindingsManager::new(tui_keybindings(), vec![]);
        assert_eq!(kb.keys("tui.altScreen.pageUp"), s(&["pageUp"]));
        assert!(kb.keys("tui.altScreen.halfPageUp").is_empty());
        assert_eq!(
            kb.keys("tui.altScreen.previousPrompt"),
            s(&["ctrl+shift+up", "ctrl+up"])
        );
        assert_eq!(
            kb.keys("tui.altScreen.searchPrevious"),
            s(&["shift+enter", "ctrl+shift+g"])
        );
        assert_eq!(kb.keys("tui.altScreen.bottom"), s(&["ctrl+end"]));
    }

    #[test]
    fn rebinding_does_not_evict_other_actions() {
        let kb = KeybindingsManager::new(
            tui_keybindings(),
            user(&[("tui.input.submit", &["enter", "ctrl+enter"])]),
        );
        assert_eq!(kb.keys("tui.input.submit"), s(&["enter", "ctrl+enter"]));
        assert_eq!(kb.keys("tui.select.confirm"), s(&["enter"]));
        let kb = KeybindingsManager::new(
            tui_keybindings(),
            user(&[("tui.select.up", &["up", "ctrl+p"])]),
        );
        assert_eq!(kb.keys("tui.editor.cursorUp"), s(&["up"]));
    }

    #[test]
    fn reports_conflicts_without_evicting_defaults() {
        let kb = KeybindingsManager::new(
            tui_keybindings(),
            user(&[
                ("tui.input.submit", &["ctrl+x"]),
                ("tui.select.confirm", &["ctrl+x"]),
            ]),
        );
        assert_eq!(
            kb.conflicts(),
            vec![KeybindingConflict {
                key: "ctrl+x".into(),
                keybindings: s(&["tui.input.submit", "tui.select.confirm"])
            }]
        );
        assert_eq!(kb.keys("tui.editor.cursorLeft"), s(&["left", "ctrl+b"]));
    }
}
