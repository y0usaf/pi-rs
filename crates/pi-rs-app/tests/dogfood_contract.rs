//! Fail-closed checks for the checked dogfood fixture contract (Rust port of
//! the deleted tests/dogfood-suite/test_contract.py; PLAN A.3). The contract
//! is validated and rendered here so the workflow has a Rust owner; the
//! pinned pi-flake source identity is deliberately duplicated outside the
//! contract so a checked fixture cannot silently rewrite it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Pinned pi-flake identity (mirrors scripts/dogfood-oracle).
const REVISION: &str = "94694da7321ce74aa7b82c13db7e60e28c0caba6";
const ROOT_TREE: &str = "d618b6d10b574a8624991b055838e7a16e8c8a35";
const EXTENSIONS_TREE: &str = "c4a04dfe88314b5e48ebb200ccfd546645c3af9e";

/// (id, directory, package_name, version, entrypoint, tree, default_bundle)
const PACKAGES: &[(&str, &str, &str, &str, &str, &str, bool)] = &[
    ("codex-fast", "pi-codex-fast", "pi-codex-fast", "1.0.0", "src/index.ts", "d986679665640e82610eb3a207177f1a213556e2", true),
    ("gecko-websearch", "pi-gecko-websearch", "pi-gecko-websearch", "1.0.0", "src/index.ts", "d14e1be3c3518a73d8e847850e4aa66b90081f22", true),
    ("rtk", "pi-rtk", "@sherif-fanous/pi-rtk", "0.3.0", "index.ts", "aed5f7967be1d941ebef3f7fb40dbba81c1db688", true),
    ("compact", "pi-compact", "pi-compact", "0.1.0", "src/index.ts", "d0985c0c44c23def835c813b0977b0da90befb13", true),
    ("context-janitor", "pi-context-janitor", "pi-context-janitor", "0.1.0", "src/index.ts", "8b0a34c91ec7102fda8e666d9af0512536bd4481", true),
    ("morph", "pi-morph", "pi-morph", "0.1.0", "src/index.ts", "053b9195a0d598a0d5b22461a03bf07c7b5e4f5f", false),
    ("tool-management", "pi-tool-management", "pi-tool-management", "1.0.0", "src/index.ts", "a073980dd35a9a4f73985445b8bfe42c08606283", true),
    ("webfetch", "pi-webfetch", "pi-webfetch", "1.0.0", "src/index.ts", "6a13fa1bb8b47cb48e9a5bb81d42ea1be1afaf43", true),
    ("hashline", "pi-hashline", "pi-hashline", "0.2.0", "src/index.ts", "7496e74f6bedea661678e7709178ced1123279c9", true),
    ("minimal-editor", "pi-minimal-editor", "pi-minimal-editor", "0.1.0", "src/index.ts", "2b61951f3b0c4d3222dedc84e30efa1b199cfd32", true),
    ("working-indicator", "pi-working-indicator", "pi-working-indicator", "0.1.0", "extensions/index.ts", "8ec6bb77bcdcc4b1383952b9c0e95810e18bee06", true),
    ("pomodoro", "pi-pomodoro", "pi-pomodoro", "1.0.0", "src/index.ts", "83f4405a76741b883440aaa3e58eb3befd6082a2", true),
    ("rlm", "pi-rlm", "pi-rlm", "0.1.0", "src/index.ts", "80c5f219c04d502b27092e9b02aef8725d9967ec", true),
    ("review", "earendil_pi-review", "@earendil-works/pi-review", "0.1.0", "review.ts", "0d2767278f978ea450ee471c2f7b01030256323c", true),
    ("vcc", "sting8k_pi-vcc", "@sting8k/pi-vcc", "0.3.12", "index.ts", "0387247559584cc50dde0781f4695234b6afd442", true),
];

const FIXTURE_KINDS: &[&str] = &[
    "provider", "browser_socket", "subprocess", "timer", "filesystem",
    "compaction", "session", "terminal",
];
const LOAD_MODES: &[&str] = &["direct", "configured", "bundled"];

fn load_contract() -> Value {
    let path = repo_root().join("tests/dogfood-suite/contract.json");
    let text = std::fs::read_to_string(&path).expect("read contract.json");
    serde_json::from_str(&text).expect("contract.json parses")
}

fn as_object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{what} must be an object"))
}

fn as_array<'a>(value: &'a Value, what: &str) -> Result<&'a Vec<Value>, String> {
    value.as_array().ok_or_else(|| format!("{what} must be a list"))
}

fn as_str<'a>(value: &'a Value, what: &str) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| format!("{what} must be a string"))
}

fn as_bool(value: &Value, what: &str) -> Result<bool, String> {
    value.as_bool().ok_or_else(|| format!("{what} must be a bool"))
}

