//! Deterministic application startup paths and read-only legacy provenance.
//!
//! This module only identifies storage locations. It never creates, parses,
//! copies, renames, chmods, or removes a resource. The fixed counterparts are:
//!
//! | Resource | Canonical XDG entry | Legacy entry under `~/.pi/agent` |
//! |---|---|---|
//! | config | config root / `config.lua` | `settings.json` |
//! | credentials | state root / `credentials.json` | `auth.json` |
//! | sessions | state root / `sessions` | `sessions` |
//! | packages | data root / `packages` | `packages` |
//! | cache | cache root | `cache` |

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde::Serialize;

const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
const XDG_DATA_HOME: &str = "XDG_DATA_HOME";
const XDG_STATE_HOME: &str = "XDG_STATE_HOME";
const XDG_CACHE_HOME: &str = "XDG_CACHE_HOME";
const HOME: &str = "HOME";

/// Environment values used to derive one deterministic startup context.
pub type StartupEnvironment = BTreeMap<OsString, OsString>;

/// An invalid or unavailable environment path required at startup.
#[derive(Debug, thiserror::Error)]
pub enum StartupPathError {
    #[error("{variable} is required and must not be empty")]
    Missing { variable: &'static str },
    #[error("{variable} must be an absolute path, got '{value}'")]
    Relative {
        variable: &'static str,
        value: PathBuf,
    },
    #[error("{variable} is not valid UTF-8 and cannot be exposed to the Lua startup context")]
    NonUnicode { variable: &'static str },
}

/// Canonical XDG roots and the read-only legacy root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupPaths {
    config: PathBuf,
    data: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    legacy: PathBuf,
}

impl StartupPaths {
    /// Derive paths from the current process environment.
    pub fn discover() -> Result<Self, StartupPathError> {
        let environment = [
            HOME,
            XDG_CONFIG_HOME,
            XDG_DATA_HOME,
            XDG_STATE_HOME,
            XDG_CACHE_HOME,
        ]
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect();
        Self::from_environment(&environment)
    }

    /// Derive paths from an explicit environment without consulting process
    /// state. Empty XDG values have the XDG-defined meaning of "unset".
    pub fn from_environment(environment: &StartupEnvironment) -> Result<Self, StartupPathError> {
        let home = required_path(environment, HOME)?;
        let config = xdg_root(environment, XDG_CONFIG_HOME, &home, ".config")?;
        let data = xdg_root(environment, XDG_DATA_HOME, &home, ".local/share")?;
        let state = xdg_root(environment, XDG_STATE_HOME, &home, ".local/state")?;
        let cache = xdg_root(environment, XDG_CACHE_HOME, &home, ".cache")?;
        Ok(Self {
            config,
            data,
            state,
            cache,
            legacy: home.join(".pi/agent"),
        })
    }

    #[must_use]
    pub fn config(&self) -> &Path {
        &self.config
    }

    #[must_use]
    pub fn data(&self) -> &Path {
        &self.data
    }

    #[must_use]
    pub fn state(&self) -> &Path {
        &self.state
    }

    #[must_use]
    pub fn cache(&self) -> &Path {
        &self.cache
    }

    #[must_use]
    pub fn legacy(&self) -> &Path {
        &self.legacy
    }

    #[must_use]
    pub fn canonical_resource(&self, resource: Resource) -> PathBuf {
        match resource {
            Resource::Config => self.config.join("config.lua"),
            Resource::Credentials => self.state.join("credentials.json"),
            Resource::Sessions => self.state.join("sessions"),
            Resource::Packages => self.data.join("packages"),
            Resource::Cache => self.cache.clone(),
        }
    }

