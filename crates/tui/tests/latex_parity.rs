//! Golden parity for render_latex against upstream latex.ts (fixtures from parity/gen-latex-fixtures.mjs:
//! upstream test sources, every known command in several shapes, environments, random token soups;
//! each rendered inline and display).

use pi_tui::latex::render_latex;
use serde_json::Value;

#[test]
fn latex_matches_upstream() {
    let f: Value = serde_json::from_str(include_str!("fixtures/latex.json")).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for c in f["cases"].as_array().unwrap() {
        let source = c["s"].as_str().unwrap();
        for (key, display) in [("i", false), ("d", true)] {
            let want = c[key].as_str();
            let got = render_latex(source, display);
            checked += 1;
            if got.as_deref() != want {
                failures.push(format!(
                    "{source:?} display={display}\n  got:  {got:?}\n  want: {want:?}"
                ));
            }
        }
    }
    eprintln!("checked {checked} renders");
    if !failures.is_empty() {
        let n = failures.len();
        let shown: Vec<_> = failures.into_iter().take(12).collect();
        panic!("{n} renders differ:\n{}", shown.join("\n"));
    }
}
