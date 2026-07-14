//! Generic `pi` launcher mechanics.
//!
//! The launcher selects ordinary Lua files, loads them in command-line order
//! through the host's canonical package transaction, then dispatches one
//! immutable startup snapshot to the active `application` root.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};
use serde::{Deserialize, Serialize};

const PACKAGE_MANIFEST_ENV: &str = "PI_PACKAGE_MANIFEST";
const PACKAGE_MANIFEST_VERSION: u32 = 1;

use crate::startup::StartupContext;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub package_root: Option<PathBuf>,
    pub manifest: Option<PathBuf>,
    pub packages: Vec<PathBuf>,
    pub arguments: Vec<String>,
    pub help: bool,
    pub version: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    #[error("{0}")]
    Arguments(String),
    #[error("cannot derive application startup paths: {0}")]
    Startup(#[from] crate::startup::StartupPathError),
    #[error("cannot discover the package root: {0}")]
    CurrentDirectory(std::io::Error),
    #[error("package root '{path}' is unavailable: {source}")]
    PackageRoot {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("package root '{0}' is not a directory")]
    PackageRootNotDirectory(PathBuf),
    #[error("package manifest is absent: '{0}'")]
    ManifestAbsent(PathBuf),
    #[error("package manifest path '{path}' is unavailable: {source}")]
    ManifestPath {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("package manifest is not a regular file: '{0}'")]
    ManifestNotFile(PathBuf),
    #[error("package manifest '{path}' is unreadable: {source}")]
    ManifestUnreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("package manifest '{path}' is invalid: {source}")]
    ManifestInvalid {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("package manifest '{path}' uses version {actual}; expected version {expected}")]
    ManifestVersion {
        path: PathBuf,
        actual: u32,
        expected: u32,
    },
    #[error("package manifest '{0}' selects no packages")]
    ManifestEmpty(PathBuf),
    #[error("package {position} is absent: '{path}'")]
    PackageAbsent { position: usize, path: PathBuf },
    #[error("package {position} path '{path}' is unavailable: {source}")]
    PackagePath {
        position: usize,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("package {position} is unreadable because it is not a regular file: '{path}'")]
    PackageNotFile { position: usize, path: PathBuf },
    #[error("cannot create the Lua host: {0}")]
    Host(pi_rs_host::HostError),
    #[error("package {position} '{path}' is unreadable: {source}")]
    PackageUnreadable {
        position: usize,
        path: PathBuf,
        source: pi_rs_host::HostError,
    },
    #[error("failed to load package {position} '{path}': {source}")]
    PackageLoad {
        position: usize,
        path: PathBuf,
        source: pi_rs_host::HostError,
    },
    #[error("application root dispatch failed: {0}")]
    Dispatch(pi_rs_host::HostError),
    #[error("cannot encode the application result: {0}")]
    Encode(serde_json::Error),
    #[error("cannot write the application result: {0}")]
    Output(std::io::Error),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageManifest {
    version: u32,
    packages: Vec<PathBuf>,
}

struct PackageSelection {
    manifest: Option<PathBuf>,
    packages: Vec<PathBuf>,
}

#[derive(Serialize)]
struct ResultDocument<'a> {
    version: u32,
    generation: u64,
    source: &'a str,
    actions: &'a [pi_rs_host::kernel::Action],
    effects: &'a [pi_rs_host::kernel::Effect],
}

pub fn parse<I>(arguments: I) -> Result<Options, LauncherError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut options = Options::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            options.arguments = arguments
                .map(|value| {
                    value.into_string().map_err(|_| {
                        LauncherError::Arguments(
                            "application arguments must be valid UTF-8".to_owned(),
                        )
                    })
                })
                .collect::<Result<_, _>>()?;
            break;
        }
        if argument == "--help" || argument == "-h" {
            options.help = true;
        } else if argument == "--version" || argument == "-V" {
            options.version = true;
        } else if argument == "--root" {
            let value = arguments.next().ok_or_else(|| {
                LauncherError::Arguments("--root requires a directory".to_owned())
            })?;
            if options.package_root.replace(PathBuf::from(value)).is_some() {
                return Err(LauncherError::Arguments(
                    "--root may be specified only once".to_owned(),
                ));
            }
        } else if argument == "--manifest" {
            let value = arguments.next().ok_or_else(|| {
                LauncherError::Arguments("--manifest requires a JSON file".to_owned())
            })?;
            if options.manifest.replace(PathBuf::from(value)).is_some() {
                return Err(LauncherError::Arguments(
                    "--manifest may be specified only once".to_owned(),
                ));
            }
        } else if argument == "--package" || argument == "-p" {
            let value = arguments.next().ok_or_else(|| {
                LauncherError::Arguments("--package requires a Lua file".to_owned())
            })?;
            options.packages.push(PathBuf::from(value));
        } else {
            return Err(LauncherError::Arguments(format!(
                "unknown option '{}'; application arguments must follow --",
                argument.to_string_lossy()
            )));
        }
    }
    Ok(options)
}

#[must_use]
pub fn help_text() -> &'static str {
    "pi - generic Lua application launcher\n\nUsage:\n  pi [--root DIR] [--manifest FILE] [--package FILE ...] [-- ARG ...]\n\nOptions:\n  --root DIR                           Resolve relative command-line inputs from DIR\n  --manifest FILE                     Load a versioned JSON package manifest\n  --package FILE, -p FILE             Load an ordinary Lua package; repeat in dependency order\n  --help, -h                          Show this help\n  --version, -V                       Show the launcher version\n\nPI_PACKAGE_MANIFEST selects a distribution manifest when --manifest is absent.\nManifest packages resolve from the manifest directory; explicit packages resolve\nfrom --root. With no selection, the raw launcher prints guidance and exits cleanly.\nPackages register roots through pi.roots.v1 and emit queued actions.\n"
}

