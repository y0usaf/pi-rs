//! Public Lua seam exerciser for the CombinedAutocompleteProvider binding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn autocomplete_demo_completes_commands_arguments_and_paths() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/tui-autocomplete-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("tui-autocomplete-demo", "{}")
        .expect("command")
        .expect("result");

    // Slash-command menu: fuzzy over names, hint folded into the description.
    assert_eq!(result["menu"]["prefix"], "/re");
    let menu_items = result["menu"]["items"].as_array().expect("menu items");
    assert_eq!(menu_items.len(), 1);
    assert_eq!(menu_items[0]["value"], "resume");
    assert_eq!(menu_items[0]["description"], "<session> — resume a session");

    // Argument completions route through the Lua callback with the
    // argument text as the prefix.
    assert_eq!(result["arguments"]["prefix"], "gpt");
    assert_eq!(result["arguments"]["items"][0]["value"], "openai/gpt-5.4");

    // applyCompletion: slash-command branch inserts "/name " and puts the
    // cursor after the trailing space.
    assert_eq!(result["line"], "/resume ");
    assert_eq!(result["cursor"], 8);

    // Forced ./ path extraction produced directory suggestions.
    assert!(result["file_count"].as_u64().unwrap() > 0);

    // shouldTriggerFileCompletion allows plain text.
    assert_eq!(result["tab_allowed"], true);
}
