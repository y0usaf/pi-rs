//! Pins the Lua system-prompt port (buildSystemPrompt +
//! loadProjectContextFiles + the agent-session normalization/composition
//! in `builtins/utils/system-prompt.lua`) to Pi's real implementation.
//! The oracle in tests/system-prompt-parity/oracle.json is generated
//! from `ref/pi/packages/coding-agent` by scripts/system-prompt-oracle;
//! cases replay through the public Lua surface (`system-prompt-parity`
//! command), never a Rust module.
//!
//! This file is its own test binary: it owns the process-global `TZ` pin
//! (the oracle is generated with TZ=UTC and a fixed epoch).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

const README_PATH: &str = "/pi-rs-pkg/README.md";
const DOCS_PATH: &str = "/pi-rs-pkg/docs";
const EXAMPLES_PATH: &str = "/pi-rs-pkg/examples";

fn fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/system-prompt-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn host() -> Host {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::CODING_AGENT_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

/// An empty Lua table crosses the boundary as `{}`; the oracle's empty
/// context-file list is `[]`. Normalize the encoding artifact.
fn normalize_empty(value: &mut serde_json::Value) {
    if let Some(map) = value.as_object()
        && map.is_empty()
    {
        *value = serde_json::Value::Array(Vec::new());
    }
}

#[test]
fn system_prompt_matches_pi_oracle() {
    // SAFETY: this test binary is single-threaded at this point and owns
    // the process env; the oracle pins TZ=UTC for the date line.
    unsafe { std::env::set_var("TZ", "UTC") };
    let cases = fixture("cases.json");
    let oracle = fixture("oracle.json");
    let host = host();

    let session_cases = cases["session"].as_array().unwrap();
    let session_oracle = oracle["session"].as_array().unwrap();
    assert_eq!(session_cases.len(), session_oracle.len());
    for (case, expected) in session_cases.iter().zip(session_oracle) {
        let name = case["name"].as_str().unwrap();
        let root = tempfile::tempdir().unwrap();
        for (rel, content) in case["tree"].as_object().unwrap() {
            let path = root.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content.as_str().unwrap()).unwrap();
        }
        let root_str = root.path().to_string_lossy().into_owned();
        let mut request = case.clone();
        request["mode"] = "session".into();
        request["cwd"] = root
            .path()
            .join(case["cwd"].as_str().unwrap())
            .to_string_lossy()
            .into_owned()
            .into();
        request["agentDir"] = root
            .path()
            .join(case["agentDir"].as_str().unwrap())
            .to_string_lossy()
            .into_owned()
            .into();
        request["readmePath"] = README_PATH.into();
        request["docsPath"] = DOCS_PATH.into();
        request["examplesPath"] = EXAMPLES_PATH.into();
        request["now"] = (case["nowMs"].as_i64().unwrap() / 1000).into();
        let mut result = host
            .call_command("system-prompt-parity", &request.to_string())
            .unwrap()
            .unwrap();
        normalize_empty(&mut result["contextFiles"]);
        let prompt = result["prompt"]
            .as_str()
            .unwrap()
            .replace(&root_str, "{ROOT}");
        assert_eq!(prompt, expected["prompt"].as_str().unwrap(), "{name}");
        let context_files: serde_json::Value = serde_json::to_value(
            result["contextFiles"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| {
                    serde_json::json!({
                        "path": file["path"].as_str().unwrap().replace(&root_str, "{ROOT}"),
                        "content": file["content"],
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(context_files, expected["contextFiles"], "{name}");
    }

    let raw_cases = cases["raw"].as_array().unwrap();
    let raw_oracle = oracle["raw"].as_array().unwrap();
    assert_eq!(raw_cases.len(), raw_oracle.len());
    for (case, expected) in raw_cases.iter().zip(raw_oracle) {
        let name = case["name"].as_str().unwrap();
        let mut request = case.clone();
        request["mode"] = "raw".into();
        request["readmePath"] = README_PATH.into();
        request["docsPath"] = DOCS_PATH.into();
        request["examplesPath"] = EXAMPLES_PATH.into();
        request["now"] = (case["nowMs"].as_i64().unwrap() / 1000).into();
        let result = host
            .call_command("system-prompt-parity", &request.to_string())
            .unwrap()
            .unwrap();
        assert_eq!(
            result["prompt"].as_str().unwrap(),
            expected["prompt"].as_str().unwrap(),
            "{name}"
        );
    }
}
