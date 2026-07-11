//! WS6.7 public Lua seam exercisers for loaders and terminal images.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn run(name: &str) -> serde_json::Value {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/{name}.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = std::fs::read_to_string(path).expect("example exists");
    host.load(&format!("examples/extensions/{name}.lua"), &source)
        .expect("example loads");
    host.call_command(name, "")
        .expect("command")
        .expect("result")
}

#[test]
fn loader_example_advances_and_cancels() {
    let result = run("tui-loader-demo");
    assert_eq!(
        result["before"],
        serde_json::json!(["", " a Working...   "])
    );
    assert_eq!(result["after"], serde_json::json!(["", " b Working...   "]));
    assert_eq!(result["aborted"], true);
    assert_eq!(result["running"], true);
}

#[test]
fn image_example_renders_deterministic_protocol_snapshot() {
    let result = run("tui-image-demo");
    assert_eq!(result["rows"], 1);
    assert_eq!(result["image"], true);
    assert_eq!(result["fallback"], "[Image: demo.png [image/png] 20x20]");
    assert_eq!(
        result["hyperlink"],
        "\u{1b}]8;;https://pi.dev\u{1b}\\pi\u{1b}]8;;\u{1b}\\"
    );
    assert_eq!(result["deleted"], "\u{1b}_Ga=d,d=I,i=42,q=2\u{1b}\\");
}
