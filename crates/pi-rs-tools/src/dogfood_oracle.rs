//! A.3 Rust owner for the pinned pi-flake dogfood fixture contract.
//!
//! Faithful port of the former `scripts/dogfood-oracle` (Python). It
//! validates `tests/dogfood-suite/contract.json` against the closed package/
//! fixture-kind/bundle contract and generates/checks `DOGFOOD_SUITE.md`.
//! `--source <pi-flake-checkout>` additionally verifies the pinned git trees
//! and flake declarations. Normal checks consume only checked-in provenance
//! and need no child `git`/Python runtime.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum DogfoodError {
    #[error("dogfood oracle: {0}")]
    Message(String),
    #[error("dogfood oracle: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dogfood oracle: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, DogfoodError>;

pub const REVISION: &str = "94694da7321ce74aa7b82c13db7e60e28c0caba6";
const ROOT_TREE: &str = "d618b6d10b574a8624991b055838e7a16e8c8a35";
const EXTENSIONS_TREE: &str = "c4a04dfe88314b5e48ebb200ccfd546645c3af9e";

pub const FIXTURE_KINDS: &[&str] = &[
    "provider",
    "browser_socket",
    "subprocess",
    "timer",
    "filesystem",
    "compaction",
    "session",
    "terminal",
];
pub const LOAD_MODES: &[&str] = &["direct", "configured", "bundled"];

/// (ident, directory, package_name, version, entrypoint, tree, default)
pub const PACKAGES: &[(&str, &str, &str, &str, &str, &str, bool)] = &[
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

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(DogfoodError::Message(msg.into()))
}

fn git(source: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()
        .map_err(|e| DogfoodError::Message(format!("git -C {} {:?}: {}", source.display(), args, e)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return fail(if stderr.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn validate_source(source: &Path) -> Result<()> {
    if git(source, &["rev-parse", "HEAD"])? != REVISION {
        return fail(format!("source HEAD must be {REVISION}"));
    }
    if git(source, &["rev-parse", "HEAD^{tree}"])? != ROOT_TREE {
        return fail("pinned pi-flake root tree differs");
    }
    if git(source, &["rev-parse", "HEAD:extensions"])? != EXTENSIONS_TREE {
        return fail("pinned pi-flake extensions tree differs");
    }
    let flake = source.join("flake.nix");
    let flake_text = std::fs::read_to_string(&flake)?;
    for (ident, directory, package_name, version, entrypoint, tree, _default) in PACKAGES {
        let actual_tree = git(source, &["rev-parse", &format!("HEAD:extensions/{directory}")])?;
        if actual_tree != *tree {
            return fail(format!("{ident}: source tree differs ({actual_tree})"));
        }
        let package_json_path = source.join("extensions").join(directory).join("package.json");
        let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&package_json_path)?)?;
        let got_name = metadata.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let got_version = metadata.get("version").and_then(|v| v.as_str()).unwrap_or("");
        if got_name != *package_name || got_version != *version {
            return fail(format!("{ident}: package name/version differs"));
        }
        let declared: Vec<String> = metadata
            .get("pi")
            .and_then(|v| v.get("extensions"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.strip_prefix("./").unwrap_or(s).to_owned())
                    .collect()
            })
            .unwrap_or_default();
        if !declared.is_empty() && declared != vec![entrypoint.to_string()] {
            return fail(format!("{ident}: entrypoint differs: {declared:?}"));
        }
        let entry = source.join("extensions").join(directory).join(entrypoint);
        if !entry.is_file() {
            return fail(format!("{ident}: missing entrypoint {entrypoint}"));
        }
        let decl_a = format!("{ident} = self.packages.${{system}}.\"pi-{ident}\";");
        let decl_b = format!("\"{ident}\" = self.packages.${{system}}.\"pi-{ident}\";");
        if !flake_text.contains(&decl_a) && !flake_text.contains(&decl_b) {
            return fail(format!("{ident}: missing extensionPackagesFor declaration"));
        }
    }
    if !flake_text.contains("[\"morph\"]") {
        return fail("source default bundle no longer excludes only morph");
    }
    Ok(())
}

fn as_obj<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| DogfoodError::Message(format!("{what}: expected an object")))
}

