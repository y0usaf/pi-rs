//! Public file-backed exercisers for bounded terminal/display mechanisms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host_with_examples() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    for name in [
        "tui-stdin-buffer-demo",
        "tui-terminal-demo",
        "tui-render-demo",
    ] {
        let path = format!(
            "{}/../../examples/extensions/{name}.lua",
            env!("CARGO_MANIFEST_DIR")
        );
        host.load_file(&path).expect("example loads");
    }
    host
}

#[test]
fn stdin_buffer_example_pins_events_and_buffer_bytes() {
    let host = host_with_examples();
    let result = host
        .call_command("tui-stdin-buffer-demo", "")
        .expect("command")
        .expect("result");

    assert_eq!(
        result["first"],
        serde_json::json!([{ "kind": "data", "data": "a" }])
    );
    assert_eq!(result["pending"], "\u{1b}[");
    assert_eq!(
        result["second"],
        serde_json::json!([{ "kind": "data", "data": "\u{1b}[A" }])
    );
    assert_eq!(
        result["paste"],
        serde_json::json!([{ "kind": "paste", "data": "hello world" }])
    );
    assert_eq!(result["cleared"], "");
    assert_eq!(
        result["flushed"],
        serde_json::json!([{ "kind": "data", "data": "\u{1b}[" }])
    );
}

#[test]
fn terminal_example_pins_state_and_all_output_bytes() {
    let host = host_with_examples();
    let result = host
        .call_command("tui-terminal-demo", "")
        .expect("command")
        .expect("result");

    assert_eq!(
        result["dimensions"],
        serde_json::json!({ "columns": 100, "rows": 40 })
    );
    assert_eq!(
        result["started"],
        "\u{1b}[?2004h\u{1b}[>7u\u{1b}[?u\u{1b}[c"
    );
    assert_eq!(result["negotiation"], serde_json::json!({}));
    assert_eq!(result["modify_output"], "\u{1b}[>4;2m");
    assert_eq!(result["kitty_output"], "\u{1b}[>4;0m");
    assert_eq!(
        result["flags"],
        serde_json::json!({ "kitty": true, "modify_other_keys": false })
    );
    assert_eq!(result["input"], serde_json::json!(["x"]));
    assert_eq!(result["flushed"], serde_json::json!(["\u{1b}["]));
    assert_eq!(
        result["drawing"],
        concat!(
            "ok\u{1b}[2B\u{1b}[1A\u{1b}[?25l\u{1b}[?25h\u{1b}[K\u{1b}[J",
            "\u{1b}[2J\u{1b}[H\u{1b}]0;pi-rs\u{7}\u{1b}]9;4;3\u{7}",
            "\u{1b}]9;4;3\u{7}\u{1b}]9;4;0;\u{7}"
        )
    );
    assert_eq!(result["drained"], "\u{1b}[<u");
    assert_eq!(result["discarded"], serde_json::json!({}));
    assert_eq!(result["stopped"], "\u{1b}[?2004l");
}

#[test]
fn retained_display_example_is_versioned_transactional_and_minimal() {
    let host = host_with_examples();
    let result = host
        .call_command("tui-render-demo", "")
        .expect("command")
        .expect("result");

    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["first"]["revision"], 1);
    assert_eq!(result["first"]["visited_nodes"], 2);
    assert_eq!(result["first"]["painted_cells"], 3);
    assert_eq!(
        result["first"]["identities"]["added"],
        serde_json::json!([1, 2])
    );
    assert!(
        result["first"]["ansi"]
            .as_str()
            .is_some_and(|ansi| ansi.contains("\u{1b}[6 q\u{1b}[?25h"))
    );

    assert_eq!(result["unchanged"]["ansi"], "");
    assert_eq!(
        result["unchanged"]["identities"]["retained"],
        serde_json::json!([1, 2])
    );
    assert_eq!(result["changed"]["changed_cells"], 1);
    assert_eq!(
        result["changed"]["identities"]["retained"],
        serde_json::json!([1])
    );
    assert_eq!(
        result["changed"]["identities"]["changed"],
        serde_json::json!([2])
    );

    assert_eq!(result["malformed_ok"], false);
    assert!(
        result["malformed_error"]
            .as_str()
            .is_some_and(|error| error.contains("terminal control data"))
    );
    assert_eq!(result["revision_before_error"], 3);
    assert_eq!(result["revision_after_error"], 3);
    assert_eq!(result["redrawn"]["revision"], 4);
    assert_eq!(result["redrawn"]["full_redraw"], true);
    assert!(
        result["redrawn"]["ansi"]
            .as_str()
            .is_some_and(|ansi| ansi.contains("\u{1b}[?25l"))
    );
}

#[test]
fn live_display_process_constructor_has_no_terminal_side_effect() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load(
        "<live-display-constructor>",
        r#"local pi = ...
pi.register_command("live-display-constructor", {
  handler = function()
    local process = pi.tui.display_process()
    local dimensions = process:dimensions()
    return { columns = dimensions.columns, rows = dimensions.rows }
  end,
})"#,
    )
    .expect("extension loads");
    let result = host
        .call_command("live-display-constructor", "")
        .expect("command")
        .expect("result");
    assert!(result["columns"].as_u64().is_some_and(|value| value > 0));
    assert!(result["rows"].as_u64().is_some_and(|value| value > 0));
}
