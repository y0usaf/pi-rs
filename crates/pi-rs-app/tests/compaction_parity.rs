//! PLAN 6.5 — compaction differential parity against Pi.
//!
//! `tests/compaction-parity/oracle.json` is generated from Pi's real
//! compaction pipeline (`scripts/compaction-oracle` over
//! `core/compaction/compaction.ts` and pi-ai `utils/overflow.ts`); each
//! case replays here through the Lua compaction policy
//! (`utils/compaction.lua`) via the `compaction-parity` command in the
//! coding-agent pack. Cut points, messages to summarize, file ops,
//! summarization requests (system prompt, prompt text, maxTokens,
//! reasoning), summaries/details, token estimates, and overflow
//! decisions compare as parsed JSON. Empty Lua tables normalize to `[]`
//! (Lua has one empty-table value for both encodings).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_app::builtins::{CODING_AGENT_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

const CASES: &str = include_str!("../../../tests/compaction-parity/cases.json");
const ORACLE: &str = include_str!("../../../tests/compaction-parity/oracle.json");

/// Fold Lua's `{}`/`[]` encoding artifact: any empty object compares as
/// an empty array.
fn normalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) if map.is_empty() => {
            *value = serde_json::Value::Array(Vec::new());
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map.iter_mut() {
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
fn compaction_pipeline_matches_pi_for_every_oracle_case() {
    let cases: serde_json::Value = serde_json::from_str(CASES).unwrap();
    let oracle: serde_json::Value = serde_json::from_str(ORACLE).unwrap();
    let models = &cases["models"];

    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(temp.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let mut checked = 0usize;
    for (case, expected) in cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let name = case["name"].as_str().unwrap();
        assert_eq!(expected["name"].as_str().unwrap(), name, "oracle order");

        let request = serde_json::json!({ "case": case, "models": models });
        let mut actual = host
            .call_command("compaction-parity", &request.to_string())
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .unwrap();
        actual["name"] = serde_json::json!(name);

        let mut expected = expected.clone();
        normalize(&mut expected);
        normalize(&mut actual);
        assert_eq!(actual, expected, "{name}");
    }
    for _ in oracle["cases"].as_array().unwrap() {
        checked += 1;
    }
    assert_eq!(
        checked,
        cases["cases"].as_array().unwrap().len(),
        "every oracle case replayed"
    );
}
