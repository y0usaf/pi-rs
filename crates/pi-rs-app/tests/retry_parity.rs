#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 7.10 — Pi-derived AgentSession retry classification, attempt/event
//! order, context removal, exhaustion, disabled, and cancellation matrix.

use std::sync::Mutex;

use pi_rs_app::builtins::{INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

const CASES: &str = include_str!("../../../tests/retry-parity/cases.json");
const ORACLE: &str = include_str!("../../../tests/retry-parity/oracle.json");
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn normalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) if map.is_empty() => {
            *value = serde_json::Value::Array(Vec::new());
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                normalize(item);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        _ => {}
    }
}

#[test]
fn retry_policy_matches_pi_for_every_oracle_case() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let spec: serde_json::Value = serde_json::from_str(CASES).unwrap();
    let oracle: serde_json::Value = serde_json::from_str(ORACLE).unwrap();
    unsafe {
        std::env::set_var("PI_OFFLINE", "1");
    }

    for (case, expected) in spec["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let name = case["name"].as_str().unwrap();
        assert_eq!(expected["name"], name, "oracle order");
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let agent_dir = temp.path().join("agent");
        std::fs::create_dir_all(project.join(".pi")).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        let settings = serde_json::json!({
            "lastChangelogVersion": "0.79.0",
            "retry": case.get("settings").cloned().unwrap_or_else(|| serde_json::json!({}))
        });
        std::fs::write(
            project.join(".pi/config.lua"),
            pi_rs_host::config::update_managed_settings("", settings.as_object().unwrap()),
        )
        .unwrap();
        unsafe {
            std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir);
        }

        let host = Host::new(HostConfig {
            cwd: Some(project.to_string_lossy().into_owned()),
            ..HostConfig::default()
        })
        .unwrap();
        let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, INTERACTIVE_PACK]);
        assert!(report.errors.is_empty(), "{name}: {:?}", report.errors);
        let request = serde_json::json!({
            "case": case,
            "model": spec["model"],
            "cwd": project,
            "agentDir": agent_dir,
            "sessionDir": temp.path().join("sessions"),
            "version": "0.79.0"
        });
        let mut actual = host
            .call_command("retry-policy-parity", &request.to_string())
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .unwrap();
        actual["name"] = serde_json::json!(name);
        let mut expected = expected.clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{name}");
    }
}