    #[must_use]
    pub fn legacy_resource(&self, resource: Resource) -> PathBuf {
        match resource {
            Resource::Config => self.legacy.join("settings.json"),
            Resource::Credentials => self.legacy.join("auth.json"),
            Resource::Sessions => self.legacy.join("sessions"),
            Resource::Packages => self.legacy.join("packages"),
            Resource::Cache => self.legacy.join("cache"),
        }
    }
}

fn required_path(
    environment: &StartupEnvironment,
    variable: &'static str,
) -> Result<PathBuf, StartupPathError> {
    let value = environment
        .get(OsStr::new(variable))
        .filter(|value| !value.is_empty())
        .ok_or(StartupPathError::Missing { variable })?;
    validated_path(variable, value)
}

fn xdg_root(
    environment: &StartupEnvironment,
    variable: &'static str,
    home: &Path,
    default: &str,
) -> Result<PathBuf, StartupPathError> {
    let base = match environment
        .get(OsStr::new(variable))
        .filter(|value| !value.is_empty())
    {
        Some(value) => validated_path(variable, value)?,
        None => home.join(default),
    };
    Ok(base.join("pi"))
}

fn validated_path(variable: &'static str, value: &OsString) -> Result<PathBuf, StartupPathError> {
    let path = PathBuf::from(value);
    if path.to_str().is_none() {
        return Err(StartupPathError::NonUnicode { variable });
    }
    if !path.is_absolute() {
        return Err(StartupPathError::Relative {
            variable,
            value: path,
        });
    }
    Ok(path)
}

/// Storage resources covered by the startup provenance boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Config,
    Credentials,
    Sessions,
    Packages,
    Cache,
}

impl Resource {
    pub const ALL: [Self; 5] = [
        Self::Config,
        Self::Credentials,
        Self::Sessions,
        Self::Packages,
        Self::Cache,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Credentials => "credentials",
            Self::Sessions => "sessions",
            Self::Packages => "packages",
            Self::Cache => "cache",
        }
    }
}

/// Which location supplies an individual resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSource {
    Canonical,
    Legacy,
    Absent,
}

/// Result of probing the selected path without opening its contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Present,
    Inaccessible,
    Absent,
}

/// Per-resource source selection. `destination` is always canonical XDG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceResolution {
    resource: Resource,
    source: ResourceSource,
    selected: Option<PathBuf>,
    destination: PathBuf,
    legacy: PathBuf,
    probe: ProbeOutcome,
    diagnostic: Option<String>,
}

impl ResourceResolution {
    #[must_use]
    pub fn resource(&self) -> Resource {
        self.resource
    }

    #[must_use]
    pub fn source(&self) -> ResourceSource {
        self.source
    }

    #[must_use]
    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    #[must_use]
    pub fn legacy(&self) -> &Path {
        &self.legacy
    }

    #[must_use]
    pub fn probe(&self) -> ProbeOutcome {
        self.probe
    }

    #[must_use]
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    /// Describe an explicit import request without touching either path.
    #[must_use]
    pub fn import_intent(&self) -> ImportIntent {
        let (source, outcome, diagnostic) = match (self.source, self.probe) {
            (ResourceSource::Canonical, _) => (
                self.destination.clone(),
                ImportOutcome::CanonicalPresent,
                "canonical entry already wins; explicit import is a no-op",
            ),
            (ResourceSource::Legacy, ProbeOutcome::Present) => (
                self.legacy.clone(),
                ImportOutcome::Available,
                "legacy source is available for explicit import; no bytes were copied",
            ),
            (ResourceSource::Legacy, ProbeOutcome::Inaccessible) => (
                self.legacy.clone(),
                ImportOutcome::SourceInaccessible,
                "legacy source could not be inspected; no bytes were copied",
            ),
            (ResourceSource::Absent, _) | (ResourceSource::Legacy, ProbeOutcome::Absent) => (
                self.legacy.clone(),
                ImportOutcome::SourceAbsent,
                "legacy source is absent; explicit import is a no-op",
            ),
        };
        ImportIntent {
            resource: self.resource,
            source,
            destination: self.destination.clone(),
            provenance: self.source,
            outcome,
            diagnostic,
        }
    }
}

/// Outcome reported for an explicit, non-executing import intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOutcome {
    Available,
    CanonicalPresent,
    SourceAbsent,
    SourceInaccessible,
}

