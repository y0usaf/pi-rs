//! Offline self-test for the external-extension-inventory workflow (A.3 Rust
//! owner).
//!
//! Reproduces the negative controls of the former
//! `scripts/test-external-extension-inventory` (bash wrapper around the
//! Python tool) against the checked fixtures + provenance + manifest. The
//! capability/private/stale controls test the `extract` + `validate_manifest`
//! layer in memory (no file mutation needed); the tampered control tests the
//! provenance hash layer over a temporary copy. `nix flake check` runs the
//! Rust binary and needs no repo-owned Python/bash.

use std::path::{Path, PathBuf};

use super::external_extension_inventory::{
    extract, load_and_validate_fixtures, validate_manifest,
};

#[derive(Debug, thiserror::Error)]
pub enum SelftestError {
    #[error("external-extension-inventory selftest: {0}")]
    Message(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, SelftestError>;

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(SelftestError::Message(msg.into()))
}

fn make_writable(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            make_writable(&p)?;
        } else {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = p.metadata()?.permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(&p, perms)?;
        }
    }
    Ok(())
}

/// Recursively copy a directory into `dst`.
fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(src)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for entry in entries {
        let rel = entry.strip_prefix(src).map_err(|_| SelftestError::Message("strip_prefix".into()))?;
        let target = dst.join(rel);
        if entry.is_dir() {
            copy_dir(&entry, &target)?;
        } else {
            std::fs::copy(&entry, &target)?;
        }
    }
    Ok(())
}

struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(tag: &str) -> Result<Temp> {
        let dir = std::env::temp_dir().join(format!(
            "pi-rs-external-ext-inventory-selftest-{tag}-{}",
            std::process::id()
        ));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Temp { dir })
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Load provenance + sources and extract capability rows.
fn load_rows(root: &Path) -> Result<super::external_extension_inventory::RowMap> {
    let base = root.join("tests/external-extension-inventory");
    let (_, sources) = load_and_validate_fixtures(&base)
        .map_err(|e| SelftestError::Message(format!("must load fixtures: {e}")))?;
    extract(&sources).map_err(|e| SelftestError::Message(format!("must extract rows: {e}")))
}

fn read_manifest(root: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(root.join("tests/external-extension-inventory/manifest.json"))?;
    serde_json::from_str(&text).map_err(Into::into)
}

/// Assert that `validate_manifest` rejects the rows with a message containing
/// `needle`.
fn assert_rejected(
    rows: &super::external_extension_inventory::RowMap,
    manifest: &serde_json::Value,
    needle: &str,
) -> Result<()> {
    let err = match validate_manifest(rows, manifest) {
        Ok(_) => return fail(format!("expected rejection containing {needle:?}, got success")),
        Err(e) => e.to_string(),
    };
    if !err.contains(needle) {
        return fail(format!("expected rejection to contain {needle:?}, got {err:?}"));
    }
    Ok(())
}

fn run_selftest(root: &Path) -> Result<()> {
    // Positive control: the checked fixtures + provenance + manifest validate.
    let rows = load_rows(root)?;
    let manifest = read_manifest(root)?;
    validate_manifest(&rows, &manifest)
        .map_err(|e| SelftestError::Message(format!("checked manifest must validate: {e}")))?;

    // A source capability added to the pinned fixture cannot bypass
    // classification.
    {
        let mut r = rows.clone();
        r.entry("pi_api".to_owned())
            .or_default()
            .entry("ExtensionAPI.unclassifiedCapability".to_owned())
            .or_default()
            .insert("pi-codex-fast".to_owned());
        assert_rejected(&r, &manifest, "ExtensionAPI.unclassifiedCapability")?;
    }

    // A new concrete Pi class dependency is independently visible and
    // unclassified.
    {
        let mut r = rows.clone();
        r.entry("private_pi".to_owned())
            .or_default()
            .entry("SecretComponent".to_owned())
            .or_default()
            .insert("pi-codex-fast".to_owned());
        // classify the package import so only private_pi remains missing
        r.entry("package_imports".to_owned())
            .or_default()
            .entry("@earendil-works/pi-coding-agent#SecretComponent".to_owned())
            .or_default()
            .insert("pi-codex-fast".to_owned());
        let mut m = manifest.clone();
        m["categories"]["package_imports"][0]["items"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("items".into()))?
            .push(serde_json::json!("@earendil-works/pi-coding-agent#SecretComponent"));
        assert_rejected(&r, &m, "SecretComponent")?;
    }

    // Stale manifest rows fail closed.
    {
        let mut m = manifest.clone();
        m["categories"]["private_pi"][0]["items"]
            .as_array_mut()
            .ok_or_else(|| SelftestError::Message("items".into()))?
            .push(serde_json::json!("RemovedPrivateClass"));
        assert_rejected(&rows, &m, "stale=[\"RemovedPrivateClass\"]")?;
    }

    // Fixture provenance tampering fails closed (hash layer over a temp copy).
    {
        let temp = Temp::new("tampered")?;
        let base_src = root.join("tests/external-extension-inventory");
        copy_dir(&base_src, &temp.dir)?;
        make_writable(&temp.dir)?;
        let fixture = temp
            .dir
            .join("fixtures/pi-codex-fast/src/index.ts");
        let mut source = std::fs::read_to_string(&fixture)?;
        source.push('\n');
        std::fs::write(&fixture, source)?;
        let err = match load_and_validate_fixtures(&temp.dir) {
            Ok(_) => return fail("expected fixture hash rejection, got success"),
            Err(e) => e.to_string(),
        };
        if !err.contains("fixture hash mismatch") {
            return fail(format!("expected fixture hash mismatch, got {err:?}"));
        }
    }
    Ok(())
}

/// The serde-visited positive control plus fail-closed controls from `root`.
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
