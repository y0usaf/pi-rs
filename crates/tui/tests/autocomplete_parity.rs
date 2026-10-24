//! Golden parity for autocomplete over a fixture tree (`parity/gen-autocomplete-fixtures.mjs`).
//! Needs `fd` on PATH for `@` fuzzy file suggestions, as upstream does.

use pi_tui::autocomplete::*;
use serde_json::{Value, json};
use std::path::Path;
use std::rc::Rc;

fn build_tree(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
    let files = [
        "src/main.rs",
        "src/lib.rs",
        "src/utils/mod.rs",
        "src/utils/width.rs",
        "README.md",
        "readme-old.txt",
        "Cargo.toml",
        "docs/guide.md",
        "docs/API.md",
        "my file.txt",
        "日本語/ファイル.md",
        ".hidden/x",
        "app/[slug]/page.tsx",
        "_under.md",
        "1-first.md",
        "b.md",
        "B2.md",
    ];
    for f in files {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }
    std::fs::create_dir_all(root.join("emptydir")).unwrap();
    std::os::unix::fs::symlink("docs", root.join("docs-link")).unwrap();
    std::fs::create_dir_all(root.join("home/Dev")).unwrap();
    std::fs::write(root.join("home/notes.txt"), "x").unwrap();
}

fn item_json(i: &AutocompleteItem) -> Value {
    let mut v = json!({ "value": i.value, "label": i.label });
    if let Some(d) = &i.description {
        v["description"] = json!(d);
    }
    v
}

#[test]
fn autocomplete_matches_upstream() {
    if std::process::Command::new("fd")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("fd not found; skipping");
        return;
    }
    let outer = std::env::temp_dir().join(format!("pi-tui-ac-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outer);
    std::fs::create_dir_all(outer.join("sibling")).unwrap();
    std::fs::write(outer.join("top.txt"), "x").unwrap();
    let root = outer.join("work");
    build_tree(&root);
    let items = |p: &str| -> Option<Vec<AutocompleteItem>> {
        Some(
            ["gpt", "claude", "gemini"]
                .iter()
                .filter(|x| x.starts_with(p))
                .map(|x| AutocompleteItem {
                    value: x.to_string(),
                    label: x.to_string(),
                    description: None,
                })
                .collect(),
        )
    };
    let commands = vec![
        CommandEntry::Command(SlashCommand {
            name: "model".into(),
            description: Some("Select model".into()),
            argument_hint: Some("<name>".into()),
            argument_completions: Some(Rc::new(items)),
        }),
        CommandEntry::Command(SlashCommand {
            name: "settings".into(),
            description: Some("Open settings".into()),
            ..Default::default()
        }),
        CommandEntry::Command(SlashCommand {
            name: "skill:review".into(),
            description: Some("Review code".into()),
            ..Default::default()
        }),
        CommandEntry::Command(SlashCommand {
            name: "skill:model-tuning".into(),
            ..Default::default()
        }),
        CommandEntry::Item(AutocompleteItem {
            value: "export".into(),
            label: "export".into(),
            description: Some("Export session".into()),
        }),
        CommandEntry::Command(SlashCommand {
            name: "new".into(),
            argument_hint: Some("<title>".into()),
            ..Default::default()
        }),
    ];
    let p = CombinedAutocompleteProvider::new(commands, root.to_str().unwrap(), Some("fd"))
        .with_home(root.join("home").to_str().unwrap());
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/autocomplete.json")).unwrap();
    let mut f = Vec::new();
    for c in &cases {
        let t = c["t"].as_str().unwrap();
        let force = c["force"].as_bool().unwrap();
        let lines = vec![t.to_string()];
        let s = p.get_suggestions(&lines, 0, t.len(), force);
        let got_s = s.as_ref().map_or(Value::Null, |s| json!({ "items": s.items.iter().map(item_json).collect::<Vec<_>>(), "prefix": s.prefix }));
        let applied: Vec<Value> = s.as_ref().map_or(vec![], |s| {
            s.items
                .iter()
                .take(3)
                .map(|it| {
                    let r =
                        p.apply_completion(&[format!("{t}\x22tail")], 0, t.len(), it, &s.prefix);
                    let col16 = r.lines[0][..r.cursor_col].encode_utf16().count();
                    json!({ "lines": r.lines, "cursorLine": r.cursor_line, "cursorCol": col16 })
                })
                .collect()
        });
        let got = json!({ "s": got_s, "applied": applied, "trig": p.should_trigger_file_completion(&lines, 0, t.len()) });
        let want = json!({ "s": c["s"], "applied": c["applied"], "trig": c["trig"] });
        if got != want {
            f.push(format!(
                "{t:?} force={force}\n  got:  {got}\n  want: {want}"
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&outer);
    assert!(
        f.is_empty(),
        "{} of {} cases differ:\n{}",
        f.len(),
        cases.len(),
        f.into_iter().take(6).collect::<Vec<_>>().join("\n")
    );
}
