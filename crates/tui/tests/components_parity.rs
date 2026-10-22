//! Golden parity for components against the TypeScript (`parity/gen-components-fixtures.mjs`).

use pi_tui::components::loader::{Loader, LoaderIndicatorOptions};
use pi_tui::components::spacer::Spacer;
use pi_tui::components::text::Text;
use pi_tui::components::truncated_text::TruncatedText;
use pi_tui::tui::Component;
use serde_json::Value;
use std::rc::Rc;
use std::sync::OnceLock;

const WIDTHS: &[usize] = &[1, 2, 3, 5, 8, 13, 21, 40, 80];

fn fixtures() -> &'static Value {
    static F: OnceLock<Value> = OnceLock::new();
    F.get_or_init(|| serde_json::from_str(include_str!("fixtures/components.json")).unwrap())
}

fn lines(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect()
}

fn check(failures: &mut Vec<String>, what: String, got: Vec<String>, want: Vec<String>) {
    if got != want {
        failures.push(format!("{what}\n  got:  {got:?}\n  want: {want:?}"));
    }
}

fn finish(failures: Vec<String>) {
    let n = failures.len();
    assert!(n == 0, "{n} mismatches; first:\n{}", failures.into_iter().take(8).collect::<Vec<_>>().join("\n"));
}

#[test]
fn text_matches_upstream() {
    let mut f = Vec::new();
    for c in fixtures()["text"].as_array().unwrap() {
        let t = c["t"].as_str().unwrap();
        let (px, py) = (c["px"].as_u64().unwrap() as usize, c["py"].as_u64().unwrap() as usize);
        let mut comp = if c["bg"].as_bool().unwrap() {
            Text::with_bg(t, px, py, Rc::new(|s: &str| format!("\x1b[48;5;236m{s}\x1b[49m")))
        } else {
            Text::new(t, px, py)
        };
        for (i, w) in WIDTHS.iter().enumerate() {
            check(&mut f, format!("Text({t:?}, {px}, {py}).render({w})"), comp.render(*w), lines(&c["r"][i]));
        }
    }
    finish(f);
}

#[test]
fn truncated_text_matches_upstream() {
    let mut f = Vec::new();
    for c in fixtures()["truncated"].as_array().unwrap() {
        let t = c["t"].as_str().unwrap();
        let (px, py) = (c["px"].as_u64().unwrap() as usize, c["py"].as_u64().unwrap() as usize);
        let mut comp = TruncatedText::new(t, px, py);
        for (i, w) in WIDTHS.iter().enumerate() {
            check(&mut f, format!("TruncatedText({t:?}, {px}, {py}).render({w})"), comp.render(*w), lines(&c["r"][i]));
        }
    }
    finish(f);
}

#[test]
fn spacer_matches_upstream() {
    for c in fixtures()["spacer"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        assert_eq!(Spacer::new(n).render(10), lines(&c["r"]));
    }
}

#[test]
fn loader_matches_upstream() {
    let mut f = Vec::new();
    for c in fixtures()["loader"].as_array().unwrap() {
        let msg = c["msg"].as_str().unwrap();
        let ind = c["ind"].as_object().map(|o| LoaderIndicatorOptions { frames: o.get("frames").map(lines), interval_ms: None });
        let mut l = Loader::new(
            Rc::new(|s: &str| format!("\x1b[36m{s}\x1b[39m")),
            Rc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m")),
            msg,
            ind,
        );
        l.tick(0);
        for (n, frame) in c["frames"].as_array().unwrap().iter().enumerate() {
            for (i, w) in WIDTHS.iter().enumerate() {
                check(&mut f, format!("Loader({msg:?}) frame {n} render({w})"), l.render(*w), lines(&frame[i]));
            }
            l.tick(80 * (n as u64 + 1));
        }
    }
    finish(f);
}

fn style(f: fn(&str) -> String) -> pi_tui::components::select_list::Style {
    Rc::new(move |s: &str| f(s))
}

#[test]
fn fuzzy_matches_upstream() {
    use pi_tui::fuzzy::{fuzzy_filter, fuzzy_match};
    let names = ["model", "settings", "session", "new", "resume", "Model-Picker", "a/b/c.ts", "src/utils.rs", "x1", "abc12", "12abc", "日本", "export_html", "tree", "fork", "compact"];
    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    for c in fixtures()["fuzzy"].as_array().unwrap() {
        let q = c["q"].as_str().unwrap();
        for (i, n) in names.iter().enumerate() {
            let m = fuzzy_match(q, n);
            let want = &c["m"][i];
            assert_eq!(m.matches, want["matches"].as_bool().unwrap(), "fuzzy_match({q:?}, {n:?})");
            assert!((m.score - want["score"].as_f64().unwrap()).abs() < 1e-9, "fuzzy_match({q:?}, {n:?}) score {} vs {}", m.score, want["score"]);
        }
        assert_eq!(fuzzy_filter(&names, q, |s| s.clone()), lines(&c["f"]), "fuzzy_filter({q:?})");
    }
}

#[test]
fn select_list_matches_upstream() {
    use pi_tui::components::select_list::*;
    use std::cell::RefCell;
    let theme = SelectListTheme {
        selected_prefix: style(|s| format!("<P>{s}")),
        selected_text: style(|s| format!("\x1b[1m{s}\x1b[22m")),
        description: style(|s| format!("\x1b[2m{s}\x1b[22m")),
        scroll_info: style(|s| format!("\x1b[90m{s}\x1b[39m")),
        no_match: style(|s| format!("!{s}")),
    };
    let mut f = Vec::new();
    for (n, c) in fixtures()["select"].as_array().unwrap().iter().enumerate() {
        let items: Vec<SelectItem> = c["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| SelectItem::new(i["value"].as_str().unwrap(), i["label"].as_str().unwrap(), i["description"].as_str()))
            .collect();
        let layout = SelectListLayoutOptions {
            min_primary_column_width: c["layout"]["minPrimaryColumnWidth"].as_u64().map(|v| v as usize),
            max_primary_column_width: c["layout"]["maxPrimaryColumnWidth"].as_u64().map(|v| v as usize),
            truncate_primary: None,
        };
        let mut list = SelectList::new(items, c["maxVisible"].as_u64().unwrap() as usize, theme.clone(), layout);
        let events: Rc<RefCell<Vec<Value>>> = Rc::default();
        let e1 = events.clone();
        list.on_select = Some(Box::new(move |i| e1.borrow_mut().push(serde_json::json!(["select", i.value]))));
        let e2 = events.clone();
        list.on_cancel = Some(Box::new(move || e2.borrow_mut().push(serde_json::json!(["cancel"]))));
        let e3 = events.clone();
        list.on_selection_change = Some(Box::new(move |i| e3.borrow_mut().push(serde_json::json!(["change", i.value]))));
        for (s, step) in c["steps"].as_array().unwrap().iter().enumerate() {
            if let Some(filter) = step["filter"].as_str() {
                list.set_filter(filter);
            }
            if let Some(input) = step["input"].as_str() {
                list.handle_input(input);
            }
            for (i, w) in [20, 41, 60, 100].iter().enumerate() {
                check(&mut f, format!("select {n} step {s} render({w})"), list.render(*w), lines(&step["r"][i]));
            }
            let got = Value::Array(events.borrow_mut().drain(..).collect());
            if got != step["ev"] {
                f.push(format!("select {n} step {s} events {got} vs {}", step["ev"]));
            }
        }
    }
    finish(f);
}