fn canonical_root(selected: Option<&Path>) -> Result<PathBuf, LauncherError> {
    let selected = match selected {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().map_err(LauncherError::CurrentDirectory)?,
    };
    let root = std::fs::canonicalize(&selected).map_err(|source| LauncherError::PackageRoot {
        path: selected,
        source,
    })?;
    if !root.is_dir() {
        return Err(LauncherError::PackageRootNotDirectory(root));
    }
    Ok(root)
}

fn selected_manifest(options: &Options) -> Option<PathBuf> {
    options
        .manifest
        .clone()
        .or_else(|| std::env::var_os(PACKAGE_MANIFEST_ENV).map(PathBuf::from))
}

fn resolve_package(base: &Path, path: &Path, position: usize) -> Result<PathBuf, LauncherError> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let resolved = std::fs::canonicalize(&joined).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            LauncherError::PackageAbsent {
                position,
                path: joined.clone(),
            }
        } else {
            LauncherError::PackagePath {
                position,
                path: joined.clone(),
                source,
            }
        }
    })?;
    if !resolved.is_file() {
        return Err(LauncherError::PackageNotFile {
            position,
            path: resolved,
        });
    }
    Ok(resolved)
}

fn package_selection(
    root: &Path,
    selected_manifest: Option<&Path>,
    explicit: &[PathBuf],
) -> Result<PackageSelection, LauncherError> {
    let mut selected = Vec::<(PathBuf, PathBuf)>::new();
    let manifest = if let Some(path) = selected_manifest {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let resolved = std::fs::canonicalize(&joined).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                LauncherError::ManifestAbsent(joined.clone())
            } else {
                LauncherError::ManifestPath {
                    path: joined.clone(),
                    source,
                }
            }
        })?;
        if !resolved.is_file() {
            return Err(LauncherError::ManifestNotFile(resolved));
        }
        let bytes =
            std::fs::read(&resolved).map_err(|source| LauncherError::ManifestUnreadable {
                path: resolved.clone(),
                source,
            })?;
        let document: PackageManifest =
            serde_json::from_slice(&bytes).map_err(|source| LauncherError::ManifestInvalid {
                path: resolved.clone(),
                source,
            })?;
        if document.version != PACKAGE_MANIFEST_VERSION {
            return Err(LauncherError::ManifestVersion {
                path: resolved,
                actual: document.version,
                expected: PACKAGE_MANIFEST_VERSION,
            });
        }
        if document.packages.is_empty() {
            return Err(LauncherError::ManifestEmpty(resolved));
        }
        let base = resolved
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        selected.extend(
            document
                .packages
                .into_iter()
                .map(|package| (base.clone(), package)),
        );
        Some(resolved)
    } else {
        None
    };
    selected.extend(
        explicit
            .iter()
            .cloned()
            .map(|package| (root.to_path_buf(), package)),
    );
    let packages = selected
        .iter()
        .enumerate()
        .map(|(index, (base, package))| resolve_package(base, package, index + 1))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PackageSelection { manifest, packages })
}

fn dispatch(options: &Options, manifest: Option<&Path>) -> Result<DispatchBatch, LauncherError> {
    let startup = StartupContext::discover()?;
    let root = canonical_root(options.package_root.as_deref())?;
    let selection = package_selection(&root, manifest, &options.packages)?;
    let host = Host::new(HostConfig {
        cwd: Some(root.to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .map_err(LauncherError::Host)?;

    for (index, path) in selection.packages.iter().enumerate() {
        host.load_package(PackageSource::File { path })
            .map_err(|source| match source {
                source @ pi_rs_host::HostError::Io(_) => LauncherError::PackageUnreadable {
                    position: index + 1,
                    path: path.clone(),
                    source,
                },
                source => LauncherError::PackageLoad {
                    position: index + 1,
                    path: path.clone(),
                    source,
                },
            })?;
    }

    let event = serde_json::json!({
        "kind": "startup",
        "arguments": options.arguments,
    });
    let context = serde_json::json!({
        "root": root.to_string_lossy(),
        "manifest": selection.manifest.as_ref().map(|path| path.to_string_lossy()),
        "packages": selection.packages
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>(),
        "storage": startup,
    });
    host.dispatch(DispatchRequest::new(RootKind::Application, event, context))
        .map_err(LauncherError::Dispatch)
}

#[must_use]
pub fn no_selection_text() -> &'static str {
    "pi: no application package selected; pass --package FILE or --manifest FILE (see pi --help).\n"
}

pub fn run(options: &Options, output: &mut dyn std::io::Write) -> Result<(), LauncherError> {
    let manifest = selected_manifest(options);
    if manifest.is_none() && options.packages.is_empty() {
        let _ = StartupContext::discover()?;
        return output
            .write_all(no_selection_text().as_bytes())
            .map_err(LauncherError::Output);
    }
    let batch = dispatch(options, manifest.as_deref())?;
    let document = ResultDocument {
        version: batch.version,
        generation: batch.generation.get(),
        source: &batch.source,
        actions: &batch.actions,
        effects: &batch.effects,
    };
    serde_json::to_writer(&mut *output, &document).map_err(LauncherError::Encode)?;
    output.write_all(b"\n").map_err(LauncherError::Output)
}

pub fn write_help(output: &mut dyn std::io::Write) -> Result<(), LauncherError> {
    output
        .write_all(help_text().as_bytes())
        .map_err(LauncherError::Output)
}
