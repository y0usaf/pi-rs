//! WS1.5: embedded-pack loading through the public load path.
//! - an `include_str!` pack loads under its synthetic `<name>` key and
//!   its registrations attribute to that key
//! - a failing pack is reported per-pack without aborting the batch
//! - embedded and on-disk registrations share one registry (first
//!   registration per tool name wins, spec: `getAllRegisteredTools`)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{EmbeddedPack, Host, HostConfig};

/// The pack: an example `.lua` compiled into the test binary — the same
/// mechanism pi-rs-app will use for shipped defaults (locked decision).
const HELLO: EmbeddedPack = EmbeddedPack {
    name: "pack:hello",
    source: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/hello.lua"
    )),
};

#[test]
fn embedded_pack_loads_under_synthetic_key() {
    let host = Host::new(HostConfig::default()).expect("host starts");
    let report = host.load_embedded(&[HELLO]);
    assert_eq!(report.loaded, vec!["<pack:hello>".to_owned()]);
    assert!(report.errors.is_empty());

    let tools = host.tools().expect("tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "hello");
    assert_eq!(tools[0].source, "<pack:hello>");

    let result = host
        .call_tool("hello", "call-1", &serde_json::json!({ "name": "pi-rs" }))
        .expect("tool runs");
    assert_eq!(
        result["content"][0],
        serde_json::json!({ "type": "text", "text": "Hello, pi-rs!" })
    );
}

#[test]
fn failing_pack_is_reported_without_aborting_batch() {
    let host = Host::new(HostConfig::default()).expect("host starts");
    let broken = EmbeddedPack {
        name: "pack:broken",
        source: "error('boom at load')",
    };
    let report = host.load_embedded(&[broken, HELLO]);

    assert_eq!(report.loaded, vec!["<pack:hello>".to_owned()]);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].path, "<pack:broken>");
    assert!(
        report.errors[0]
            .error
            .starts_with("Failed to load extension:"),
        "spec error prefix, got: {}",
        report.errors[0].error
    );
    assert!(report.errors[0].error.contains("boom at load"));
}

#[test]
fn embedded_and_disk_share_one_registry_first_wins() {
    let host = Host::new(HostConfig::default()).expect("host starts");
    // Embedded pack loads first (pi-rs-app order: shipped defaults, then
    // user extensions); a later file registration of the same tool name
    // does not shadow it in the mirror.
    let report = host.load_embedded(&[HELLO]);
    assert!(report.errors.is_empty());
    host.load(
        "test://shadow",
        r#"
            local pi = ...
            pi.register_tool({
                name = "hello",
                description = "shadowing hello",
                parameters = { type = "object", properties = {} },
                execute = function() return { content = {} } end,
            })
        "#,
    )
    .expect("load");

    let tools = host.tools().expect("tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].source, "<pack:hello>");
}
