//! Golden parity with upstream `utils.ts`: fixtures come from running the TypeScript
//! (`parity/gen-utils-fixtures.mjs`).

use pi_tui::utils::*;
use serde_json::Value;

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

#[test]
fn utils_match_upstream() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/utils.json")).unwrap();
    let mut failures = Vec::new();
    let mut check = |what: String, got: String, want: String| {
        if got != want {
            failures.push(format!("{what}\n  got:  {got:?}\n  want: {want:?}"));
        }
    };
    for c in &cases {
        let text = s(&c["s"]);
        check(format!("visible_width({text:?})"), visible_width(text).to_string(), c["vw"].to_string());
        check(format!("strip({text:?})"), strip_terminal_sequences(text), s(&c["strip"]).to_string());
        check(format!("normalize({text:?})"), normalize_terminal_output(text), s(&c["norm"]).to_string());
        check(format!("bg({text:?})"), active_background_ansi(text), s(&c["bg"]).to_string());
        for (w, want) in c["wrap"].as_object().unwrap() {
            let w: usize = w.parse().unwrap();
            let want: Vec<String> = want.as_array().unwrap().iter().map(|v| s(v).to_string()).collect();
            check(format!("wrap({text:?}, {w})"), format!("{:?}", wrap_text_with_ansi(text, w)), format!("{want:?}"));
        }
        for (w, want) in c["trunc"].as_object().unwrap() {
            let w: usize = w.parse().unwrap();
            check(format!("trunc({text:?}, {w})"), truncate_to_width(text, w, "...", false), s(&want[0]).to_string());
            check(format!("trunc_pad({text:?}, {w})"), truncate_to_width(text, w, "\u{2026}", true), s(&want[1]).to_string());
            check(format!("trunc_empty({text:?}, {w})"), truncate_to_width(text, w, "", false), s(&want[2]).to_string());
        }
        for sl in c["slice"].as_array().unwrap() {
            let (a, l, strict) = (n(&sl[0]), n(&sl[1]), sl[2].as_bool().unwrap());
            let (t, w) = slice_with_width(text, a, l, strict);
            check(format!("slice({text:?}, {a}, {l}, {strict})"), format!("{t:?} {w}"), format!("{:?} {}", s(&sl[3]["text"]), sl[3]["width"]));
        }
        for sg in c["seg"].as_array().unwrap() {
            let (b, a, l, strict) = (n(&sg[0]), n(&sg[1]), n(&sg[2]), sg[3].as_bool().unwrap());
            let r = extract_segments(text, b, a, l, strict);
            let o = &sg[4];
            check(
                format!("segments({text:?}, {b}, {a}, {l}, {strict})"),
                format!("{:?} {} {:?} {}", r.before, r.before_width, r.after, r.after_width),
                format!("{:?} {} {:?} {}", s(&o["before"]), o["beforeWidth"], s(&o["after"]), o["afterWidth"]),
            );
        }
        for cl in c["cell"].as_array().unwrap() {
            let col = n(&cl[0]);
            let want_range = if cl[1].is_null() { "None".to_string() } else { format!("Some(({}, {}))", cl[1]["start"], cl[1]["end"]) };
            check(format!("cell_range({text:?}, {col})"), format!("{:?}", grapheme_cell_range(text, col)), want_range);
            let want_link = if cl[2].is_null() { "None".to_string() } else { format!("Some({:?})", s(&cl[2])) };
            check(format!("osc8_at({text:?}, {col})"), format!("{:?}", osc8_link_at_column(text, col)), want_link);
        }
    }
    let total = failures.len();
    failures.truncate(25);
    assert!(total == 0, "{total} mismatches; first ones:\n{}", failures.join("\n"));
}
