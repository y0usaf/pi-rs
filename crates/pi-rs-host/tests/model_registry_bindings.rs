//! Public Lua seam exercisers for the `pi.ai` model-registry bindings
//! (core/model-registry.ts) and `pi.auth.get_api_key` per-provider
//! resolution, through the examples/ conformance extension.
//!
//! This file is its own test binary: it owns the process-global
//! `PI_CODING_AGENT_DIR`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn model_registry_bindings_round_trip_through_the_public_surface() {
    let agent_dir = tempfile::tempdir().unwrap();
    // The VM's auth storage resolves PI_CODING_AGENT_DIR at Host::new.
    // SAFETY: this test binary has no other threads reading the env yet.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };

    let host = Host::new(HostConfig::default()).unwrap();
    let path = format!(
        "{}/../../examples/extensions/model-registry-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).unwrap();

    // A provider without ambient env keys on CI and developer machines.
    let request = serde_json::json!({
        "provider": "moonshotai",
        "model": "kimi-k2.6",
        "key": "sk-demo",
    });
    let result = host
        .call_command("model-registry-demo", &request.to_string())
        .unwrap()
        .unwrap();

    // Catalog rows resolve by provider/id; unknown ids are nil.
    assert_eq!(result["found"]["provider"], "moonshotai", "{result}");
    assert_eq!(result["found"]["id"], "kimi-k2.6");
    assert_eq!(result["missing"], true);

    // Availability follows configured auth through refresh, and the
    // stored key resolves through pi.auth.get_api_key.
    assert_eq!(result["available_before"], false, "{result}");
    assert_eq!(result["available_after"], true);
    assert_eq!(result["has_configured_auth"], true);
    assert_eq!(result["api_key"], "sk-demo");
    assert_eq!(result["subscription"], false);
    // No models.json half yet — never an error.
    assert!(result["registry_error"].is_null());

    // Thinking-level vocabulary (PLAN 7.2): a mapped model supports xhigh
    // (explicitly mapped); a plain reasoning model does not; a
    // non-reasoning model supports only "off"; clamping searches upward
    // first (xhigh -> high on the plain model), and clamps to "off" on a
    // non-reasoning model.
    assert_eq!(
        result["mapped_levels"],
        serde_json::json!(["off", "minimal", "low", "medium", "high", "xhigh"])
    );
    assert_eq!(
        result["plain_levels"],
        serde_json::json!(["off", "minimal", "low", "medium", "high"])
    );
    assert_eq!(result["basic_levels"], serde_json::json!(["off"]));
    assert_eq!(result["clamped_up"], "high");
    assert_eq!(result["clamped_off"], "off");
}