fn str_field<'a>(map: &'a serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| DogfoodError::Message(format!("{what}: field {key:?} must be a string")))
}

/// Validate the checked contract against the closed package contract.
pub fn validate_contract(contract: &serde_json::Value) -> Result<()> {
    let oracle = contract
        .get("oracle")
        .and_then(|v| v.as_object())
        .ok_or_else(|| DogfoodError::Message("contract has no oracle object".into()))?;
    let got_oracle = serde_json::json!({
        "repository": str_field(oracle, "repository", "oracle")?,
        "revision": str_field(oracle, "revision", "oracle")?,
        "pi_runtime": str_field(oracle, "pi_runtime", "oracle")?,
        "root_tree": str_field(oracle, "root_tree", "oracle")?,
        "extensions_tree": str_field(oracle, "extensions_tree", "oracle")?,
    });
    let expected_oracle = serde_json::json!({
        "repository": "pi-flake",
        "revision": REVISION,
        "pi_runtime": "0.80.6",
        "root_tree": ROOT_TREE,
        "extensions_tree": EXTENSIONS_TREE,
    });
    if got_oracle != expected_oracle {
        return fail(format!("oracle differs: expected {expected_oracle}, got {got_oracle}"));
    }

    let rows = contract
        .get("packages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| DogfoodError::Message("packages must be a list".into()))?;
    let expected_ids: Vec<&str> = PACKAGES.iter().map(|p| p.0).collect();
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r.get("id").and_then(|v| v.as_str()).unwrap_or(""))
        .collect();
    if ids != expected_ids {
        return fail(format!("package order/membership differs: expected {expected_ids:?}, got {ids:?}"));
    }
    let unique: BTreeSet<&str> = ids.iter().copied().collect();
    if unique.len() != ids.len() {
        return fail("duplicate package id");
    }

    let mut seen_cases: BTreeSet<String> = BTreeSet::new();
    let mut seen_kinds: BTreeSet<&str> = BTreeSet::new();
    for (row, package) in rows.iter().zip(PACKAGES.iter()) {
        let map = as_obj(row, "package row")?;
        let ident = package.0;
        let expected_source = serde_json::json!({
            "directory": package.1,
            "package_name": package.2,
            "version": package.3,
            "entrypoint": package.4,
            "tree": package.5,
        });
        if map.get("source").and_then(|v| v.as_object()).map(|s| serde_json::Value::Object(s.clone())) != Some(expected_source.clone()) {
            // more robust equality: object comparison
            let actual = map.get("source").cloned().unwrap_or(serde_json::Value::Null);
            if actual != expected_source {
                return fail(format!("{ident}: source provenance differs"));
            }
        }
        let default = map.get("default_bundle").and_then(|v| v.as_bool()).unwrap_or(false);
        if default != package.6 {
            return fail(format!("{ident}: default_bundle differs"));
        }
        let load_modes: Vec<&str> = map
            .get("load_modes")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if load_modes != LOAD_MODES {
            return fail(format!("{ident}: load_modes must be {LOAD_MODES:?}"));
        }
        let cases = map
            .get("cases")
            .and_then(|v| v.as_array())
            .ok_or_else(|| DogfoodError::Message(format!("{ident}: at least one deterministic case is required")))?;
        if cases.is_empty() {
            return fail(format!("{ident}: at least one deterministic case is required"));
        }
        let cleanup = map
            .get("cleanup")
            .and_then(|v| v.as_array())
            .ok_or_else(|| DogfoodError::Message(format!("{ident}: explicit lifecycle cleanup assertions are required")))?;
        if cleanup.is_empty() || cleanup.iter().any(|c| !c.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)) {
            return fail(format!("{ident}: explicit lifecycle cleanup assertions are required"));
        }
        for case in cases {
            let cmap = as_obj(case, "case")?;
            let case_id = cmap
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DogfoodError::Message(format!("{ident}: invalid case id")))?;
            if !case_id.starts_with(&format!("{ident}.")) {
                return fail(format!("{ident}: invalid case id {case_id:?}"));
            }
            if !seen_cases.insert(case_id.to_owned()) {
                return fail(format!("duplicate case id {case_id}"));
            }
            let kinds: Vec<&str> = cmap
                .get("kinds")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if kinds.is_empty() || kinds.iter().any(|k| !FIXTURE_KINDS.contains(k)) {
                return fail(format!("{case_id}: invalid fixture kinds {kinds:?}"));
            }
            seen_kinds.extend(kinds);
            let script = cmap.get("script").and_then(|v| v.as_object());
            if !script.map(|s| !s.is_empty()).unwrap_or(false) {
                return fail(format!("{case_id}: scripted inputs are required"));
            }
            let expect = cmap.get("expect").and_then(|v| v.as_object());
            if !expect.map(|s| !s.is_empty()).unwrap_or(false) {
                return fail(format!("{case_id}: expected observations are required"));
            }
        }
    }

    let all: BTreeSet<&str> = seen_kinds.iter().copied().collect();
    let expected_kinds: BTreeSet<&str> = FIXTURE_KINDS.iter().copied().collect();
    if all != expected_kinds {
        let missing: Vec<&str> = expected_kinds.difference(&all).copied().collect();
        return fail(format!(
            "fixture kind coverage differs: missing={missing:?}"
        ));
    }

    let all_ids: Vec<&str> = PACKAGES.iter().map(|p| p.0).collect();
    let default_ids: Vec<&str> = PACKAGES.iter().filter(|p| p.6).map(|p| p.0).collect();
    let expected_bundle = serde_json::json!({
        "all": all_ids,
        "default": default_ids,
    });
    if contract.get("bundles").and_then(|v| v.as_object()).map(|b| serde_json::Value::Object(b.clone())) != Some(expected_bundle.clone()) {
        let actual = contract.get("bundles").cloned().unwrap_or(serde_json::Value::Null);
        if actual != expected_bundle {
            return fail("bundle composition differs from package declarations");
        }
    }
    Ok(())
}

