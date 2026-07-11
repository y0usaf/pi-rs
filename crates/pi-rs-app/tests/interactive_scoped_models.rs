//! `/scoped-models` product wiring: ordered session scope + persistence.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn selector_order_drives_cycle_and_persists_exact_ids() {
    let agent_dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[pi_rs_app::builtins::INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let scenario_path = format!(
        "{}/../../tests/ui-parity/scoped-models-turn.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario = std::fs::read_to_string(scenario_path).unwrap();
    let result = host
        .call_command("interactive-scoped-models-parity-sequence", &scenario)
        .expect("command")
        .expect("result");

    let ordered = serde_json::json!(["openai/gpt-5.2", "openai/gpt-5.4", "openai/gpt-5-mini"]);
    assert_eq!(result["scopedModels"], ordered);
    assert_eq!(result["savedModels"], ordered);
    assert_eq!(result["currentModel"], "openai/gpt-5-mini");

    let persisted: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(agent_dir.path().join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["enabledModels"], ordered);
}
