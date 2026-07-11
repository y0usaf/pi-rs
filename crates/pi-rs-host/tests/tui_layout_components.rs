//! WS6.8 public Lua component surface integration fixture.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn layout_example_exercises_all_component_userdata() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/tui-layout-demo.lua"
    );
    let source = std::fs::read_to_string(path).expect("layout exerciser exists");
    host.load("examples/extensions/tui-layout-demo.lua", &source)
        .expect("layout exerciser loads");
    let result = host
        .call_command("tui-layout-demo", "")
        .expect("command runs")
        .expect("command returns a result");

    assert_eq!(result["input_value"], "xyz");
    assert_eq!(
        result["changed"],
        serde_json::json!({"kind": "changed", "value": "xyz"})
    );
    assert_eq!(
        result["submitted"],
        serde_json::json!({"kind": "submit", "value": "xyz"})
    );
    assert_eq!(result["selected"]["id"], "mode");
    assert_eq!(
        result["action"],
        serde_json::json!({"id": "mode", "value": "fast"})
    );
    assert_eq!(
        result["settings_changed"],
        serde_json::json!({"kind": "changed", "id": "mode", "value": "safe"})
    );
    assert_eq!(
        result["settings_cancelled"],
        serde_json::json!({"kind": "cancel"})
    );
    assert!(
        result["settings_lines"][0]
            .as_str()
            .is_some_and(|line| line.starts_with("> \x1b[7mm\x1b[27mode"))
    );
    assert_eq!(result["spacer_lines"], serde_json::json!(["", ""]));
    // Truncated output carries finalizeTruncatedResult's resets.
    assert_eq!(
        result["truncated_lines"],
        serde_json::json!([" abc\u{1b}[0m...\u{1b}[0m "])
    );
    assert_eq!(result["text_lines"], serde_json::json!(["title   "]));
    assert_eq!(result["empty"], serde_json::json!({}));
    assert_eq!(
        result["lines"],
        serde_json::json!([
            "[            ]",
            "[ title      ]",
            "[  abcde\u{1b}[0m...\u{1b}[0m  ]",
            "[ > xyz\u{1b}_pi:c\u{7}\u{1b}[7m \u{1b}[27m     ]",
            "[            ]"
        ])
    );
}
