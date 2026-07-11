//! Public Lua seam fixture for pi v0.79.0 multiline editor mechanism.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn multiline_editor_example_pins_compatibility_state_effects_and_rendering() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/tui-multiline-editor-demo.lua"
    );
    host.load_file(path)
        .expect("multiline editor example loads");
    let result = host
        .call_command("tui-multiline-editor-demo", "")
        .expect("command runs")
        .expect("command result");

    assert_eq!(
        result["legacy"],
        serde_json::json!({ "value": "hell!o", "cursor": 5 })
    );
    assert_eq!(
        result["normalized_lines"],
        serde_json::json!(["alpha", "beta    !"])
    );
    assert_eq!(result["paste_effect"]["kind"], "changed");
    assert!(
        result["stored_text"]
            .as_str()
            .is_some_and(|text| text.contains("[paste #1 +11 lines]"))
    );
    assert!(
        result["expanded_text"]
            .as_str()
            .is_some_and(|text| text.ends_with("paste-10\npaste-11"))
    );
    assert_eq!(result["cursor"]["line"], 2);
    assert_eq!(result["padding_x"], 1);
    assert_eq!(result["autocomplete_max_visible"], 20);
    assert_eq!(result["submitted"]["kind"], "submit");
    assert_eq!(result["submitted"]["text"], result["expanded_text"]);
    assert_eq!(result["after_submit"], "");
    assert_eq!(result["history_effect"]["kind"], "changed");
    assert_eq!(result["history_text"], result["expanded_text"]);
    assert_eq!(result["disabled_submit_effect"]["kind"], "none");
    assert_eq!(result["newline_effect"]["kind"], "changed");
    assert_eq!(result["newline_text"], "policy\n");
    assert_eq!(result["disable_submit"], true);

    let autocomplete = &result["autocomplete"];
    assert_eq!(
        autocomplete["first_request"]["lines"],
        serde_json::json!(["/m"])
    );
    assert_eq!(
        autocomplete["current_request"]["lines"],
        serde_json::json!(["/mo"])
    );
    assert_eq!(autocomplete["stale_accepted"], false);
    assert_eq!(autocomplete["current_accepted"], true);
    assert_eq!(autocomplete["showing"], false);
    assert_eq!(autocomplete["effect"]["kind"], "changed");
    assert_eq!(autocomplete["value"], "/more ");
    assert_eq!(autocomplete["forced_request"]["force"], true);
    assert_eq!(autocomplete["forced_request"]["explicit_tab"], true);
    assert_eq!(autocomplete["forced_value"], "src/lib.rs");
    assert_eq!(autocomplete["forced_changed"], true);
    assert_eq!(autocomplete["forced_undo"], "src/");
    assert!(
        autocomplete["rendered"]
            .as_array()
            .is_some_and(|lines| lines.iter().any(|line| {
                line.as_str()
                    .is_some_and(|line| line.contains("models") && line.contains("choose a model"))
            }))
    );

    let rendered = result["rendered"].as_array().expect("render rows");
    assert!(rendered.len() >= 3);
    assert!(rendered.iter().any(|line| {
        line.as_str()
            .is_some_and(|line| line.contains("\u{1b}_pi:c\u{7}"))
    }));
}
