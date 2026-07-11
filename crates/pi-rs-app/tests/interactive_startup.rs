//! WS7/8.1 exact startup-policy core through the public Lua surface.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[pi_rs_app::builtins::INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

fn request(width: usize) -> serde_json::Value {
    serde_json::json!({
        "theme": "dark", "color_mode": "truecolor", "app_name": "pi",
        "version": "0.1.0", "expanded": false, "width": width,
        "cwd": "/tmp/project", "branch": "main", "session_name": "",
        "usage": {"input": 100, "output": 10, "cache_read": 50, "cache_write": 50, "cost": 0.001},
        "context_percent": 12.3, "context_window": 200000, "auto_compact": true,
        "model_id": "test-model", "provider": "test", "provider_count": 1,
        "reasoning": false, "thinking_level": "off"
    })
}

#[test]
fn compact_header_matches_pi_text_and_dark_theme_codes() {
    let result = host()
        .call_command("interactive-startup-core", &request(93).to_string())
        .unwrap()
        .unwrap();
    let header = result["header"].as_str().unwrap();
    assert!(header.starts_with(
        "\u{1b}[1m\u{1b}[38;2;138;190;183mpi\u{1b}[39m\u{1b}[22m\u{1b}[38;2;102;102;102m v0.1.0"
    ));
    assert!(header.contains("escape\u{1b}[39m\u{1b}[38;2;128;128;128m interrupt"));
    assert!(header.contains("Press ctrl+o to show full startup help and loaded resources."));
    assert!(header.ends_with("Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.\u{1b}[39m"));
    assert_eq!(result["theme"], "dark");
}

#[test]
fn footer_matches_pi_usage_and_width_contract() {
    let result = host()
        .call_command("interactive-startup-core", &request(93).to_string())
        .unwrap()
        .unwrap();
    let lines = result["footer"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].as_str().unwrap().contains("/tmp/project (main)"));
    let stats = lines[1].as_str().unwrap();
    assert!(stats.contains("↑100 ↓10 R50 W50 CH25.0% $0.001 12.3%/200k (auto)"));
    assert!(stats.contains("test-model"));
}

#[test]
fn custom_editor_routes_policy_before_editor_mechanism() {
    let request = serde_json::json!({
        "value": "",
        "actions": ["app.tools.expand"],
        "extension_shortcut": "ctrl+x",
        "input": ["\u{0018}", "\u{000f}", "\u{0016}", "\u{001b}", "\u{0004}"]
    });
    let result = host()
        .call_command("interactive-custom-editor", &request.to_string())
        .unwrap()
        .unwrap();
    assert_eq!(
        result["trace"],
        serde_json::json!([
            "extension",
            "app.tools.expand",
            "pasteImage",
            "escape",
            "exit"
        ])
    );
    assert_eq!(result["text"], "");
}

#[test]
fn custom_editor_leaves_nonempty_ctrl_d_to_delete_forward() {
    let request = serde_json::json!({ "value": "ab", "input": ["\u{001b}[D", "\u{0004}"] });
    let result = host()
        .call_command("interactive-custom-editor", &request.to_string())
        .unwrap()
        .unwrap();
    assert!(
        result["trace"]
            .as_object()
            .is_some_and(|trace| trace.is_empty())
    );
    assert_eq!(result["text"], "a");
    assert_eq!(result["effects"][0]["kind"], "none");
    assert_eq!(result["effects"][1]["kind"], "changed");
}

#[test]
fn frontend_frame_mounts_pi_message_components_without_extra_chrome() {
    let request = serde_json::json!({
        "theme": "dark", "colorMode": "truecolor", "version": "0.1.0",
        "width": 72, "cwd": "/tmp/project", "branch": "main",
        "model": {
            "id": "test-model", "provider": "test", "reasoning": false,
            "contextWindow": 200000
        },
        "transcript": [
            {"kind": "user", "text": "hello"},
            {"kind": "assistant", "message": {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "considering"},
                    {"type": "text", "text": "world"}
                ],
                "stopReason": "stop"
            }},
            {"kind": "tool", "name": "read", "args": {"path": "a.txt"},
             "state": "success",
             "result": {"content": [{"type": "text", "text": "file body"}]}}
        ],
        "streaming": "next", "status": "Thinking…", "editor": "first\nsecond"
    });
    let result = host()
        .call_command("interactive-frame", &request.to_string())
        .unwrap()
        .unwrap();
    let joined = result["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    // Pi-derived presentation: message bodies, thinking trace, tool title,
    // args JSON, tool output, streaming text, status, editor, footer.
    for expected in [
        "pi",
        "hello",
        "considering",
        "world",
        "read",
        "a.txt",
        "file body",
        "next",
        "Thinking…",
        "first",
        "second",
        "/tmp/project (main)",
        "test-model",
    ] {
        assert!(
            joined.contains(expected),
            "missing {expected:?} in {joined:?}"
        );
    }
    // Pi renders no transcript role labels; the placeholder chrome is gone.
    for forbidden in ["you: ", "assistant: ", "thinking: ", "tool: "] {
        assert!(
            !joined.contains(forbidden),
            "placeholder chrome {forbidden:?} leaked into {joined:?}"
        );
    }
    // User messages render on the userMessageBg strip (dark: #343541).
    assert!(joined.contains("\u{1b}[48;2;52;53;65m"));
}

