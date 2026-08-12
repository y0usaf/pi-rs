//! Offline self-test for the model-catalog workflow (A.3 Rust owner).
//!
//! Reproduces the former `scripts/test-model-catalog-update` bash fixture
//! assertions in Rust against the committed fixtures under
//! `tests/model-catalog-update/`. `nix flake check` runs this so the
//! model-catalog workflow needs no Node/Bun runtime.

use std::path::{Path, PathBuf};

use crate::model_catalog::{self, Options};

#[derive(Debug, thiserror::Error)]
pub enum SelftestError {
    #[error("selftest: {0}")]
    Message(String),
    #[error(transparent)]
    ModelCatalog(#[from] model_catalog::ModelCatalogError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn fail<T>(msg: impl Into<String>) -> Result<T, SelftestError> {
    Err(SelftestError::Message(msg.into()))
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Temp, SelftestError> {
        let dir = std::env::temp_dir().join(format!(
            "pi-rs-model-catalog-selftest-{}",
            std::process::id()
        ));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Temp(dir))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source_desc(revision: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "local",
        "revision": revision,
        "path": "tests/model-catalog-update/models.generated.ts",
    })
}

fn run_fixture(root: &Path, tmp: &Path) -> Result<(), SelftestError> {
    let opts = Options {
        source: &root.join("tests/model-catalog-update/models.generated.ts"),
        overrides: &root.join("tests/model-catalog-update/overrides.json"),
        output: &tmp.join("models.json"),
        provenance: &tmp.join("provenance.json"),
        summary_output: Some(&tmp.join("summary.md")),
        revision: "fixture-revision".to_owned(),
        source_desc: source_desc("fixture-revision"),
        remote: false,
    };
    model_catalog::run_normalize(&opts)?;
    Ok(())
}

/// Assert the fixture body once (single run) against the deterministic output.
fn assert_first(_tmp: &Temp) -> Result<(), SelftestError> {
    let tmp = _tmp;
    let models: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tmp.0.join("models.json"))?)?;
    let arr = models
        .as_array()
        .ok_or_else(|| SelftestError::Message("models.json must be an array".into()))?;
    if arr.len() != 2 {
        return fail(format!("expected 2 providers, got {}", arr.len()));
    }
    // First provider is z-provider (source order, preserve_order).
    let p0 = &arr[0];
    if p0["provider"] != serde_json::json!("z-provider") {
        return fail("first provider must be z-provider");
    }
    if p0["models"][0]["thinkingLevelMap"]["max"] != serde_json::json!("max") {
        return fail("thinkingLevelMap.max must be \"max\"");
    }
    if p0["models"][0]["cost"]["tiers"][0]["inputTokensAbove"] != serde_json::json!(500) {
        return fail("tiers[0].inputTokensAbove must be 500");
    }
    let p1 = &arr[1];
    if p1["provider"] != serde_json::json!("a-provider") {
        return fail("second provider must be a-provider");
    }
    if p1["models"][0]["name"] != serde_json::json!("Corrected Name") {
        return fail("override must set name to 'Corrected Name'");
    }
    if p1["models"][0].get("compat").is_some() {
        return fail("override must remove 'compat'");
    }
    Ok(())
}

fn assert_provenance(root: &Path, tmp: &Temp) -> Result<(), SelftestError> {
    let prov: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tmp.0.join("provenance.json"))?)?;
    if prov["schemaVersion"] != serde_json::json!(1) {
        return fail("provenance schemaVersion must be 1");
    }
    if prov["source"]["revision"] != serde_json::json!("fixture-revision") {
        return fail("provenance source.revision must be fixture-revision");
    }
    if prov["overrides"]["count"] != serde_json::json!(1) {
        return fail("provenance overrides.count must be 1");
    }
    // The overrides sha256 must equal the SHA-256 of the fixture overrides file.
    let overrides_bytes = std::fs::read(root.join("tests/model-catalog-update/overrides.json"))?;
    let expected = crate::model_catalog::sha256(&overrides_bytes);
    if prov["overrides"]["sha256"] != serde_json::json!(expected) {
        return fail("provenance overrides.sha256 must match the fixture overrides file");
    }
    if prov["inventory"]["providers"] != serde_json::json!(2) {
        return fail("inventory.providers must be 2");
    }
    if prov["inventory"]["models"] != serde_json::json!(2) {
        return fail("inventory.models must be 2");
    }
    Ok(())
}

fn assert_rejections(root: &Path, tmp: &Temp) -> Result<(), SelftestError> {
    let cases: &[(&str, &str)] = &[
        (
            "unknown-field.generated.ts",
            "unknown field(s): newUpstreamField",
        ),
        ("unsupported-api.generated.ts", "unsupported wire protocol"),
    ];
    for (fixture, needle) in cases {
        let opts = Options {
            source: &root.join(format!("tests/model-catalog-update/{fixture}")),
            overrides: &root.join("scripts/model-catalog-overrides.json"),
            output: &tmp.0.join("rejected.json"),
            provenance: &tmp.0.join("rejected-provenance.json"),
            summary_output: None,
            revision: "fixture-revision".to_owned(),
            source_desc: serde_json::json!({"kind":"local","revision":"fixture-revision","path":fixture}),
            remote: false,
        };
        let err = match model_catalog::run_normalize(&opts) {
            Ok(_) => return fail(format!("expected {fixture} to be rejected")),
            Err(e) => e.to_string(),
        };
        if !err.contains(needle) {
            return fail(format!(
                "{fixture}: expected rejection to contain {needle:?}, got {err:?}"
            ));
        }
    }
    Ok(())
}

/// Run the full offline model-catalog self-test against the repo at `root`.
pub fn run_root(root: &Path) -> Result<(), SelftestError> {
    let tmp = Temp::new()?;

    run_fixture(root, &tmp.0)?;
    let first_models = std::fs::read(tmp.0.join("models.json"))?;
    let first_prov = std::fs::read(tmp.0.join("provenance.json"))?;

    // Idempotency: a second run produces byte-identical outputs.
    run_fixture(root, &tmp.0)?;
    let second_models = std::fs::read(tmp.0.join("models.json"))?;
    let second_prov = std::fs::read(tmp.0.join("provenance.json"))?;
    if first_models != second_models {
        return fail("model-catalog output is not idempotent (models.json differs)");
    }
    if first_prov != second_prov {
        return fail("model-catalog output is not idempotent (provenance.json differs)");
    }

    assert_first(&tmp)?;
    assert_provenance(root, &tmp)?;

    let summary = std::fs::read_to_string(tmp.0.join("summary.md"))?;
    if !summary.contains("providers: 2") {
        return fail("summary.md must mention providers: 2");
    }
    if !summary.contains("models: 2") {
        return fail("summary.md must mention models: 2");
    }

    assert_rejections(root, &tmp)?;
    Ok(())
}

/// Resolve the repository root from CARGO_MANIFEST_DIR and run the self-test.
pub fn run() -> Result<(), SelftestError> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| SelftestError::Message("cannot locate repo root".into()))?;
    run_root(root)
}