fn validate(contract: &Value) -> Result<(), String> {
    let root = as_object(contract, "contract")?;

    let expected_oracle = json!({
        "repository": "pi-flake",
        "revision": REVISION,
        "pi_runtime": "0.80.6",
        "root_tree": ROOT_TREE,
        "extensions_tree": EXTENSIONS_TREE,
    });
    if root.get("oracle") != Some(&expected_oracle) {
        return Err(format!(
            "oracle differs: expected {expected_oracle}, got {}",
            root.get("oracle").unwrap_or(&Value::Null)
        ));
    }

    let rows = as_array(root.get("packages").ok_or("packages missing")?, "packages")?;
    let expected_ids: Vec<&str> = PACKAGES.iter().map(|package| package.0).collect();
    let ids: Vec<String> = rows
        .iter()
        .map(|row| as_str(row.get("id").unwrap_or(&Value::Null), "row.id").map(str::to_string))
        .collect::<Result<_, _>>()?;
    if ids != expected_ids {
        return Err(format!(
            "package order/membership differs: expected {expected_ids:?}, got {ids:?}"
        ));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for id in &ids {
        if !seen.insert(id) {
            return Err(format!("duplicate package id {id}"));
        }
    }

    let mut seen_cases: HashSet<String> = HashSet::new();
    let mut seen_kinds: HashSet<&str> = HashSet::new();
    for (row, package) in rows.iter().zip(PACKAGES.iter()) {
        let (ident, directory, package_name, version, entrypoint, tree, default) = *package;
        let row_obj = as_object(row, "row")?;
        let expected_source = json!({
            "directory": directory,
            "package_name": package_name,
            "version": version,
            "entrypoint": entrypoint,
            "tree": tree,
        });
        if row_obj.get("source") != Some(&expected_source) {
            return Err(format!("{ident}: source provenance differs"));
        }
        if as_bool(row_obj.get("default_bundle").unwrap_or(&Value::Null), "default_bundle")? != default {
            return Err(format!("{ident}: default_bundle differs"));
        }
        let load_modes: Vec<Value> = LOAD_MODES
            .iter()
            .map(|mode| Value::String((*mode).to_string()))
            .collect();
        if row_obj.get("load_modes") != Some(&Value::Array(load_modes)) {
            return Err(format!("{ident}: load_modes must be {LOAD_MODES:?}"));
        }
        let cases = as_array(row_obj.get("cases").ok_or("cases missing")?, "cases")?;
        if cases.is_empty() {
            return Err(format!("{ident}: at least one deterministic case is required"));
        }
        let cleanup = as_array(row_obj.get("cleanup").ok_or("cleanup missing")?, "cleanup")?;
        if cleanup.is_empty()
            || cleanup
                .iter()
                .any(|item| !as_str(item, "cleanup item").map(|value| !value.trim().is_empty()).unwrap_or(true))
        {
            return Err(format!("{ident}: explicit lifecycle cleanup assertions are required"));
        }
        for case in cases {
            let case_obj = as_object(case, "case")?;
            let case_id = as_str(case_obj.get("id").unwrap_or(&Value::Null), "case.id")?;
            if !case_id.starts_with(&format!("{ident}.")) {
                return Err(format!("{ident}: invalid case id {case_id:?}"));
            }
            if !seen_cases.insert(case_id.to_string()) {
                return Err(format!("duplicate case id {case_id}"));
            }
            let kinds = as_array(case_obj.get("kinds").ok_or("kinds missing")?, "kinds")?;
            if kinds.is_empty() {
                return Err(format!("{case_id}: invalid fixture kinds {kinds:?}"));
            }
            for kind in kinds {
                let kind = as_str(kind, "kind")?;
                if !FIXTURE_KINDS.contains(&kind) {
                    return Err(format!("{case_id}: invalid fixture kinds {kinds:?}"));
                }
                seen_kinds.insert(kind);
            }
            let script = case_obj.get("script").unwrap_or(&Value::Null);
            if !script.is_object() || script.as_object().is_some_and(|map| map.is_empty()) {
                return Err(format!("{case_id}: scripted inputs are required"));
            }
            let expect = case_obj.get("expect").unwrap_or(&Value::Null);
            if !expect.is_object() || expect.as_object().is_some_and(|map| map.is_empty()) {
                return Err(format!("{case_id}: expected observations are required"));
            }
        }
    }

    if seen_kinds.len() != FIXTURE_KINDS.len() {
        let missing: Vec<&str> = FIXTURE_KINDS
            .iter()
            .copied()
            .filter(|kind| !seen_kinds.contains(kind))
            .collect();
        return Err(format!("fixture kind coverage differs: missing={missing:?}"));
    }
    let bundle = root.get("bundles").unwrap_or(&Value::Null);
    let all_ids: Vec<String> = expected_ids.iter().map(|id| (*id).to_string()).collect();
    let default_ids: Vec<String> = PACKAGES
        .iter()
        .filter(|package| package.6)
        .map(|package| package.0.to_string())
        .collect();
    let expected_bundle = json!({ "all": all_ids, "default": default_ids });
    if bundle != &expected_bundle {
        return Err("bundle composition differs from package declarations".to_string());
    }
    Ok(())
}

fn render(contract: &Value) -> String {
    let root = contract.as_object().expect("contract is an object");
    let mut lines = vec![
        "# Maintained dogfood suite".to_string(),
        String::new(),
        format!("Fixture contract for `pi-flake` `{REVISION}` (Pi 0.80.6)."),
        "The source runtime is an extension-behavior oracle only; pi-rs product parity remains pinned to Pi v0.79.0.".to_string(),
        String::new(),
        "Normal checks consume `tests/dogfood-suite/contract.json` and do not require a sibling checkout.".to_string(),
        "`scripts/dogfood-oracle --source /path/to/pi-flake --check` additionally verifies the pinned source trees and package declarations.".to_string(),
        String::new(),
        "| Package | Upstream source | Version | Default bundle | Fixture cases | Cleanup assertions |".to_string(),
        "|---|---|---:|:---:|---:|---|".to_string(),
    ];
    let rows = root["packages"].as_array().expect("packages is a list");
    for row in rows {
        let cleanup = row["cleanup"]
            .as_array()
            .expect("cleanup is a list")
            .iter()
            .map(|item| item.as_str().expect("cleanup item is a string"))
            .collect::<Vec<_>>()
            .join("; ")
            .replace('|', "\\|");
        let source = row["source"].as_object().expect("source is an object");
        lines.push(format!(
            "| `{}` | `{}/{}` | {} | {} | {} | {} |",
            row["id"].as_str().unwrap_or_default(),
            source["directory"].as_str().unwrap_or_default(),
            source["entrypoint"].as_str().unwrap_or_default(),
            source["version"].as_str().unwrap_or_default(),
            if row["default_bundle"].as_bool().unwrap_or_default() { "yes" } else { "no" },
            row["cases"].as_array().map_or(0, Vec::len),
            cleanup,
        ));
    }
    lines.extend([
        String::new(),
        "## Deterministic fixture coverage".to_string(),
        String::new(),
        "| Case | Kinds | Scripted boundary | Expected observation |".to_string(),
        "|---|---|---|---|".to_string(),
    ]);
    for row in rows {
        for case in row["cases"].as_array().expect("cases is a list") {
            let script = case["script"]
                .as_object()
                .expect("script is an object")
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
                .replace('|', "\\|");
            let expect = case["expect"]
                .as_object()
                .expect("expect is an object")
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
                .replace('|', "\\|");
            let kinds = case["kinds"]
                .as_array()
                .expect("kinds is a list")
                .iter()
                .map(|kind| kind.as_str().expect("kind is a string"))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "| `{}` | {} | {} | {} |",
                case["id"].as_str().unwrap_or_default(),
                kinds,
                script,
                expect,
            ));
        }
    }
    lines.extend([
        String::new(),
        "Load modes pinned for every package: direct file, configured package, and Nix bundle.".to_string(),
        "The default source bundle contains 14 packages and excludes opt-in `morph`; the all-package acceptance bundle contains all 15.".to_string(),
        String::new(),
        "This fixture-only preflight intentionally contains no inert translation packages: executable Lua translations depend on the public lifecycle/module/mechanism work owned by PLAN 9.2–9.9.".to_string(),
        String::new(),
    ]);
    lines.join("\n")
}

