//! Manifest generation for the A.3 source-language gate.
//!
//! Scans a repository root (via `git ls-files` for tracked files, reading each
//! file's first line for shebang detection) and writes two reviewed JSON
//! manifests:
//!
//! - `allowlist.json`: the total, explicit browser-export JavaScript allowlist
//!   (provenance-marked, standalone-page-only).
//! - `legacy.json`: the grandfathered foreign-language footprint that the
//!   migration is actively porting away from. New files not enumerated here
//!   are rejected by the gate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

use crate::gate::{Language, detect};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("git ls-files failed: {0}")]
    Git(String),
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest write failed for {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// List tracked files relative to `root` using `git ls-files`.
///
/// Falls back to a full recursive directory walk when the root is not inside a
/// git work tree (e.g. the nix store source path, which has no `.git`). The
/// flake source is already clean (filtered), so a walk is equivalent for gating.
pub fn tracked_files(root: &Path) -> Result<Vec<String>, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .output();
    if let Ok(output) = output
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        return Ok(text
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect());
    }
    walk_files(root)
}

/// Recursively enumerate relative file paths under `root`.
fn walk_files(root: &Path) -> Result<Vec<String>, Error> {
    fn visit(dir: &Path, root: &Path, out: &mut Vec<String>) -> Result<(), Error> {
        let mut entries = std::fs::read_dir(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let mut names = Vec::new();
        while let Some(entry) = entries.next().transpose().map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Skip VCS metadata / build output from a walk fallback.
            if matches!(name.as_str(), ".git" | "target" | ".direnv") {
                continue;
            }
            names.push((name, entry.file_type().map_err(|source| Error::Io {
                path: entry.path(),
                source,
            })?));
        }
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, file_type) in names {
            let full = dir.join(&name);
            let rel = full.strip_prefix(root).map_err(|_| Error::Io {
                path: full.clone(),
                source: std::io::Error::other("path escaped root"),
            })?;
            if file_type.is_dir() {
                visit(&full, root, out)?;
            } else {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(root, root, &mut out)?;
    Ok(out)
}

/// Read the first line of a file (for shebang sniffing).
fn first_line(root: &Path, rel: &str) -> Option<String> {
    let path = root.join(rel);
    let bytes = std::fs::read(&path).ok()?;
    let end = bytes.iter().position(|&b| b == b'\n').unwrap_or(bytes.len());
    let line = String::from_utf8_lossy(&bytes[..end]).into_owned();
    Some(line)
}

/// Scan the repo and produce the two manifests.
pub fn scan(root: &Path) -> Result<(Value, Value), Error> {
    let files = tracked_files(root)?;

    let mut js_allowlist: Map<String, Value> = Map::new();
    let mut legacy: Map<String, Value> = Map::new();

    for rel in &files {
        let line = first_line(root, rel);
        let Some(language) = detect(Path::new(rel), line.as_deref()) else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("language".into(), Value::String(language.label().into()));
        if let Some(l) = &line
            && l.trim_start().starts_with("#!")
        {
            entry.insert("shebang".into(), Value::String(l.trim().to_owned()));
        }
        match language {
            Language::JavaScript => {
                js_allowlist.insert(rel.clone(), Value::Object(entry));
            }
            _ => {
                legacy.insert(rel.clone(), Value::Object(entry));
            }
        }
    }

    let allowlist = Value::Object({
        let mut m = Map::new();
        m.insert(
            "schemaVersion".into(),
            Value::String("1".into()),
        );
        m.insert(
            "note".into(),
            Value::String(
                "Browser-export JavaScript allowlist (A.3). Standalone-page-only, \
                 provenance-marked; can never become an extension/package/generator/test/host dependency."
                    .into(),
            ),
        );
        m.insert("files".into(), Value::Object(js_allowlist));
        m
    });

    let legacy_doc = Value::Object({
        let mut m = Map::new();
        m.insert(
            "schemaVersion".into(),
            Value::String("1".into()),
        );
        m.insert(
            "note".into(),
            Value::String(
                "Grandfathered foreign-language footprint (A.3). These files predate the gate and \
                 are being ported to Rust/Lua; new files not enumerated here are rejected."
                    .into(),
            ),
        );
        m.insert("files".into(), Value::Object(legacy));
        m
    });

    Ok((allowlist, legacy_doc))
}

/// Write the two manifests under `tests/source-language/`.
pub fn write_manifests(root: &Path) -> Result<(PathBuf, PathBuf), Error> {
    let (allowlist, legacy) = scan(root)?;
    let dir = root.join("tests/source-language");
    std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;
    let allow_path = dir.join("allowlist.json");
    let legacy_path = dir.join("legacy.json");
    std::fs::write(&allow_path, format!("{}\n", serde_json::to_string_pretty(&allowlist).map_err(|e| Error::Write { path: allow_path.clone(), source: std::io::Error::other(e.to_string()) })?))
        .map_err(|source| Error::Write { path: allow_path.clone(), source })?;
    std::fs::write(&legacy_path, format!("{}\n", serde_json::to_string_pretty(&legacy).map_err(|e| Error::Write { path: legacy_path.clone(), source: std::io::Error::other(e.to_string()) })?))
        .map_err(|source| Error::Write { path: legacy_path.clone(), source })?;
    Ok((allow_path, legacy_path))
}

/// Load the two manifests from `tests/source-language/`.
pub fn load_manifests(root: &Path) -> Result<(BTreeSet<String>, BTreeSet<String>), Error> {
    let dir = root.join("tests/source-language");
    let allow_path = dir.join("allowlist.json");
    let legacy_path = dir.join("legacy.json");
    let allow: Value = serde_json::from_str(
        &std::fs::read_to_string(&allow_path).map_err(|source| Error::Io {
            path: allow_path.clone(),
            source,
        })?,
    )
    .map_err(|source| Error::Json {
        path: allow_path.clone(),
        source,
    })?;
    let legacy: Value = serde_json::from_str(
        &std::fs::read_to_string(&legacy_path).map_err(|source| Error::Io {
            path: legacy_path.clone(),
            source,
        })?,
    )
    .map_err(|source| Error::Json {
        path: legacy_path.clone(),
        source,
    })?;
    let allow_files = allow["files"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let legacy_files = legacy["files"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    Ok((allow_files, legacy_files))
}
