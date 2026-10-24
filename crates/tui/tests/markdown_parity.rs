//! Golden parity for Markdown against upstream (fixtures from parity/gen-markdown-fixtures.mjs).
//! Math renders through the ported latex.ts (`pi_tui::latex`), so cases with math run in every variant.

use pi_tui::components::markdown::{
    DefaultTextStyle, Markdown, MarkdownOptions, MarkdownTheme, StyleFn,
};
use pi_tui::tui::Component;
use serde_json::Value;
use std::rc::Rc;

/// chalk-like style: reopen after an inner close, close and reopen around newlines.
fn sty(open: u32, close: u32) -> StyleFn {
    Rc::new(move |s: &str| {
        let c = format!("\x1b[{close}m");
        let o = format!("\x1b[{open}m");
        let t = s.split(&c).collect::<Vec<_>>().join(&format!("{c}{o}"));
        let t = t.split('\n').collect::<Vec<_>>().join(&format!("{c}\n{o}"));
        format!("{o}{t}{c}")
    })
}

fn theme(indent: bool) -> MarkdownTheme {
    let (bold, cyan) = (sty(1, 22), sty(36, 39));
    let heading: StyleFn = {
        let (b, c) = (bold.clone(), cyan.clone());
        Rc::new(move |s: &str| b(&c(s)))
    };
    MarkdownTheme {
        heading,
        link: sty(34, 39),
        link_url: sty(2, 22),
        code: sty(33, 39),
        code_block: sty(32, 39),
        code_block_border: sty(2, 22),
        quote: sty(3, 23),
        quote_border: sty(2, 22),
        hr: sty(2, 22),
        list_bullet: cyan,
        bold,
        italic: sty(3, 23),
        strikethrough: sty(9, 29),
        underline: sty(4, 24),
        highlight_code: None,
        code_block_indent: indent.then(|| "> ".to_string()),
    }
}

fn variant(v: u64, text: &str) -> Markdown {
    let gray = DefaultTextStyle {
        color: Some(sty(90, 39)),
        italic: true,
        ..Default::default()
    };
    let bg = DefaultTextStyle {
        color: Some(sty(33, 39)),
        bg_color: Some(Rc::new(|s: &str| format!("\x1b[48;5;236m{s}\x1b[49m"))),
        bold: true,
        underline: true,
        strikethrough: true,
        ..Default::default()
    };
    let o = MarkdownOptions::default();
    match v {
        0 => Markdown::new(text, 0, 0, theme(false), None, o),
        1 => Markdown::new(text, 1, 1, theme(false), Some(gray), o),
        2 => Markdown::new(text, 2, 0, theme(false), Some(bg), o),
        3 => Markdown::new(
            text,
            0,
            0,
            theme(false),
            None,
            MarkdownOptions {
                hyperlinks: true,
                ..o
            },
        ),
        4 => Markdown::new(
            text,
            0,
            0,
            theme(false),
            None,
            MarkdownOptions {
                preserve_ordered_list_markers: true,
                preserve_backslash_escapes: true,
                ..o
            },
        ),
        _ => Markdown::new(
            text,
            1,
            0,
            theme(true),
            None,
            MarkdownOptions {
                render_latex: Some(false),
                ..o
            },
        ),
    }
}

const WIDTHS: &[usize] = &[1, 3, 8, 20, 40, 80];

#[test]
fn markdown_matches_upstream() {
    let f: Value = serde_json::from_str(include_str!("fixtures/markdown.json")).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (ci, c) in f["cases"].as_array().unwrap().iter().enumerate() {
        let text = c["t"].as_str().unwrap();
        for r in c["r"].as_array().unwrap() {
            let v = r["v"].as_u64().unwrap();
            let mut md = variant(v, text);
            for (wi, want) in r["out"].as_array().unwrap().iter().enumerate() {
                let want: Vec<String> = want
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| l.as_str().unwrap().to_string())
                    .collect();
                let got = md.render(WIDTHS[wi]);
                checked += 1;
                if got != want {
                    failures.push(format!(
                        "case {ci} v{v} w{} {text:?}\n  got:  {got:?}\n  want: {want:?}",
                        WIDTHS[wi]
                    ));
                }
            }
        }
    }
    eprintln!("checked {checked} renders");
    if !failures.is_empty() {
        let n = failures.len();
        let shown: Vec<_> = failures.into_iter().take(6).collect();
        panic!("{n} renders differ:\n{}", shown.join("\n"));
    }
}
