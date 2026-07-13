//! Declarative first-party package manifest.
//!
//! The launcher consumes this data uniformly; package IDs and default activation
//! are policy data rather than hard-coded mode branches. Every source still
//! enters through `Host::load_embedded`, the same transactional load path used
//! by ordinary files.

use std::collections::HashSet;

use pi_rs_host::{EmbeddedPack, Host, LoadReport};

use super::{CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};

#[derive(Debug, Clone, Copy)]
pub struct BuiltinPackage {
    pub id: &'static str,
    pub enabled_by_default: bool,
    pub pack: EmbeddedPack,
}

#[derive(Debug)]
pub struct BuiltinManifest {
    pub packages: &'static [BuiltinPackage],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("unknown builtin package '{0}'")]
    UnknownPackage(String),
    #[error("builtin package '{0}' was suppressed more than once")]
    DuplicateSuppression(String),
}

pub const PACKAGES: &[BuiltinPackage] = &[
    BuiltinPackage {
        id: "agent-policy",
        enabled_by_default: true,
        pack: pi_rs_agent::PACK,
    },
    BuiltinPackage {
        id: "coding-tools",
        enabled_by_default: true,
        pack: TOOLS_PACK,
    },
    BuiltinPackage {
        id: "print-application",
        enabled_by_default: true,
        pack: CODING_AGENT_PACK,
    },
    BuiltinPackage {
        id: "interactive-frontend",
        enabled_by_default: true,
        pack: INTERACTIVE_PACK,
    },
];

pub const DEFAULT_MANIFEST: BuiltinManifest = BuiltinManifest { packages: PACKAGES };

impl BuiltinManifest {
    /// Load default-active packages, minus explicit package IDs.
    ///
    /// Selection is deterministic and fail-closed: unknown or duplicate IDs do
    /// not silently alter the shipped composition.
    pub fn load(&self, host: &Host, suppressed: &[&str]) -> Result<LoadReport, ManifestError> {
        let known = self
            .packages
            .iter()
            .map(|package| package.id)
            .collect::<HashSet<_>>();
        let mut disabled = HashSet::new();
        for id in suppressed {
            if !known.contains(id) {
                return Err(ManifestError::UnknownPackage((*id).to_owned()));
            }
            if !disabled.insert(*id) {
                return Err(ManifestError::DuplicateSuppression((*id).to_owned()));
            }
        }
        let packs = self
            .packages
            .iter()
            .filter(|package| package.enabled_by_default && !disabled.contains(package.id))
            .map(|package| package.pack)
            .collect::<Vec<_>>();
        Ok(host.load_embedded(&packs))
    }

    /// Load the substrate with zero first-party policy packs.
    #[must_use]
    pub fn load_zero(&self, host: &Host) -> LoadReport {
        host.load_embedded(&[])
    }
}
