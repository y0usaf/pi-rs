//! PLAN 6.1 — session-persistence differential parity against Pi.
//!
//! `tests/session-parity/oracle.json` is generated from Pi's real
//! `AgentSession` + `SessionManager` (`scripts/session-oracle`); each case
//! replays here through the product persistence policy — the
//! `utils/agent-session.lua` fragments over `pi.session.*` — via the
//! `session-parity` command in the coding-agent pack. Comparison is
//! per-line entry sequence with uuids normalized in first-appearance
//! order, timestamps scrubbed, and the case cwd substituted; entries
//! compare as parsed JSON values (Lua tables do not preserve JS object
//! insertion order, and pi reads session files field-wise).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use pi_rs_app::builtins::{CODING_AGENT_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

const CASES: &str = include_str!("../../../tests/session-parity/cases.json");
const ORACLE: &str = include_str!("../../../tests/session-parity/oracle.json");

const ISO_LEN: usize = "2026-01-01T00:00:00.000Z".len();

fn is_iso_timestamp(value: &str) -> bool {
    value.len() == ISO_LEN
        && value.ends_with('Z')
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[10] == b'T'
        && value.as_bytes()[19] == b'.'
}

/// Mirror of gen-oracle.ts `scrubValues`.
fn scrub_values(value: &mut serde_json::Value, cwd: &str) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                scrub_values(item, cwd);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if key == "timestamp" && item.is_number() {
                    *item = serde_json::json!(0);
                } else if key == "timestamp" && item.as_str().map(is_iso_timestamp).unwrap_or(false)
                {
                    *item = serde_json::json!("TS");
                } else {
                    scrub_values(item, cwd);
                }
            }
        }
        serde_json::Value::String(text) if text.contains(cwd) => {
            *text = text.replace(cwd, "{CWD}");
        }
        _ => {}
    }
}

/// Mirror of gen-oracle.ts `normalizeEntries`.
fn normalize_entries(content: &str, cwd: &str) -> Vec<serde_json::Value> {
    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut next = 0usize;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut entry: serde_json::Value = serde_json::from_str(line).unwrap();
            for key in ["id", "parentId"] {
                if let Some(id) = entry.get(key).and_then(serde_json::Value::as_str) {
                    let id = id.to_owned();
                    let mapped = id_map
                        .entry(id)
                        .or_insert_with(|| {
                            next += 1;
                            format!("U{next}")
                        })
                        .clone();
                    entry[key] = serde_json::json!(mapped);
                }
            }
            scrub_values(&mut entry, cwd);
            entry
        })
        .collect()
}

#[test]
fn session_files_match_pi_for_every_oracle_case() {
    let cases: serde_json::Value = serde_json::from_str(CASES).unwrap();
    let oracle: serde_json::Value = serde_json::from_str(ORACLE).unwrap();
    let models = &cases["models"];

    let mut checked = 0usize;
    for (case, expected) in cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        let name = case["name"].as_str().unwrap();
        assert_eq!(expected["name"].as_str().unwrap(), name, "oracle order");

        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd_string = cwd.to_string_lossy().into_owned();
        let session_dir = temp.path().join("sessions");

        let host = Host::new(HostConfig {
            cwd: Some(cwd_string.clone()),
            ..HostConfig::default()
        })
        .unwrap();
        let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, CODING_AGENT_PACK]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        let request = serde_json::json!({
            "case": case,
            "models": models,
            "cwd": cwd_string,
            "sessionDir": session_dir.to_string_lossy(),
            "agentDir": temp.path().to_string_lossy(),
        });
        let result = host
            .call_command("session-parity", &request.to_string())
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .unwrap();

        let session_file = result["sessionFile"].as_str();
        let exists = session_file.map(|path| std::path::Path::new(path).exists()) == Some(true);
        assert_eq!(
            exists, expected["exists"],
            "{name}: session file existence (path {session_file:?})"
        );
        let entries = if exists {
            let content = std::fs::read_to_string(session_file.unwrap()).unwrap();
            normalize_entries(&content, &cwd_string)
        } else {
            Vec::new()
        };
        assert_eq!(
            serde_json::Value::Array(entries),
            expected["entries"],
            "{name}: session entries"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        oracle["cases"].as_array().unwrap().len(),
        "every oracle case replayed"
    );
}
