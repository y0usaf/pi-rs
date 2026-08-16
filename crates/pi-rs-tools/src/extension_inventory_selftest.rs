//! Offline self-test for the extension-inventory workflow (A.3 Rust owner).
//!
//! Fail-closed controls for the former `scripts/extension-inventory` (which
//! had no separate unittest). The positive control validates the checked
//! surface + manifest against the repo; each negative control mutates the
//! in-memory manifest and asserts `validate` rejects it with a targeted
//! message, so `nix flake check` runs the Rust binary and needs no
//! repo-owned Python.

use std::path::{Path, PathBuf};

use super::extension_inventory::{extract, validate, ExtInvError};

#[derive(Debug, thiserror::Error)]
pub enum SelftestError {
    #[error("extension-inventory selftest: {0}")]
    Message(String),
    #[error(transparent)]
    Inventory(#[from] ExtInvError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, SelftestError>;

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(SelftestError::Message(msg.into()))
}

fn read_manifest(root: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(root.join("tests/extension-inventory/manifest.json"))?;
    serde_json::from_str(&text).map_err(Into::into)
}

/// Assert that `validate` rejects a mutated manifest with a message containing
/// `needle`.
fn assert_rejected(
    root: &Path,
    extracted: &serde_json::Value,
    manifest: &serde_json::Value,
    needle: &str,
) -> Result<()> {
    let err = match validate(root, extracted, manifest) {
        Ok(_) => return fail(format!("expected rejection containing {needle:?}, got success")),
        Err(e) => e.to_string(),
    };
    if !err.contains(needle) {
        return fail(format!("expected rejection to contain {needle:?}, got {err:?}"));
    }
    Ok(())
}

fn run_selftest(root: &Path) -> Result<()> {
    let extracted = extract(root)
        .map_err(|e| SelftestError::Message(format!("must extract: {e}")))?;
    let manifest = read_manifest(root)?;

    // Positive control: the checked surface + manifest validate.
    validate(root, &extracted, &manifest)
        .map_err(|e| SelftestError::Message(format!("checked manifest must validate: {e}")))?;

    // invalid status
    {
        let mut m = manifest.clone();
        m["categories"]["events"][0]["status"] = serde_json::json!("approximately-done");
        assert_rejected(root, &extracted, &m, "invalid status")?;
    }

    // duplicate classification
    {
        let mut m = manifest.clone();
        let dup = m["categories"]["api"][0]["items"][0].clone();
        m["categories"]["api"][1]["items"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("items".into()))?
            .push(dup);
        assert_rejected(root, &extracted, &m, "classified more than once")?;
    }

    // missing classification
    {
        let mut m = manifest.clone();
        m["categories"]["events"][0]["items"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("items".into()))?
            .pop();
        assert_rejected(root, &extracted, &m, "missing=")?;
    }

    // stale classification
    {
        let mut m = manifest.clone();
        m["categories"]["events"][0]["items"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("items".into()))?
            .push(serde_json::json!("event.not_a_real_pi_event"));
        assert_rejected(root, &extracted, &m, "stale=")?;
    }

    // stale translation source
    {
        let mut m = manifest.clone();
        m["translations"][0]["source"] = serde_json::json!("not-a-real-example.ts");
        assert_rejected(root, &extracted, &m, "translations.not-a-real-example.ts: stale source")?;
    }

    // missing translation Lua target
    {
        let mut m = manifest.clone();
        m["translations"][0]["lua"] = serde_json::json!("examples/extensions/missing.lua");
        assert_rejected(root, &extracted, &m, "missing examples/extensions/missing.lua")?;
    }
    Ok(())
}

/// Run the self-test against the repo at `root`.
pub fn run_root(root: &Path) -> Result<()> {
    run_selftest(root)
}

/// Resolve the repository root from CARGO_MANIFEST_DIR and run the self-test.
pub fn run() -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| SelftestError::Message("cannot locate repo root".into()))?;
    run_selftest(root)
}
