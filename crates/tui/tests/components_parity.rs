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
