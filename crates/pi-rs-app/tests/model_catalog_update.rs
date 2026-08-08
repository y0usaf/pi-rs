//! Fixture-backed normalization and rejection tests for the reviewed
//! model-catalog update path (Rust port of the deleted
//! scripts/test-model-catalog-update; PLAN A.3 — the model-catalog workflow
//! is owned by Rust). Runs the update-model-catalog binary against the
//! checked tests/model-catalog-update fixtures and pins idempotency,
//! normalization, override application, and fail-closed rejection.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_update-model-catalog")
}

fn fixture_dir() -> PathBuf {
    repo_root().join("tests/model-catalog-update")
}

fn run_updater(
    args: &[&str],
    cwd: &Path,
) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn update-model-catalog")
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn fixture_run_is_idempotent_and_normalizes() {
    let tmp = tmp_dir();
    let out = tmp.path();
    let models = out.join("models.json");
    let provenance = out.join("provenance.json");
    let summary = out.join("summary.md");
    let models_fixture = fixture_dir().join("models.generated.ts");
    let overrides_fixture = fixture_dir().join("overrides.json");
    let models_arg = models_fixture.to_str().unwrap();
    let overrides_arg = overrides_fixture.to_str().unwrap();
    let base_args: Vec<&str> = vec![
        "--source",
        models_arg,
        "--revision",
        "fixture-revision",
        "--source-path",
        "tests/model-catalog-update/models.generated.ts",
        "--overrides",
        overrides_arg,
    ];
    let run = |models_arg: &str, prov_arg: &str, sum_arg: &str| {
        let output = run_updater(
            &[
                &base_args[..],
                &["--output", models_arg, "--provenance", prov_arg, "--summary-output", sum_arg],
            ]
            .concat(),
            &repo_root(),
        );
        assert!(output.status.success(), "updater failed: {}", String::from_utf8_lossy(&output.stderr));
        output
    };
    let first = run(models.to_str().unwrap(), provenance.to_str().unwrap(), summary.to_str().unwrap());
    let _second = run(models.to_str().unwrap(), provenance.to_str().unwrap(), summary.to_str().unwrap());
    let first_models = std::fs::read(&models).unwrap();
    let first_prov = std::fs::read(&provenance).unwrap();
    let second_models = std::fs::read(&models).unwrap();
    let second_prov = std::fs::read(&provenance).unwrap();
    assert_eq!(first_models, second_models, "models.json must be byte-idempotent");
    assert_eq!(first_prov, second_prov, "provenance.json must be byte-idempotent");

    let catalog: serde_json::Value =
        serde_json::from_slice(&first_models).expect("models.json parses");
    let rows = catalog.as_array().expect("catalog is a list");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["provider"], "z-provider");
    assert_eq!(rows[0]["models"][0]["thinkingLevelMap"]["max"], "max");
    assert_eq!(rows[0]["models"][0]["cost"]["tiers"][0]["inputTokensAbove"], 500);
    assert_eq!(rows[1]["provider"], "a-provider");
    assert_eq!(rows[1]["models"][0]["name"], "Corrected Name");
    assert!(rows[1]["models"][0].get("compat").is_none(), "override must remove compat");

    let prov: serde_json::Value =
        serde_json::from_slice(&first_prov).expect("provenance.json parses");
    assert_eq!(prov["schemaVersion"], 1);
    assert_eq!(prov["source"]["revision"], "fixture-revision");
    assert_eq!(prov["overrides"]["count"], 1);
    assert_eq!(prov["inventory"]["providers"], 2);
    assert_eq!(prov["inventory"]["models"], 2);

    let summary_text = std::fs::read_to_string(&summary).unwrap();
    assert!(summary_text.contains("providers: 2"), "summary shows providers");
    assert!(summary_text.contains("models: 2"), "summary shows models");
    assert!(String::from_utf8_lossy(&first.stdout).contains("providers: 2"));
}

fn expect_rejection(fixture: &str, message: &str) {
    let tmp = tmp_dir();
    let out = tmp.path();
    let output = run_updater(
        &[
            "--source",
            fixture_dir().join(fixture).to_str().unwrap(),
            "--overrides",
            repo_root().join("scripts/model-catalog-overrides.json").to_str().unwrap(),
            "--output",
            out.join("rejected.json").to_str().unwrap(),
            "--provenance",
            out.join("rejected-provenance.json").to_str().unwrap(),
        ],
        &repo_root(),
    );
    assert!(!output.status.success(), "{fixture} must be rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(message), "expected {message:?} in stderr, got: {stderr}");
}

#[test]
fn unknown_field_is_rejected() {
    expect_rejection("unknown-field.generated.ts", "unknown field(s): newUpstreamField");
}

#[test]
fn unsupported_api_is_rejected() {
    expect_rejection("unsupported-api.generated.ts", "unsupported wire protocol");
}
