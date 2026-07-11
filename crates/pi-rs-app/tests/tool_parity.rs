//! Pins the seven Lua tool ports (crates/pi-rs-app/src/builtins/tools/) to
//! Pi's real `core/tools/` implementations. The oracle in
//! tests/tool-parity/oracle.json is generated from ref/pi by
//! scripts/tool-oracle; cases replay through the public Lua surface (the
//! `tool-parity` command, which mirrors the agent loop's exact
//! prepare_arguments → validate → execute invocation), never a Rust
//! module. Results, details (including truncation shapes), error
//! strings, abort behavior, filesystem effects, and the bash tool's
//! persisted full output are compared byte-for-byte after `{ROOT}` /
//! `{FULL_OUTPUT}` substitution.
//!
//! Boundary (recorded): grep/find cases are restricted to deterministic
//! outputs (one matching file) because rg/fd traverse directories in
//! parallel; multi-file ordering behavior stays covered by the
//! behavioral tests in tools.rs. The find-path-not-found case pins fd's
//! stderr passthrough and therefore assumes a reasonably recent fd.
//! Image cases use a PNG already within pi's resize limits, where pi's
//! auto-resize returns the original bytes (the resize mechanism itself
//! is PLAN 5.3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../../tests/tool-parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn base64_decode(input: &str) -> Vec<u8> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let vals: Vec<u32> = input
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .map(|b| CHARS.iter().position(|c| *c == b).unwrap() as u32)
        .collect();
    for chunk in vals.chunks(4) {
        let mut v: u32 = 0;
        for (i, c) in chunk.iter().enumerate() {
            v |= c << (18 - 6 * i);
        }
        let bytes = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&bytes[..chunk.len() - 1]);
    }
    out
}

/// The shared content generators from tests/tool-parity/gen-oracle.ts.
fn generate(spec: &Value) -> String {
    match spec["gen"].as_str().unwrap() {
        "repeat" => spec["unit"]
            .as_str()
            .unwrap()
            .repeat(usize::try_from(spec["count"].as_i64().unwrap()).unwrap()),
        "lines" => {
            let prefix = spec["prefix"].as_str().unwrap_or("");
            let suffix = spec["suffix"].as_str().unwrap_or("");
            let count = spec["count"].as_i64().unwrap();
            (1..=count)
                .map(|i| format!("{prefix}{i}{suffix}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
        other => panic!("unknown generator {other}"),
    }
}

fn materialize(root: &std::path::Path, case: &Value) {
    if let Some(tree) = case["tree"].as_object() {
        for (rel, value) in tree {
            let path = root.join(rel);
            if rel.ends_with('/') {
                std::fs::create_dir_all(&path).unwrap();
                continue;
            }
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let content = value
                .as_str()
                .map_or_else(|| generate(value), str::to_owned);
            std::fs::write(&path, content).unwrap();
        }
    }
    if let Some(binary) = case["binary"].as_object() {
        for (rel, b64) in binary {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, base64_decode(b64.as_str().unwrap())).unwrap();
        }
    }
}

fn walk_files(root: &std::path::Path) -> serde_json::Map<String, Value> {
    fn visit(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                out.push((rel, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    visit(root, root, &mut files);
    files
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect()
}

#[test]
fn tool_execute_matches_pi_oracle() {
    let cases = fixture("cases.json");
    let oracle = fixture("oracle.json");
    let cases = cases["cases"].as_array().unwrap();
    let expected = oracle["cases"].as_array().unwrap();
    assert_eq!(cases.len(), expected.len(), "case/oracle length mismatch");

    for (case, expected) in cases.iter().zip(expected) {
        let name = case["name"].as_str().unwrap();
        assert_eq!(name, expected["name"].as_str().unwrap());
        let root = tempfile::tempdir().unwrap();
        materialize(root.path(), case);
        let root_str = root.path().to_string_lossy().into_owned();

        let host = Host::new(HostConfig {
            cwd: Some(root_str.clone()),
            ..HostConfig::default()
        })
        .expect("host boots");
        let report = host.load_embedded(&[
            pi_rs_app::builtins::TOOLS_PACK,
            pi_rs_app::builtins::CODING_AGENT_PACK,
        ]);
        assert!(report.errors.is_empty(), "load errors: {:?}", report.errors);

        let request = serde_json::json!({
            "tool": case["tool"],
            "args": case["args"],
            "abort": case.get("abort"),
            "abortAfterMs": case.get("abortAfterMs"),
            "model": case.get("model"),
        });
        let response = host
            .call_command("tool-parity", &request.to_string())
            .unwrap()
            .unwrap();

        assert_eq!(
            response["ok"].as_bool().unwrap(),
            expected["ok"].as_bool().unwrap(),
            "{name}: ok mismatch — got {response}"
        );

        if expected["ok"].as_bool().unwrap() {
            let full_output_path = response["result"]["details"]["fullOutputPath"]
                .as_str()
                .map(str::to_owned);
            if case["recordFullOutput"].as_bool() == Some(true) {
                let path = full_output_path.as_deref().expect("full output path");
                let full = std::fs::read_to_string(path).expect("full output file");
                assert_eq!(
                    full,
                    expected["fullOutput"].as_str().unwrap(),
                    "{name}: persisted full output mismatch"
                );
            }
            let mut serialized = response["result"].to_string();
            serialized = serialized.replace(&root_str, "{ROOT}");
            if let Some(path) = &full_output_path {
                serialized = serialized.replace(path.as_str(), "{FULL_OUTPUT}");
                std::fs::remove_file(path).ok();
            }
            let result: Value = serde_json::from_str(&serialized).unwrap();
            assert_eq!(result, expected["result"], "{name}: result mismatch");
        } else {
            let error = response["error"]
                .as_str()
                .unwrap()
                .replace(&root_str, "{ROOT}");
            assert_eq!(
                error,
                expected["error"].as_str().unwrap(),
                "{name}: error mismatch"
            );
        }

        if case["recordFs"].as_bool() == Some(true) {
            let files = Value::Object(walk_files(root.path()));
            assert_eq!(files, expected["files"], "{name}: filesystem effects");
        }
    }
}