#[test]
fn parity_sequence_exercises_all_shared_checkpoints() {
    let request = include_str!("../../../tests/ui-parity/basic-turn.json");
    let result = host()
        .call_command("interactive-parity-sequence", request)
        .unwrap()
        .unwrap();
    let frames = result["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 6);
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "startup",
            "submitted",
            "streaming",
            "complete",
            "resize",
            "resize-wide"
        ]
    );
    assert_eq!(frames[0]["columns"], 72);
    assert_eq!(frames[4]["columns"], 48);
    assert_eq!(frames[5]["columns"], 100);
    assert!(frames.iter().all(|frame| frame["ansi"].as_str().is_some()));
}

#[test]
fn tool_render_example_drives_custom_renderers_through_the_transcript() {
    let host = host();
    host.load_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/tool-render-demo.lua"
    ))
    .unwrap();
    let request = serde_json::json!({
        "theme": "dark", "colorMode": "truecolor", "version": "0.1.0",
        "width": 60, "cwd": "/tmp/project", "branch": "main",
        "model": {
            "id": "test-model", "provider": "test", "reasoning": false,
            "contextWindow": 200000
        },
        "transcript": [
            {"kind": "tool", "toolCallId": "t1", "name": "render-demo",
             "args": {"target": "world"}, "state": "success",
             "executionStarted": true, "argsComplete": true,
             "result": {"content": [{"type": "text", "text": "greeted world"}]}}
        ]
    });
    let result = host
        .call_command("interactive-frame", &request.to_string())
        .unwrap()
        .unwrap();
    let joined = result["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    // The custom renderers replace the generic fallback: no pretty-printed
    // args JSON, and the renderer-authored strings appear on the success
    // Box background (dark toolSuccessBg #283228).
    assert!(joined.contains("render-demo"), "{joined:?}");
    assert!(joined.contains("world"), "{joined:?}");
    assert!(joined.contains("greeted world"), "{joined:?}");
    assert!(!joined.contains("\"target\""), "{joined:?}");
    assert!(joined.contains("\u{1b}[48;2;40;50;40m"), "{joined:?}");
}

#[test]
fn public_text_utils_example_exercises_new_api() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/tui-text-utils-demo.lua"
    ))
    .unwrap();
    let result = host
        .call_command("tui-text-utils-demo", "")
        .unwrap()
        .unwrap();
    assert_eq!(result["width"], 8);
    assert_eq!(result["padded"], "pi   ");
    // finalizeTruncatedResult: the cut and the ellipsis carry full resets.
    assert!(
        result["clipped"]
            .as_str()
            .unwrap()
            .contains("界a\u{1b}[0m...\u{1b}[0m")
    );
}

#[test]
fn submit_router_matches_pi_command_interception() {
    let request = serde_json::json!({
        "texts": ["   ", "/quit", "hello", "/unknown-cmd arg", "  spaced  ", "/login", "/logout"]
    });
    let result = host()
        .call_command("interactive-submit-route", &request.to_string())
        .unwrap()
        .unwrap();
    let trace = result["trace"].as_array().unwrap();
    // Whitespace-only input is dropped before routing (spec: text.trim()).
    assert_eq!(trace.len(), 9);
    // "/quit" clears the editor and shuts down.
    assert_eq!(trace[0]["action"], "set_text");
    assert_eq!(trace[0]["value"], "");
    assert_eq!(trace[1]["action"], "quit");
    // Non-command text prompts; unknown "/" commands fall through to the
    // prompt path (pi: extension commands and template expansion resolve
    // inside session.prompt); surrounding whitespace is trimmed.
    assert_eq!(trace[2]["action"], "prompt");
    assert_eq!(trace[2]["value"], "hello");
    assert_eq!(trace[3]["action"], "prompt");
    assert_eq!(trace[3]["value"], "/unknown-cmd arg");
    assert_eq!(trace[4]["action"], "prompt");
    assert_eq!(trace[4]["value"], "spaced");
    // "/login" and "/logout" open the auth selectors, then clear the
    // editor (spec: showOAuthSelector(mode) before editor.setText("")).
    assert_eq!(trace[5]["action"], "show_oauth_selector");
    assert_eq!(trace[5]["value"], "login");
    assert_eq!(trace[6]["action"], "set_text");
    assert_eq!(trace[7]["action"], "show_oauth_selector");
    assert_eq!(trace[7]["value"], "logout");
    assert_eq!(trace[8]["action"], "set_text");
}

