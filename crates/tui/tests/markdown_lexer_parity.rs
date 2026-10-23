//! Token trees from the Rust marked port vs upstream's configured marked (fixtures from
//! parity/gen-markdown-tokens.mjs).

use pi_tui::components::markdown::lexer::{Cell, Kind, Token, lex};
use serde_json::{Value, json};

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Space => "space",
        Kind::Code => "code",
        Kind::Heading => "heading",
        Kind::Hr => "hr",
        Kind::Blockquote => "blockquote",
        Kind::List => "list",
        Kind::ListItem => "list_item",
        Kind::Html => "html",
        Kind::Def => "def",
        Kind::Table => "table",
        Kind::Paragraph => "paragraph",
        Kind::Text => "text",
        Kind::LatexBlock => "latexBlock",
        Kind::Latex => "latex",
        Kind::Escape => "escape",
        Kind::Link => "link",
        Kind::Image => "image",
        Kind::Strong => "strong",
        Kind::Em => "em",
        Kind::Codespan => "codespan",
        Kind::Br => "br",
        Kind::Del => "del",
        Kind::Checkbox => "checkbox",
    }
}

fn cell(c: &Cell) -> Value {
    json!({ "text": c.text, "tokens": c.tokens.iter().map(proj).collect::<Vec<_>>() })
}

fn proj(t: &Token) -> Value {
    let k = kind(t.kind);
    let text = match t.kind {
        Kind::Space | Kind::Hr | Kind::Br | Kind::List | Kind::Checkbox | Kind::Table => "",
        Kind::Def => "",
        _ => &t.text,
    };
    json!({
        "type": k, "raw": t.raw, "text": text,
        "depth": if t.kind == Kind::Heading { t.depth } else { 0 },
        "lang": if t.kind == Kind::Code { t.lang.clone().map(Value::String).unwrap_or(Value::Null) } else { Value::Null },
        "href": if matches!(t.kind, Kind::Link | Kind::Image) { t.href.clone() } else { String::new() },
        "title": if matches!(t.kind, Kind::Link | Kind::Image) { t.title.clone().map(Value::String).unwrap_or(Value::Null) } else { Value::Null },
        "ordered": t.ordered, "start": t.start, "loose": t.loose, "task": t.task, "checked": t.checked, "pending": t.pending,
        "tokens": t.tokens.iter().map(proj).collect::<Vec<_>>(),
        "items": t.items.iter().map(proj).collect::<Vec<_>>(),
        "header": t.header.iter().map(cell).collect::<Vec<_>>(),
        "rows": t.rows.iter().map(|r| r.iter().map(cell).collect::<Vec<_>>()).collect::<Vec<_>>(),
    })
}

#[test]
fn lexer_matches_marked() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/markdown.json")).unwrap();
    let want: Value = serde_json::from_str(include_str!("fixtures/markdown_tokens.json")).unwrap();
    let cases = cases["cases"].as_array().unwrap();
    let mut failures = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let src = c["t"].as_str().unwrap().replace('\t', "   ");
        let got = Value::Array(lex(&src).iter().map(proj).collect());
        if got != want[i] {
            failures.push(format!(
                "case {i} {src:?}\n  want {}\n  got  {}",
                want[i], got
            ));
        }
    }
    if !failures.is_empty() {
        let n = failures.len();
        let shown: Vec<_> = failures.into_iter().take(8).collect();
        panic!(
            "{n} of {} token trees differ:\n{}",
            cases.len(),
            shown.join("\n")
        );
    }
}
