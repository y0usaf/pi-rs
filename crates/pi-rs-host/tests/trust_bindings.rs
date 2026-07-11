#![allow(clippy::unwrap_used, clippy::expect_used)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn trust_store_example_exercises_public_bindings() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("agent");
    let project = root.path().join("workspace/project");
    std::fs::create_dir_all(project.join(".pi")).unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent) };

    let host = Host::new(HostConfig::default()).expect("host");
    host.load_file("../../examples/extensions/trust-store-demo.lua")
        .expect("example");
    let cwd = project.to_string_lossy();
    let result = host
        .call_command(
            "trust-store-demo",
            &serde_json::json!({ "cwd": cwd, "decision": true, "includeSessionOnly": true })
                .to_string(),
        )
        .unwrap()
        .unwrap();

    assert_eq!(result["hasInputs"], true);
    assert_eq!(result["entry"]["decision"], true);
    assert_eq!(result["entry"]["path"], cwd.as_ref());
    assert_eq!(result["options"].as_array().unwrap().len(), 5);
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(agent.join("trust.json")).unwrap()).unwrap();
    assert_eq!(persisted[cwd.as_ref()], true);
}