#[test]
fn submit_router_matches_pi_bash_interception() {
    // `!`/`!!` route to handleBashCommand after history; a bare "!" falls
    // through to the prompt path (spec: `if (command)`).
    let request = serde_json::json!({
        "texts": ["!ls -la", "!! printf x", "!", "!!"]
    });
    let result = host()
        .call_command("interactive-submit-route", &request.to_string())
        .unwrap()
        .unwrap();
    let trace = result["trace"].as_array().unwrap();
    assert_eq!(trace.len(), 6);
    assert_eq!(trace[0]["action"], "add_to_history");
    assert_eq!(trace[0]["value"], "!ls -la");
    assert_eq!(trace[1]["action"], "bash_command");
    assert_eq!(trace[1]["value"], "ls -la");
    assert_eq!(trace[1]["excluded"], false);
    assert_eq!(trace[2]["action"], "add_to_history");
    assert_eq!(trace[3]["action"], "bash_command");
    assert_eq!(trace[3]["value"], "printf x");
    assert_eq!(trace[3]["excluded"], true);
    assert_eq!(trace[4]["action"], "prompt");
    assert_eq!(trace[4]["value"], "!");
    assert_eq!(trace[5]["action"], "prompt");
    assert_eq!(trace[5]["value"], "!!");

    // A running command warns and restores the text instead of running.
    let request = serde_json::json!({ "texts": ["!echo x"], "bashRunning": true });
    let result = host()
        .call_command("interactive-submit-route", &request.to_string())
        .unwrap()
        .unwrap();
    let trace = result["trace"].as_array().unwrap();
    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0]["action"], "show_warning");
    assert_eq!(
        trace[0]["value"],
        "A bash command is already running. Press Esc to cancel it first."
    );
    assert_eq!(trace[1]["action"], "set_text");
    assert_eq!(trace[1]["value"], "!echo x");
}

#[test]
fn shell_sequence_pins_queue_restore_abort_and_press_again_exit() {
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/ui-parity/shell-turn.json")).unwrap();
    scenario["steps"] = serde_json::json!([
        { "name": "submit", "input": ["hello", "\r"] },
        { "name": "steer", "input": ["queued", "\r"] },
        { "name": "follow-up", "input": ["later", "\u{001b}[13;3u"] },
        { "name": "abort", "input": ["\u{001b}"] },
        { "name": "exit", "input": ["\u{0003}", "\u{0003}"] }
    ]);
    let result = host()
        .call_command("interactive-shell-parity-sequence", &scenario.to_string())
        .unwrap()
        .unwrap();
    // Streaming submissions steer; alt+enter queues a follow-up; escape
    // restores both to the editor and aborts (spec
    // restoreQueuedMessagesToEditor({ abort: true })).
    assert_eq!(
        result["events"],
        serde_json::json!([
            { "type": "prompt", "text": "hello" },
            { "type": "steer", "text": "queued" },
            { "type": "followUp", "text": "later" },
            { "type": "abort" }
        ])
    );
    // handleCtrlC: the first press clears, the second within 500ms exits.
    assert_eq!(result["exited"], true);
    // The abort frame shows the restored queue in the editor and no loader.
    let frames = result["frames"].as_array().unwrap();
    let abort = frames
        .iter()
        .find(|frame| frame["name"] == "abort")
        .unwrap()["ansi"]
        .as_str()
        .unwrap();
    assert!(abort.contains("queued"), "{abort:?}");
    assert!(abort.contains("later"), "{abort:?}");
    assert!(abort.contains("Operation aborted"), "{abort:?}");
    assert!(!abort.contains("Working..."), "{abort:?}");
}

#[test]
fn shortcut_example_fires_through_registered_shortcuts() {
    let host = host();
    host.load_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/shortcut-demo.lua"
    ))
    .unwrap();
    let result = host.call_command("shortcut-demo", "").unwrap().unwrap();
    // Re-registration replaced the handler in place; the key is lowercased.
    assert_eq!(
        result["shortcuts"],
        serde_json::json!([{ "shortcut": "ctrl+x", "description": "Replacement wins" }])
    );
    assert_eq!(result["notices"], serde_json::json!(["replaced ping"]));
    assert_eq!(result["fired"], serde_json::json!([{ "replaced": true }]));
}

#[test]
fn selector_overlay_confirms_and_cancels_through_show_selector() {
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/ui-parity/selector-turn.json")).unwrap();
    // Reuse the parity scenario but confirm with enter after filtering.
    scenario["steps"] = serde_json::json!([
        { "name": "open", "show": true },
        { "name": "filter", "input": ["c", "o", "d", "e", "x"] },
        { "name": "confirm", "input": ["\r"] },
        { "name": "reopen", "show": true },
        { "name": "cancel", "input": ["\u{1b}"] }
    ]);
    let result = host()
        .call_command(
            "interactive-selector-parity-sequence",
            &scenario.to_string(),
        )
        .unwrap()
        .unwrap();
    let events = result["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "select");
    assert_eq!(events[0]["id"], "openai-codex");
    assert_eq!(events[1]["type"], "cancel");
    // done() restored the editor slot after each: the final frame is the
    // focused editor stand-in, not selector chrome.
    let frames = result["frames"].as_array().unwrap();
    let last = frames.last().unwrap()["ansi"].as_str().unwrap();
    assert!(!last.contains("Select provider"), "{last:?}");
}
