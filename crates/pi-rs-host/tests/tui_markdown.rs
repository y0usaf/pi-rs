//! Public Lua seam exerciser for the themed markdown mechanism.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn markdown_demo_pins_theme_default_style_and_marker_options() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/tui-markdown-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("tui-markdown-demo", "")
        .expect("command")
        .expect("result");

    let plain: Vec<String> = result["plain"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect();
    // Pi list bullets are "- ", headings drop their prefix at depth < 3.
    assert_eq!(plain[0].trim_end(), " Ready");
    assert!(plain.iter().any(|line| line.trim_end() == " - exact"));

    let themed: Vec<String> = result["themed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect();
    // Theme bold wraps the color-styled content and the default-style prefix
    // is re-applied after each inline segment (pi renderInlineTokens).
    assert!(
        themed[0].contains("\u{1b}[1m\u{1b}[38;5;250mbold\u{1b}[39m\u{1b}[22m\u{1b}[38;5;250m")
    );
    assert!(themed[0].contains("\u{1b}[36mcode\u{1b}[39m"));
    // First token is Strong, so the line begins with the bold open code.
    assert!(themed[0].starts_with("\u{1b}[1m"));
    assert!(themed.iter().any(|line| line.contains("3) ")));

    assert_eq!(result["args_json"], "{\n  \"path\": \"a.txt\"\n}");
}
