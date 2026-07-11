//! WS6.9 public Lua seam exercisers for stdin buffering and terminal state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host_with_examples() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    for name in [
        "tui-stdin-buffer-demo",
        "tui-terminal-demo",
        "tui-process-loop-demo",
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
fn tui_session_example_pins_lifecycle_input_render_resize_and_stop() {
    let host = host_with_examples();
    let result = host
        .call_command("tui-process-loop-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(result["input"], serde_json::json!(["x"]));
    assert_eq!(result["coalesced"], true);
    assert_eq!(result["idle"], false);
    assert_eq!(result["resized"], true);
    assert_eq!(result["fullRedraws"], 2);
    let output = result["output"].as_str().expect("output string");
    assert!(output.starts_with("\u{1b}[?2004h\u{1b}[>7u\u{1b}[?u\u{1b}[c\u{1b}[?25l"));
    assert!(!output.contains(pi_rs_tui::tui::CURSOR_MARKER));
    assert!(output.ends_with("\r\n\u{1b}[?25h\u{1b}[?2004l\u{1b}[<u"));
}

#[test]
fn live_process_constructor_is_public_without_acquiring_the_terminal() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load(
        "<live-process-constructor>",
        r#"local pi = ...
pi.register_command("live-process-constructor", {
  handler = function()
    local process = pi.tui.process_session(false)
    local dimensions = process:dimensions()
    return { columns = dimensions.columns, rows = dimensions.rows }
  end,
})"#,
    )
    .expect("extension loads");
    let result = host
        .call_command("live-process-constructor", "")
        .expect("command")
        .expect("result");
    assert!(result["columns"].as_u64().is_some_and(|value| value > 0));
    assert!(result["rows"].as_u64().is_some_and(|value| value > 0));
}
