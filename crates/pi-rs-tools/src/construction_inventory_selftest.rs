//! Offline self-test for the construction-inventory workflow (A.3 Rust owner).
//!
//! Reproduces the negative controls of the former
//! `tests/construction-inventory/test_checker.py` (unittest) in Rust against a
//! temporary copy of the affected files, so `nix flake check` runs the Rust
//! binary and needs no repo-owned Python.

use std::path::{Path, PathBuf};

use super::construction_inventory::{run as inventory_run, InventoryError};

#[derive(Debug, thiserror::Error)]
pub enum SelftestError {
    #[error("construction-inventory selftest: {0}")]
    Message(String),
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, SelftestError>;

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(SelftestError::Message(msg.into()))
}

/// Recursively copy a directory into `dst` (both files and empty subdirs).
fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(src)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for entry in entries {
        let rel = entry.strip_prefix(src).map_err(|_| SelftestError::Message("strip_prefix".into()))?;
        let target = dst.join(rel);
        let ft = entry.metadata()?;
        if ft.is_dir() {
            copy_dir(&entry, &target)?;
        } else {
            std::fs::copy(&entry, &target)?;
        }
    }
    Ok(())
}

/// A temporary working tree containing the files the checker reads, mirroring
/// the Python test's setUp. The whole subtree is writable.
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new() -> Result<Temp> {
        let dir = std::env::temp_dir().join(format!(
            "pi-rs-construction-inventory-selftest-{}",
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

/// Populate the temp tree from `root` (the workspace) with the file set the
/// checker reads. Mirrors `tests/construction-inventory/test_checker.py::setUp`.
fn populate(root: &Path, temp: &Path) -> Result<()> {
    let rels = [
        "PLAN.md",
        "CONSTRUCTION_INVENTORY.md",
        "tests/construction-inventory/provenance.json",
        "tests/construction-inventory/manifest.json",
        "crates/pi-rs-agent/src/lib.rs",
        "crates/pi-rs-agent/lua",
        "crates/pi-rs-app/src",
        "crates/pi-rs-host/src/lib.rs",
    ];
    for rel in rels {
        let src = root.join(rel);
        let dst = temp.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let ft = src.metadata()?;
        if ft.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &dst)?;
        }
    }
    // Make everything writable (nix sandbox store paths are read-only).
    make_writable(temp)?;
    Ok(())
}

fn make_writable(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        let ft = entry.metadata()?;
        if ft.is_dir() {
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

/// Run `construction-inventory --check` in the temp tree; assert it fails with a
/// rejection matching `needle`.
fn assert_rejected(root: &Path, needle: &str) -> Result<()> {
    let err = match inventory_run(root, true, false) {
        Ok(_) => return fail(format!("expected rejection containing {needle:?}, got success")),
        Err(e) => e.to_string(),
    };
    if !err.contains(needle) {
        return fail(format!("expected rejection to contain {needle:?}, got {err:?}"));
    }
    Ok(())
}

/// Rewrite a JSON value in the temp manifest and write it back (compact).
fn write_manifest(root: &Path, manifest: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(manifest)? + "\n";
    std::fs::write(root.join("tests/construction-inventory/manifest.json"), text)?;
    Ok(())
}

fn read_manifest(root: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(root.join("tests/construction-inventory/manifest.json"))?;
    serde_json::from_str(&text).map_err(Into::into)
}

fn row_mut<'a>(manifest: &'a mut serde_json::Value, id: &str) -> Result<&'a mut serde_json::Value> {
    let rows = manifest
        .get_mut("rows")
        .and_then(|r| r.as_array_mut())
        .ok_or_else(|| SelftestError::Message("no rows".into()))?;
    rows.iter_mut()
        .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(id))
        .ok_or_else(|| SelftestError::Message(format!("no row {id}")))
}

