//! Shipped Lua product packages.
//!
//! This crate carries no product logic: every shipped default is ordinary
//! Lua under this crate's package directories, loaded through the same public
//! package transaction as a file-backed user package. The Rust side only
//! locates those sources; distribution manifests and package indexes are
//! assembled by the distribution target.

use std::path::{Path, PathBuf};

/// Root directory holding the shipped Lua package trees.
#[must_use]
pub fn package_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The shipped distribution manifest: one declarative index selecting every
/// shipped package file in load order. It is an ordinary versioned launcher
/// manifest — the same one a user may copy, edit, or replace — and its paths
/// resolve relative to its own directory.
#[must_use]
pub fn manifest_path() -> PathBuf {
    package_root().join("default.json")
}