fn esc(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Render the checked contract to `DOGFOOD_SUITE.md` byte-for-byte.
pub fn render_contract(contract: &serde_json::Value) -> Result<String> {
    let rows = contract
        .get("packages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| DogfoodError::Message("packages must be a list".into()))?;
    let mut lines: Vec<String> = vec![
        "# Maintained dogfood suite".to_owned(),
        String::new(),
        format!("Fixture contract for `pi-flake` `{REVISION}` (Pi 0.80.6)."),
        "The source runtime is an extension-behavior oracle only; pi-rs product parity remains pinned to Pi v0.79.0.".to_owned(),
        String::new(),
        "Normal checks consume `tests/dogfood-suite/contract.json` and do not require a sibling checkout.".to_owned(),
        "`scripts/dogfood-oracle --source /path/to/pi-flake --check` additionally verifies the pinned source trees and package declarations.".to_owned(),
        String::new(),
        "| Package | Upstream source | Version | Default bundle | Fixture cases | Cleanup assertions |".to_owned(),
        "|---|---|---:|:---:|---:|---|".to_owned(),
    ];
    for row in rows {
        let map = as_obj(row, "package row")?;
        let id = str_field(map, "id", "package row")?;
        let source = as_obj(map.get("source").ok_or_else(|| DogfoodError::Message("no source".into()))?, "source")?;
        let directory = str_field(source, "directory", "source")?;
        let entrypoint = str_field(source, "entrypoint", "source")?;
        let version = str_field(source, "version", "source")?;
        let default = map.get("default_bundle").and_then(|v| v.as_bool()).unwrap_or(false);
        let cases = map.get("cases").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let cleanup: Vec<String> = map
            .get("cleanup")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str())
                    .map(esc)
                    .collect()
            })
            .unwrap_or_default();
        let cleanup_line = cleanup.join("; ");
        lines.push(format!(
            "| `{id}` | `{directory}/{entrypoint}` | {version} | {} | {cases} | {cleanup_line} |",
            if default { "yes" } else { "no" }
        ));
    }
    lines.push(String::new());
    lines.push("## Deterministic fixture coverage".to_owned());
    lines.push(String::new());
    lines.push("| Case | Kinds | Scripted boundary | Expected observation |".to_owned());
    lines.push("|---|---|---|---|".to_owned());
    for row in rows {
        let map = as_obj(row, "package row")?;
        let empty = Vec::new();
        let cases = map.get("cases").and_then(|v| v.as_array()).unwrap_or(&empty);
        for case in cases {
            let cmap = as_obj(case, "case")?;
            let id = str_field(cmap, "id", "case")?;
            let kinds: Vec<String> = cmap
                .get("kinds")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|k| k.as_str())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let script: Vec<String> = cmap
                .get("script")
                .and_then(|v| v.as_object())
                .map(|o| o.keys().map(|k| esc(k)).collect())
                .unwrap_or_default();
            let expect: Vec<String> = cmap
                .get("expect")
                .and_then(|v| v.as_object())
                .map(|o| o.keys().map(|k| esc(k)).collect())
                .unwrap_or_default();
            lines.push(format!(
                "| `{id}` | {} | {} | {} |",
                kinds.join(", "),
                script.join(", "),
                expect.join(", ")
            ));
        }
    }
    lines.push(String::new());
    lines.push("Load modes pinned for every package: direct file, configured package, and Nix bundle.".to_owned());
    lines.push("The default source bundle contains 14 packages and excludes opt-in `morph`; the all-package acceptance bundle contains all 15.".to_owned());
    lines.push(String::new());
    lines.push("This fixture-only preflight intentionally contains no inert translation packages: executable Lua translations depend on the public lifecycle/module/mechanism work owned by PLAN 9.2–9.9.".to_owned());
    lines.push(String::new());
    Ok(lines.join("\n"))
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let bytes = hasher.finalize();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct Options<'a> {
    pub root: &'a Path,
    pub check: bool,
    pub source: Option<&'a Path>,
    pub self_test: bool,
}

