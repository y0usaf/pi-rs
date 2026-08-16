//! Offline self-test for the dogfood fixture contract (A.3 Rust owner).
//!
//! Reproduces the negative controls of the former
//! `tests/dogfood-suite/test_contract.py` (Python unittest) against a
//! temporary copy of `tests/dogfood-suite/contract.json`, so `nix flake
//! check` runs the Rust binary and needs no repo-owned Python.

use std::path::{Path, PathBuf};

use super::dogfood_oracle::{validate_contract, DogfoodError};

#[derive(Debug, thiserror::Error)]
pub enum SelftestError {
    #[error("dogfood selftest: {0}")]
    Message(String),
    #[error(transparent)]
    Dogfood(#[from] DogfoodError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, SelftestError>;

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(SelftestError::Message(msg.into()))
}

fn read_contract(root: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(root.join("tests/dogfood-suite/contract.json"))?;
    serde_json::from_str(&text).map_err(Into::into)
}

/// Assert that `validate_contract` rejects a mutated contract with a message
/// containing `needle`.
fn assert_rejected(contract: &serde_json::Value, needle: &str) -> Result<()> {
    let err = match validate_contract(contract) {
        Ok(_) => return fail(format!("expected rejection containing {needle:?}, got success")),
        Err(e) => e.to_string(),
    };
    if !err.contains(needle) {
        return fail(format!("expected rejection to contain {needle:?}, got {err:?}"));
    }
    Ok(())
}

fn run_selftest(root: &Path) -> Result<()> {
    let contract = read_contract(root)?;

    // Positive control: the checked contract validates.
    validate_contract(&contract)
        .map_err(|e| SelftestError::Message(format!("checked contract must validate: {e}")))?;

    // test_missing_package_fails_closed
    {
        let mut c = contract.clone();
        c.get_mut("packages").and_then(|p| p.as_array_mut()).ok_or_else(|| SelftestError::Message("packages".into()))?.pop();
        assert_rejected(&c, "package order/membership differs")?;
    }

    // test_stale_source_tree_fails_closed
    {
        let mut c = contract.clone();
        c["packages"][0]["source"]["tree"] = serde_json::json!("0".repeat(40));
        assert_rejected(&c, "source provenance differs")?;
    }

    // test_duplicate_case_fails_closed
    {
        let mut c = contract.clone();
        let duplicate_id = c["packages"][0]["cases"][0]["id"].clone();
        c["packages"][1]["cases"][0]["id"] = duplicate_id;
        // Mirrors the Python test: a duplicated id is rejected (here by the
        // per-package prefix check before reaching the duplicate-id set).
        assert_rejected(&c, "invalid case id")?;
    }

    // test_missing_cleanup_fails_closed
    {
        let mut c = contract.clone();
        c["packages"][0]["cleanup"] = serde_json::json!([]);
        assert_rejected(&c, "explicit lifecycle cleanup assertions are required")?;
    }

    // test_missing_fixture_kind_fails_closed
    {
        let mut c = contract.clone();
        let packages = c["packages"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("packages array missing".into()))?;
        for package in packages.iter_mut() {
            if let Some(cases) = package.get_mut("cases").and_then(|x| x.as_array_mut()) {
                for case in cases.iter_mut() {
                    if let Some(kinds) = case.get_mut("kinds").and_then(|x| x.as_array_mut()) {
                        kinds.retain(|k| k != "browser_socket");
                    }
                }
            }
        }
        assert_rejected(&c, "fixture kind coverage differs")?;
    }

    // test_bundle_drift_fails_closed
    {
        let mut c = contract.clone();
        c["bundles"]["default"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("default bundle array missing".into()))?
            .push(serde_json::json!("morph"));
        assert_rejected(&c, "bundle composition differs")?;
    }
    Ok(())
}

/// The positive/negative control suite over a temp copy read from `root`.
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
