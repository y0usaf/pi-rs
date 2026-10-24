//! Golden parity for Editor: random key scripts run through the TypeScript Editor
//! (`parity/gen-editor-fixtures.mjs`) with a small in-memory provider mirrored here; renders,
//! lines, cursors, callbacks and autocomplete visibility must match.

use pi_tui::autocomplete::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, Completion,
};
use pi_tui::components::editor::{Editor, EditorOptions, EditorTheme};
use pi_tui::components::select_list::SelectListTheme;
use pi_tui::tui::Component;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

mod common;

const COMMANDS: [&str; 6] = ["help", "model", "mode", "clear", "compact", "settings"];
const WORDS: [&str; 7] = [
    "apple",
    "apricot",
    "banana",
    "日本語",
    "cherry",
    "src/main.rs",
    "src/lib.rs",
];

struct TestProvider;

impl AutocompleteProvider for TestProvider {
    fn trigger_characters(&self) -> Option<Vec<String>> {
        Some(vec!["$".into()])
    }

    fn get_suggestions(
        &self,
        lines: &[String],
        cl: usize,
        cc: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions> {
        let line = lines.get(cl).map_or("", String::as_str);
        let before = &line[..cc.min(line.len())];
        if let Some(rest) = before.strip_prefix('/')
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            let items: Vec<AutocompleteItem> = COMMANDS
                .iter()
                .filter(|c| c.starts_with(rest))
                .map(|c| AutocompleteItem {
                    value: c.to_string(),
                    label: c.to_string(),
                    description: Some(format!("run {c}")),
                })
                .collect();
            return (!items.is_empty()).then(|| AutocompleteSuggestions {
                items,
                prefix: before.to_string(),
            });
        }
        let start = before
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let token = &before[start..];
        let (trig, word) = match token.chars().next() {
            Some(c @ ('@' | '\x23' | '$')) => (&token[..1], &token[c.len_utf8()..]),
            _ => ("", token),
        };
        if trig.is_empty() && !force {
            return None;
        }
        let items: Vec<AutocompleteItem> = WORDS
            .iter()
            .filter(|w| w.starts_with(word))
            .map(|w| AutocompleteItem {
                value: format!("{trig}{w}"),
                label: w.to_string(),
                description: None,
            })
            .collect();
        (!items.is_empty()).then(|| AutocompleteSuggestions {
            items,
            prefix: token.to_string(),
        })
    }

    fn apply_completion(
        &self,
        lines: &[String],
        cl: usize,
        cc: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> Completion {
        let line = &lines[cl];
        let start = cc.saturating_sub(prefix.len());
        let value = if prefix.starts_with('/') {
            format!("/{} ", item.value)
        } else {
            item.value.clone()
        };
        let mut out = lines.to_vec();
        out[cl] = format!("{}{value}{}", &line[..start], &line[cc..]);
        Completion {
            lines: out,
            cursor_line: cl,
            cursor_col: start + value.len(),
        }
    }

    fn should_trigger_file_completion(&self, lines: &[String], cl: usize, cc: usize) -> bool {
        let line = &lines[cl];
        let b = line[..cc.min(line.len())].trim();
        !(b.starts_with('/') && !b.contains(' '))
    }
}

fn style(f: impl Fn(&str) -> String + 'static) -> Rc<dyn Fn(&str) -> String> {
    Rc::new(f)
}

fn theme() -> EditorTheme {
    EditorTheme {
        border_color: style(|s| format!("<B>{s}</B>")),
        select_list: SelectListTheme {
            selected_prefix: style(|s| format!("<P>{s}")),
            selected_text: style(|s| format!("\x1b[1m{s}\x1b[22m")),
            description: style(|s| format!("\x1b[2m{s}\x1b[22m")),
            scroll_info: style(|s| format!("\x1b[90m{s}\x1b[39m")),
            no_match: style(|s| format!("!{s}")),
        },
    }
}

#[test]
fn editor_matches_upstream() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/editor.json")).unwrap();
    let mut f = Vec::new();
    let mut diverged = 0usize;
    for (n, c) in cases.iter().enumerate() {
        let o = &c["opts"];
        let opts = EditorOptions {
            padding_x: o["paddingX"].as_u64().map(|v| v as usize),
            autocomplete_max_visible: o["autocompleteMaxVisible"].as_u64().map(|v| v as usize),
        };
        let mut ed = Editor::new(theme(), opts);
        ed.terminal_rows = c["rows"].as_u64().unwrap() as usize;
        ed.focused = c["focused"].as_bool().unwrap();
        if c["provider"].as_bool().unwrap() {
            ed.set_autocomplete_provider(Rc::new(TestProvider));
        }
        for h in c["history"].as_array().unwrap() {
            ed.add_to_history(h.as_str().unwrap());
        }
        let events: Rc<RefCell<Vec<Value>>> = Rc::default();
        let e1 = events.clone();
        ed.on_submit = Some(Box::new(move |v| {
            e1.borrow_mut().push(json!(["submit", v]))
        }));
        let e2 = events.clone();
        ed.on_change = Some(Box::new(move |v| {
            e2.borrow_mut().push(json!(["change", v]))
        }));
        for (s, st) in c["steps"].as_array().unwrap().iter().enumerate() {
            let d = st["d"].as_str().unwrap();
            let lines = ed.get_lines();
            let segs = st["segs"].as_array().unwrap();
            if segs.len() != lines.len()
                || !lines
                    .iter()
                    .zip(segs)
                    .all(|(l, v)| common::segments_agree(l, v))
            {
                diverged += 1;
                break;
            }
            ed.handle_input(d);
            // Upstream's onSubmit also records history.
            let submitted: Vec<String> = events
                .borrow()
                .iter()
                .filter(|e| e[0] == "submit")
                .map(|e| e[1].as_str().unwrap().to_string())
                .collect();
            for v in submitted {
                ed.add_to_history(&v);
            }
            let w = st["w"].as_u64().unwrap() as usize;
            let r = ed.render(w);
            let want_lines: Vec<String> = st["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l.as_str().unwrap().to_string())
                .collect();
            let cl = st["cl"].as_u64().unwrap() as usize;
            let cc = want_lines.get(cl).map_or(0, |l| {
                common::byte_at(l, st["cc"].as_u64().unwrap() as usize)
            });
            let (gl, gc) = ed.get_cursor();
            let got = json!({ "r": r, "lines": ed.get_lines(), "cl": gl, "cc": gc, "ev": events.borrow_mut().drain(..).collect::<Vec<_>>(), "ac": ed.is_showing_autocomplete() });
            let want = json!({ "r": st["r"], "lines": want_lines, "cl": cl, "cc": cc, "ev": st["ev"], "ac": st["ac"] });
            if got != want {
                f.push(format!(
                    "case {n} step {s} input {d:?} width {w}\n  got:  {got}\n  want: {want}"
                ));
                break;
            }
        }
    }
    assert!(
        diverged <= 15,
        "{diverged} scripts hit segmentation differences"
    );
    let total = f.len();
    assert!(
        total == 0,
        "{total} of {} cases differ:\n{}",
        cases.len(),
        f.into_iter().take(5).collect::<Vec<_>>().join("\n")
    );
}
