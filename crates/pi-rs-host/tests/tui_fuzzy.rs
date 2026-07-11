//! Public Lua seam exerciser for the fuzzy-filter mechanism binding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn fuzzy_demo_filters_and_orders_like_pi_tui() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/fuzzy-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let ids = |query: &str| -> Vec<String> {
        // An empty Lua table encodes as `{}`; treat it as the empty list.
        host.call_command(
            "fuzzy-demo",
            &serde_json::json!({ "query": query }).to_string(),
        )
        .expect("command")
        .expect("result")["ids"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };

    // Empty query keeps declaration order (spec: early return).
    assert_eq!(
        ids(""),
        [
            "anthropic",
            "openai",
            "openai-codex",
            "openrouter",
            "google"
        ]
    );
    // In-order matching: "open" excludes Anthropic and Google; equal
    // prefix scores keep declaration order (stable sort, as JS).
    assert_eq!(ids("open"), ["openai", "openai-codex", "openrouter"]);
    // Space-separated tokens AND together.
    assert_eq!(ids("open codex"), ["openai-codex"]);
    // Non-matching query filters everything.
    assert!(ids("qqq").is_empty());
}

#[test]
fn fuzzy_match_and_js_regex_search_expose_the_selector_search_seams() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/fuzzy-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command(
            "fuzzy-demo",
            &serde_json::json!({ "query": "anthro" }).to_string(),
        )
        .expect("command")
        .expect("result");
    // fuzzy_match: in-order match with a numeric score.
    assert_eq!(result["matches"], true);
    assert!(result["score"].as_f64().is_some());
    // js_regex_search: case-insensitive JS index ("prefer " = 7 units).
    assert_eq!(result["regexIndex"], 7);
    // Invalid patterns surface (nil, message) like the caught RegExp error.
    assert_eq!(result["invalidIsNil"], true);
    assert_eq!(result["invalidHasError"], true);
}