fn run_selftest(root: &Path) -> Result<()> {
    // Positive control: fresh copy passes --check and generation is idempotent.
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        inventory_run(&temp.dir, true, false).map_err(|e| SelftestError::Message(format!("fresh check failed: {e}")))?;
        inventory_run(&temp.dir, false, false).map_err(|e| SelftestError::Message(format!("generate failed: {e}")))?;
        inventory_run(&temp.dir, true, false).map_err(|e| SelftestError::Message(format!("regen check failed: {e}")))?;
    }

    // unclassified embedded source
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let new_policy = temp.dir.join("crates/pi-rs-app/src/builtins/tools/new-policy.lua");
        std::fs::write(&new_policy, "local new_policy = true\n")?;
        let descriptor = temp.dir.join("crates/pi-rs-app/src/builtins/mod.rs");
        let source = std::fs::read_to_string(&descriptor)?;
        let replaced = source.replacen(
            "include_str!(\"tools/prelude.lua\"),",
            "include_str!(\"tools/new-policy.lua\"),\n        include_str!(\"tools/prelude.lua\"),",
            1,
        );
        std::fs::write(&descriptor, replaced)?;
        assert_rejected(&temp.dir, "embedded source coverage differs")?;
    }

    // unclassified public declaration
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let frontend = temp.dir.join("crates/pi-rs-app/src/builtins/interactive.lua");
        let mut source = std::fs::read_to_string(&frontend)?;
        source.push_str("\npi.register_command(\"unclassified-policy\", { handler = function() end })\n");
        std::fs::write(&frontend, source)?;
        assert_rejected(&temp.dir, "embedded declarations differ")?;
    }

    // duplicate declaration owner
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let mut manifest = read_manifest(&temp.dir)?;
        let bash = row_mut(&mut manifest, "tool.bash")?;
        bash.get_mut("declarations")
            .and_then(|d| d.as_array_mut())
            .ok_or_else(|| SelftestError::Message("declarations".into()))?
            .push(serde_json::json!("tool:read"));
        write_manifest(&temp.dir, &manifest)?;
        assert_rejected(&temp.dir, "duplicates=")?;
    }

    // stale source row
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let mut manifest = read_manifest(&temp.dir)?;
        let read = row_mut(&mut manifest, "tool.read")?;
        read["coverage"] = serde_json::json!(["crates/pi-rs-app/src/builtins/tools/removed.lua"]);
        write_manifest(&temp.dir, &manifest)?;
        assert_rejected(&temp.dir, "stale=")?;
    }

    // hardcoded product entrypoint
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let mut main = std::fs::read_to_string(temp.dir.join("crates/pi-rs-app/src/main.rs"))?;
        main.push_str("\nconst BAD: &str = \"pi-rs-run\";\n");
        std::fs::write(temp.dir.join("crates/pi-rs-app/src/main.rs"), main)?;
        assert_rejected(&temp.dir, "hardcoded Rust product entrypoints differ")?;
    }

    // stale Rust seam
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let mut main = std::fs::read_to_string(temp.dir.join("crates/pi-rs-app/src/main.rs"))?;
        main = main.replacen(
            "let role = decl_registry.select_frontend(interactive)",
            "let selected_role = decl_registry.select_frontend(interactive)",
            1,
        );
        std::fs::write(temp.dir.join("crates/pi-rs-app/src/main.rs"), main)?;
        assert_rejected(&temp.dir, "stale anchor")?;
    }

    // unclassified Rust launch call
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let mut main = std::fs::read_to_string(temp.dir.join("crates/pi-rs-app/src/main.rs"))?;
        main.push_str("\n// inventory negative control: host.call_command(name, args);\n");
        std::fs::write(temp.dir.join("crates/pi-rs-app/src/main.rs"), main)?;
        assert_rejected(&temp.dir, "Rust launch/composition calls differ")?;
    }

    // missing named open row
    {
        let temp = Temp::new()?;
        populate(root, &temp.dir)?;
        let mut manifest = read_manifest(&temp.dir)?;
        let rows = manifest
            .get_mut("rows")
            .and_then(|r| r.as_array_mut())
            .ok_or_else(|| SelftestError::Message("no rows".into()))?;
        rows.retain(|r| r.get("id").and_then(|v| v.as_str()) != Some("modules.chunk-local-helpers"));
        write_manifest(&temp.dir, &manifest)?;
        assert_rejected(&temp.dir, "missing named open rows")?;
    }
    Ok(())
}

/// The serde-visited positive control: generation+check on a clean copy.
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