#[test]
fn checked_contract_is_valid_and_render_is_idempotent() {
    let contract = load_contract();
    validate(&contract).expect("contract validates");
    let rendered = render(&contract);
    let re_rendered = render(&serde_json::from_str(&serde_json::to_string(&contract).unwrap()).unwrap());
    assert_eq!(rendered, re_rendered);
    let expected = std::fs::read_to_string(repo_root().join("DOGFOOD_SUITE.md")).expect("DOGFOOD_SUITE.md");
    assert_eq!(rendered, expected);
}

fn assert_rejected(mutate: impl FnOnce(&mut Value)) {
    let mut contract = load_contract();
    mutate(&mut contract);
    assert!(validate(&contract).is_err(), "mutation must fail closed");
}

#[test]
fn missing_package_fails_closed() {
    assert_rejected(|value| {
        value["packages"].as_array_mut().unwrap().pop();
    });
}

#[test]
fn stale_source_tree_fails_closed() {
    assert_rejected(|value| {
        value["packages"][0]["source"]["tree"] = Value::String("0".repeat(40));
    });
}

#[test]
fn duplicate_case_fails_closed() {
    assert_rejected(|value| {
        let dup = value["packages"][0]["cases"][0]["id"].clone();
        value["packages"][1]["cases"][0]["id"] = dup;
    });
}

#[test]
fn missing_cleanup_fails_closed() {
    assert_rejected(|value| {
        value["packages"][0]["cleanup"] = Value::Array(vec![]);
    });
}

#[test]
fn missing_fixture_kind_fails_closed() {
    assert_rejected(|value| {
        for package in value["packages"].as_array_mut().unwrap() {
            for case in package["cases"].as_array_mut().unwrap() {
                case["kinds"] = Value::Array(
                    case["kinds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|kind| kind.as_str() != Some("browser_socket"))
                        .cloned()
                        .collect(),
                );
            }
        }
    });
}

#[test]
fn bundle_drift_fails_closed() {
    assert_rejected(|value| {
        value["bundles"]["default"].as_array_mut().unwrap().push(Value::String("morph".to_string()));
    });
}
