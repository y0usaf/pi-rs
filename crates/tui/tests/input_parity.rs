//! Golden parity for Input: random key scripts run through the TypeScript Input
//! (`parity/gen-input-fixtures.mjs`); renders, values, cursors and callbacks must match.

use pi_tui::components::input::{Input, InputOptions};
use pi_tui::tui::Component;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

mod common;

fn byte_at(t: &str, u16_off: usize) -> usize {
    let mut units = 0;
    for (i, c) in t.char_indices() {
        if units >= u16_off {
            return i;
        }
        units += c.len_utf16();
    }
    t.len()
}

#[test]
fn input_matches_upstream() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/input.json")).unwrap();
    let mut f = Vec::new();
    let mut diverged = 0usize;
    for (n, c) in cases.iter().enumerate() {
        let o = &c["opts"];
        let mut opts = InputOptions {
            prompt: o["prompt"].as_str().map(Into::into),
            placeholder: o["placeholder"].as_str().map(Into::into),
            placeholder_style: None,
        };
        if o["styled"].as_bool().unwrap() {
            opts.placeholder_style = Some(Rc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m")));
        }
        let mut input = Input::new(opts);
        input.focused = c["focused"].as_bool().unwrap();
        let events: Rc<RefCell<Vec<Value>>> = Rc::default();
        let e1 = events.clone();
        input.on_submit = Some(Box::new(move |v| {
            e1.borrow_mut().push(json!(["submit", v]))
        }));
        let e2 = events.clone();
        input.on_escape = Some(Box::new(move || e2.borrow_mut().push(json!(["escape"]))));
        for (s, st) in c["steps"].as_array().unwrap().iter().enumerate() {
            let d = st["d"].as_str().unwrap();
            if !common::segments_agree(input.value(), &st["segs"]) {
                diverged += 1;
                break;
            }
            input.handle_input(d);
            let w = st["w"].as_u64().unwrap() as usize;
            let r = input.render(w);
            let v = st["v"].as_str().unwrap();
            let want_cursor = byte_at(v, st["c"].as_u64().unwrap() as usize);
            let got = json!({ "r": r, "v": input.value(), "c": input.cursor(), "ev": events.borrow_mut().drain(..).collect::<Vec<_>>() });
            let want = json!({ "r": st["r"], "v": v, "c": want_cursor, "ev": st["ev"] });
            if got != want {
                f.push(format!(
                    "case {n} step {s} input {d:?} width {w}\n  got:  {got}\n  want: {want}"
                ));
                break;
            }
        }
    }
    // Known: V8 splits a repeated ideograph ("日日") into two words, ICU4X keeps one.
    assert!(
        diverged <= 10,
        "{diverged} scripts hit segmentation differences"
    );
    let total = f.len();
    assert!(
        total == 0,
        "{total} of {} cases differ:\n{}",
        cases.len(),
        f.into_iter().take(6).collect::<Vec<_>>().join("\n")
    );
}
