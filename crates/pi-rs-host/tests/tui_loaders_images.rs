//! Public Lua exerciser for generic terminal image mechanisms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn image_example_renders_deterministic_protocol_snapshot() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/tui-image-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("tui-image-demo", "")
        .expect("command")
        .expect("result");

    assert_eq!(result["rows"], 1);
    assert_eq!(result["image"], true);
    assert_eq!(result["fallback"], "[Image: demo.png [image/png] 20x20]");
    assert_eq!(
        result["hyperlink"],
        "\u{1b}]8;;https://pi.dev\u{1b}\\pi\u{1b}]8;;\u{1b}\\"
    );
    assert_eq!(result["deleted"], "\u{1b}_Ga=d,d=I,i=42,q=2\u{1b}\\");
}
