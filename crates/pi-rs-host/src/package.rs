//! Source-neutral package inputs.
//!
//! Provenance chooses only how bytes are obtained and attributed. Every source
//! is sent through the same VM load transaction and receives the same API,
//! watchdog, scope, publication, and disposal behavior.

use std::path::Path;

use crate::HostError;

#[derive(Debug, Clone, Copy)]
pub enum PackageSource<'a> {
    Embedded { name: &'a str, source: &'a str },
    File { path: &'a Path },
    Memory { key: &'a str, source: &'a str },
}

#[derive(Debug)]
pub(crate) struct ResolvedPackage {
    pub(crate) source_key: String,
    pub(crate) source: String,
}

impl PackageSource<'_> {
    pub(crate) fn resolve(self) -> Result<ResolvedPackage, HostError> {
        match self {
            Self::Embedded { name, source } => Ok(ResolvedPackage {
                source_key: format!("<{name}>"),
                source: source.to_owned(),
            }),
            Self::File { path } => {
                let source_key = path.to_string_lossy().into_owned();
                let source = std::fs::read_to_string(path)
                    .map_err(|error| HostError::Io(format!("read '{source_key}': {error}")))?;
                Ok(ResolvedPackage { source_key, source })
            }
            Self::Memory { key, source } => Ok(ResolvedPackage {
                source_key: key.to_owned(),
                source: source.to_owned(),
            }),
        }
    }
}
