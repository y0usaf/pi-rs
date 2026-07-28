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