/// Source/destination/provenance report for explicit import policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportIntent {
    resource: Resource,
    source: PathBuf,
    destination: PathBuf,
    provenance: ResourceSource,
    outcome: ImportOutcome,
    diagnostic: &'static str,
}

impl ImportIntent {
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    #[must_use]
    pub fn provenance(&self) -> ResourceSource {
        self.provenance
    }

    #[must_use]
    pub fn outcome(&self) -> ImportOutcome {
        self.outcome
    }

    #[must_use]
    pub fn diagnostic(&self) -> &'static str {
        self.diagnostic
    }
}

/// Immutable storage data assembled once for application startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupContext {
    paths: StartupPaths,
    resources: Vec<ResourceResolution>,
    imports: Vec<ImportIntent>,
}

impl StartupContext {
    /// Derive and resolve the current process startup context.
    pub fn discover() -> Result<Self, StartupPathError> {
        Ok(Self::from_paths(StartupPaths::discover()?))
    }

    /// Resolve all resources from explicit paths. Resolution performs only
    /// metadata probes and never follows a resource symlink.
    #[must_use]
    pub fn from_paths(paths: StartupPaths) -> Self {
        let resources = Resource::ALL
            .into_iter()
            .map(|resource| resolve_resource(&paths, resource))
            .collect::<Vec<_>>();
        let imports = resources
            .iter()
            .map(ResourceResolution::import_intent)
            .collect();
        Self {
            paths,
            resources,
            imports,
        }
    }

    #[must_use]
    pub fn paths(&self) -> &StartupPaths {
        &self.paths
    }

    #[must_use]
    pub fn resources(&self) -> &[ResourceResolution] {
        &self.resources
    }

    #[must_use]
    pub fn imports(&self) -> &[ImportIntent] {
        &self.imports
    }

    #[must_use]
    pub fn resource(&self, resource: Resource) -> &ResourceResolution {
        &self.resources[resource_index(resource)]
    }
}

fn resource_index(resource: Resource) -> usize {
    match resource {
        Resource::Config => 0,
        Resource::Credentials => 1,
        Resource::Sessions => 2,
        Resource::Packages => 3,
        Resource::Cache => 4,
    }
}

fn resolve_resource(paths: &StartupPaths, resource: Resource) -> ResourceResolution {
    let destination = paths.canonical_resource(resource);
    let legacy = paths.legacy_resource(resource);
    match probe(&destination) {
        Probe::Present => ResourceResolution {
            resource,
            source: ResourceSource::Canonical,
            selected: Some(destination.clone()),
            destination,
            legacy,
            probe: ProbeOutcome::Present,
            diagnostic: None,
        },
        Probe::Inaccessible(diagnostic) => ResourceResolution {
            resource,
            source: ResourceSource::Canonical,
            selected: Some(destination.clone()),
            destination,
            legacy,
            probe: ProbeOutcome::Inaccessible,
            diagnostic: Some(diagnostic),
        },
        Probe::Absent => match probe(&legacy) {
            Probe::Present => ResourceResolution {
                resource,
                source: ResourceSource::Legacy,
                selected: Some(legacy.clone()),
                destination,
                legacy,
                probe: ProbeOutcome::Present,
                diagnostic: None,
            },
            Probe::Inaccessible(diagnostic) => ResourceResolution {
                resource,
                source: ResourceSource::Legacy,
                selected: Some(legacy.clone()),
                destination,
                legacy,
                probe: ProbeOutcome::Inaccessible,
                diagnostic: Some(diagnostic),
            },
            Probe::Absent => ResourceResolution {
                resource,
                source: ResourceSource::Absent,
                selected: None,
                destination,
                legacy,
                probe: ProbeOutcome::Absent,
                diagnostic: None,
            },
        },
    }
}

enum Probe {
    Present,
    Inaccessible(String),
    Absent,
}

fn probe(path: &Path) -> Probe {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Probe::Present,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Probe::Absent,
        Err(error) => Probe::Inaccessible(error.to_string()),
    }
}
