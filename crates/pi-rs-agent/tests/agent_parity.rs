//! Pins the Lua agent loop (crates/pi-rs-agent/lua/agent.lua) to Pi's real
//! Agent/agent-loop.ts. The oracle in tests/agent-parity/oracle.json is
//! generated from ref/pi by scripts/agent-oracle; each case replays through
//! the public surface (pi.agent.new via tests/agent-parity/driver.lua,
//! loaded like a user extension) and compares the full subscriber event
//! sequence, per-stream-call request snapshots, per-phase prompt/continue
//! outcomes, and final agent state.
//!
//! Normalization (recorded): `timestamp` fields are scrubbed to 0 on both
//! sides (Date.now vs os.time), and empty objects compare equal to empty
//! arrays (the Lua `{}` JSON-encoding artifact) — everything else is
//! compared structurally.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_agent::PACK;
use pi_rs_host::{Host, HostConfig};
use serde_json::Value;

fn fixture_path(name: &str) -> String {
    format!(
        "{}/../../tests/agent-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn fixture(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture_path(name)).unwrap()).unwrap()
}

/// Scrub timestamps and fold the empty-object/empty-array encoding artifact.
fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) if map.is_empty() => *value = Value::Array(Vec::new()),
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if key == "timestamp" && item.is_number() {
                    *item = Value::from(0);
                } else {
                    normalize(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        _ => {}
    }
}

fn first_divergence(path: &str, actual: &Value, expected: &Value) -> Option<String> {
    match (actual, expected) {
        (Value::Array(a), Value::Array(b)) => {
            for index in 0..a.len().max(b.len()) {
                match (a.get(index), b.get(index)) {
                    (Some(x), Some(y)) => {
                        if let Some(diff) = first_divergence(&format!("{path}[{index}]"), x, y) {
                            return Some(diff);
                        }
                    }
                    (Some(x), None) => {
                        return Some(format!("{path}[{index}]: pi-rs has extra element {x}"));
                    }
                    (None, Some(y)) => {
                        return Some(format!("{path}[{index}]: pi-rs is missing element {y}"));
                    }
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => {
                        if let Some(diff) = first_divergence(&format!("{path}.{key}"), x, y) {
                            return Some(diff);
                        }
                    }
                    (Some(x), None) => return Some(format!("{path}.{key}: pi-rs has extra {x}")),
                    (None, Some(y)) => {
                        return Some(format!("{path}.{key}: pi-rs is missing {y}"));
                    }
                    (None, None) => {}
                }
            }
            None
        }
        _ if actual == expected => None,
        _ => Some(format!("{path}: pi-rs {actual} != pi {expected}")),
    }
}

#[test]
fn agent_loop_event_order_matches_pi() {
    let cases = fixture("cases.json");
    let oracle = fixture("oracle.json");
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let driver = std::fs::read_to_string(fixture_path("driver.lua")).unwrap();
    host.load("test://agent-parity-driver", &driver)
        .expect("load driver");

    let models = &cases["models"];
    let case_list = cases["cases"].as_array().unwrap();
    let oracle_list = oracle["cases"].as_array().unwrap();
    assert_eq!(
        case_list.len(),
        oracle_list.len(),
        "oracle is stale: regenerate with scripts/agent-oracle"
    );

    for (case, oracle_case) in case_list.iter().zip(oracle_list) {
        let name = case["name"].as_str().unwrap();
        assert_eq!(
            name,
            oracle_case["name"].as_str().unwrap(),
            "oracle is stale: regenerate with scripts/agent-oracle"
        );
        let payload = serde_json::json!({ "case": case, "models": models });
        let mut actual = host
            .call_command("agent-parity", &payload.to_string())
            .expect("call")
            .expect("value");
        let mut expected = oracle_case.clone();
        normalize(&mut actual);
        normalize(&mut expected);
        for section in ["events", "requests", "phases", "state"] {
            if let Some(diff) = first_divergence(section, &actual[section], &expected[section]) {
                panic!("case {name}: {diff}");
            }
        }
    }
}