/// Run the dogfood-oracle workflow against the repo at `root`.
pub fn run(opts: &Options) -> Result<()> {
    let contract_path = opts.root.join("tests/dogfood-suite/contract.json");
    let contract: serde_json::Value = serde_json::from_slice(&std::fs::read(&contract_path)?)?;
    validate_contract(&contract)?;
    if let Some(source) = opts.source {
        validate_source(source)?;
    }
    if opts.self_test {
        crate::dogfood_selftest::run_root(opts.root).map_err(|e| DogfoodError::Message(e.to_string()))?;
        println!("dogfood fixture contract fail-closed self-tests passed");
        return Ok(());
    }
    let generated = render_contract(&contract)?;
    let output = opts.root.join("DOGFOOD_SUITE.md");
    if opts.check {
        let current = std::fs::read_to_string(&output).unwrap_or_default();
        if current != generated {
            eprintln!("{} is stale; regenerate", output.display());
            return fail("DOGFOOD_SUITE.md is stale");
        }
        println!("dogfood fixture contract is complete and current");
    } else {
        std::fs::write(&output, &generated)?;
        println!("wrote DOGFOOD_SUITE.md");
    }
    Ok(())
}

/// Convenience: run with `check` semantics from a repo root.
pub fn run_check(root: &Path) -> Result<()> {
    run(&Options { root, check: true, source: None, self_test: false })
}
