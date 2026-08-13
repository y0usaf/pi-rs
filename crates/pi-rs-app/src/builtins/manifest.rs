//! Declarative first-party package manifest.
//!
//! The launcher consumes this data uniformly; package IDs and default activation
//! are policy data rather than hard-coded mode branches. Every source still
//! enters through `Host::load_embedded`, the same transactional load path used
//! by ordinary files.

use std::collections::HashSet;

use pi_rs_host::{EmbeddedPack, Host, LoadReport};

use super::{AGENT_CORE_PACK, CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};

#[derive(Debug, Clone, Copy)]
pub struct BuiltinPackage {
    pub id: &'static str,
    pub enabled_by_default: bool,
    /// `true` for mechanism substrate that other product packs depend on
    /// (e.g. `agent-core`, which owns the `pi.agent.*` shared modules).
    /// Core packs are always loaded and cannot be suppressed; `load_zero`
    /// still loads nothing (zero *policy* packs).
    pub core: bool,
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
    #[error("failed to suppress tool '{0}': {1}")]
    ToolSuppression(String, String),
}

pub const PACKAGES: &[BuiltinPackage] = &[
    BuiltinPackage {
        id: "agent-core",
        enabled_by_default: true,
        core: true,
        pack: AGENT_CORE_PACK,
    },
    BuiltinPackage {
        id: "agent-policy",
        enabled_by_default: true,
        core: false,
        pack: pi_rs_agent::PACK,
    },
    BuiltinPackage {
        id: "coding-tools",
        enabled_by_default: true,
        core: false,
        pack: TOOLS_PACK,
    },
    BuiltinPackage {
        id: "print-application",
        enabled_by_default: true,
        core: false,
        pack: CODING_AGENT_PACK,
    },
    BuiltinPackage {
        id: "interactive-frontend",
        enabled_by_default: true,
        core: false,
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
        self.load_with_suppressed_tools(host, suppressed, &[])
    }

    /// Like [`Self::load`], but additionally unregister the named tools from
    /// their owning (first-registering) extension after load. This is the
    /// declarative per-tool ablation seam (PLAN 9.10): a default builtin tool
    /// can be suppressed while the rest of its pack stays active, and an
    /// ordinary file-backed package loaded afterwards claims the name.
    ///
    /// Core packs (`package.core`) are always kept: they are mechanism
    /// substrate the remaining policy packs depend on, so suppressing one
    /// would break the very policies still being loaded.
    pub fn load_with_suppressed_tools(
        &self,
        host: &Host,
        suppressed: &[&str],
        suppressed_tools: &[&str],
    ) -> Result<LoadReport, ManifestError> {
        // Core substrate cannot be suppressed: it is not a product policy
        // unit, and suppressing it would break every dependent pack.
        for id in suppressed {
            if self.packages.iter().any(|p| p.core && p.id == *id) {
                return Err(ManifestError::UnknownPackage(format!(
                    "core substrate '{id}' cannot be suppressed"
                )));
            }
        }
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
            .filter(|package| {
                package.core || (package.enabled_by_default && !disabled.contains(package.id))
            })
            .map(|package| package.pack)
            .collect::<Vec<_>>();
        let report = host.load_embedded(&packs);
        for tool in suppressed_tools {
            host.unregister_tool(tool).map_err(|error| {
                ManifestError::ToolSuppression((*tool).to_owned(), error.to_string())
            })?;
        }
        Ok(report)
    }

    /// Load the substrate with zero first-party policy packs. Core mechanism
    /// substrate is also omitted here (zero *policy* packs and zero embedded
    /// declarations); a bare host exposes only the Rust mechanism API.
    #[must_use]
    pub fn load_zero(&self, host: &Host) -> LoadReport {
        host.load_embedded(&[])
    }
}
