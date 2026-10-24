//! Golden parity with upstream `tui-main-screen.ts`: scripted frames (resizes, growth, shrink,
//! edits, cursor markers, fake cursors, forced redraws) rendered by the TypeScript renderer
//! (`parity/gen-render-fixtures.mjs`); the bytes written to the terminal must match exactly.

use pi_tui::terminal::{Terminal, TerminalEvent};
use pi_tui::tui::{Component, OverlayAnchor, OverlayId, OverlayMargin, OverlayOptions, SizeValue};
use pi_tui::tui_main_screen::TuiMainScreen;
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Rec {
    cols: usize,
    rows: usize,
    out: Rc<RefCell<String>>,
}

impl Terminal for Rec {
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn poll(&mut self, _: Option<u64>) -> Vec<TerminalEvent> {
        Vec::new()
    }
    fn write(&mut self, d: &str) {
        self.out.borrow_mut().push_str(d);
    }
    fn columns(&self) -> usize {
        self.cols
    }
    fn rows(&self) -> usize {
        self.rows
    }
}

struct Lines(Vec<String>);

impl Component for Lines {
    fn render(&mut self, _: usize) -> Vec<String> {
        self.0.clone()
    }
}

fn size(v: &Value) -> Option<SizeValue> {
    match v {
        Value::Number(n) => Some(SizeValue::Abs(n.as_i64().unwrap())),
        Value::String(s) => Some(SizeValue::Percent(s.trim_end_matches('%').parse().unwrap())),
        _ => None,
    }
}

fn options(o: &Value) -> OverlayOptions {
    let anchor = o["anchor"].as_str().map(|a| match a {
        "center" => OverlayAnchor::Center,
        "top-left" => OverlayAnchor::TopLeft,
        "top-right" => OverlayAnchor::TopRight,
        "bottom-left" => OverlayAnchor::BottomLeft,
        "bottom-right" => OverlayAnchor::BottomRight,
        "top-center" => OverlayAnchor::TopCenter,
        "bottom-center" => OverlayAnchor::BottomCenter,
        "left-center" => OverlayAnchor::LeftCenter,
        _ => OverlayAnchor::RightCenter,
    });
    let margin = match &o["margin"] {
        Value::Number(n) => Some(OverlayMargin::all(n.as_i64().unwrap())),
        Value::Object(_) => Some(OverlayMargin {
            top: o["margin"]["top"].as_i64().unwrap_or(0),
            right: o["margin"]["right"].as_i64().unwrap_or(0),
            bottom: o["margin"]["bottom"].as_i64().unwrap_or(0),
            left: o["margin"]["left"].as_i64().unwrap_or(0),
        }),
        _ => None,
    };
    OverlayOptions {
        width: size(&o["width"]),
        min_width: o["minWidth"].as_i64(),
        max_height: size(&o["maxHeight"]),
        anchor,
        offset_x: o["offsetX"].as_i64(),
        offset_y: o["offsetY"].as_i64(),
        row: size(&o["row"]),
        col: size(&o["col"]),
        margin,
        visible: None,
        non_capturing: o["nonCapturing"].as_bool().unwrap_or(false),
    }
}

#[test]
fn main_screen_matches_upstream() {
    let scenarios: Vec<Value> = serde_json::from_str(include_str!("fixtures/render.json")).unwrap();
    let mut failures = Vec::new();
    for (n, sc) in scenarios.iter().enumerate() {
        let out = Rc::new(RefCell::new(String::new()));
        let frames = sc["frames"].as_array().unwrap();
        let term = Rec {
            cols: frames[0]["cols"].as_u64().unwrap() as usize,
            rows: frames[0]["rows"].as_u64().unwrap() as usize,
            out: out.clone(),
        };
        let mut tui = TuiMainScreen::with_hardware_cursor(term, sc["hw"].as_bool().unwrap());
        tui.set_clear_on_shrink(sc["clearOnShrink"].as_bool().unwrap());
        let comp = Rc::new(RefCell::new(Lines(Vec::new())));
        tui.add_child(comp.clone());
        tui.start();
        out.borrow_mut().clear();
        let mut overlays: Vec<OverlayId> = Vec::new();
        for (f, fr) in frames.iter().enumerate() {
            tui.terminal.cols = fr["cols"].as_u64().unwrap() as usize;
            tui.terminal.rows = fr["rows"].as_u64().unwrap() as usize;
            comp.borrow_mut().0 = fr["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l.as_str().unwrap().to_string())
                .collect();
            for op in fr["ops"].as_array().unwrap() {
                match op["op"].as_str().unwrap() {
                    "show" => {
                        let lines = op["lines"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|l| l.as_str().unwrap().to_string())
                            .collect();
                        let c = Rc::new(RefCell::new(Lines(lines)));
                        overlays.push(tui.show_overlay(c, Some(options(&op["opts"]))));
                    }
                    kind => {
                        let i = op["index"].as_u64().unwrap() as usize;
                        let id = overlays[i];
                        match kind {
                            "hide" => {
                                tui.hide_overlay_id(id);
                                overlays.remove(i);
                            }
                            "toggle" => {
                                let h = tui.is_overlay_hidden(id);
                                tui.set_overlay_hidden(id, !h);
                            }
                            _ => tui.focus_overlay(id),
                        }
                    }
                }
            }
            tui.render_now(fr["force"].as_bool().unwrap(), 0);
            let got = std::mem::take(&mut *out.borrow_mut());
            let want = fr["out"].as_str().unwrap();
            if got != want {
                failures.push(format!(
                    "scenario {n} frame {f} ({}x{}, lines {:?})\n  got:  {got:?}\n  want: {want:?}",
                    fr["cols"], fr["rows"], fr["lines"]
                ));
                break;
            }
        }
        tui.stop(false);
        let got = std::mem::take(&mut *out.borrow_mut());
        if failures.is_empty() && got != sc["stopOut"].as_str().unwrap() {
            failures.push(format!(
                "scenario {n} stop\n  got:  {got:?}\n  want: {:?}",
                sc["stopOut"]
            ));
        }
        if tui.full_redraws() != sc["fullRedraws"].as_u64().unwrap() {
            failures.push(format!(
                "scenario {n}: full redraws {} vs {}",
                tui.full_redraws(),
                sc["fullRedraws"]
            ));
        }
    }
    let total = failures.len();
    failures.truncate(5);
    assert!(
        total == 0,
        "{total} of {} scenarios differ:\n{}",
        scenarios.len(),
        failures.join("\n")
    );
}
