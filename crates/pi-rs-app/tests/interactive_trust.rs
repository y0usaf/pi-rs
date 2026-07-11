//! `/trust` product wiring: selection, inherited/direct decisions, persistence.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn selector_persists_the_final_direct_untrusted_decision() {
    let agent_dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[pi_rs_app::builtins::INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let scenario_path = format!(
        "{}/../../tests/ui-parity/trust-turn.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario = std::fs::read_to_string(scenario_path).unwrap();
    let result = host
        .call_command("interactive-trust-parity-sequence", &scenario)
        .expect("command")
        .expect("result");

    assert_eq!(result["saved"]["decision"], false);
    assert_eq!(result["saved"]["path"], "/tmp/pi-rs-trust-fixture/project");
    let persisted: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(agent_dir.path().join("trust.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["/tmp/pi-rs-trust-fixture/project"], false);
    assert_eq!(persisted["/tmp/pi-rs-trust-fixture"], true);
}
