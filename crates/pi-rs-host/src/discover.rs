//! Extension discovery — port of the discovery half of the spec's
//! `core/extensions/loader.ts` (`discoverExtensionsInDir`,
//! `resolveExtensionEntries`, and the path assembly of
//! `discoverAndLoadExtensions`). Loading itself is
//! [`Host::load_extensions`](crate::Host::load_extensions).
//!
//! Divergence 1 mappings: extension files are `.lua` (spec: `.ts`/`.js`);
//! a directory's entry point is `init.lua` (spec: `index.ts`/`index.js`).
//! The spec's third entry-point rule — a `package.json` carrying a `pi`
//! manifest — is the npm distribution surface; its Lua analogue is the
//! WS5 package-manager decision (locked row: "decide when the WS5 port
//! reaches `package-manager.ts`") and lands there.
//!
//! One deliberate tightening over the spec: directory listings are sorted
//! by name (Node's `readdirSync` order is filesystem-dependent), so
//! discovery order is deterministic.
//!
//! Trust gate (spec: `resource-loader.ts` keeps project-local extensions
//! out of the untrusted pass): `discover_extension_paths` takes
//! `project_trusted` and skips `cwd/.pi/extensions` when false.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::paths;

/// Project config directory name (spec: `CONFIG_DIR_NAME`, `.pi`).
pub const CONFIG_DIR_NAME: &str = ".pi";

/// Environment override for the agent config directory (spec:
/// `ENV_AGENT_DIR`, `PI_CODING_AGENT_DIR`).
pub const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";

/// The agent config directory (spec: `getAgentDir()` —
/// `~/.pi/agent`, overridable via [`ENV_AGENT_DIR`]).
pub fn agent_dir() -> String {
    if let Ok(dir) = std::env::var(ENV_AGENT_DIR)
        && !dir.is_empty()
    {
        return paths::normalize_path(&dir, false);
    }
    crate::os::join(&[
        paths::home_dir(),
        CONFIG_DIR_NAME.to_owned(),
        "agent".to_owned(),
    ])
}

/// Spec `isExtensionFile`: `.ts`/`.js` → `.lua` (divergence 1).
fn is_extension_file(name: &str) -> bool {
    name.ends_with(".lua")
}

/// Spec `resolveExtensionEntries`: entry points of a directory. The
/// manifest rule is deferred to WS5 (module docs); the index rule maps to
/// `init.lua`.
fn resolve_extension_entries(dir: &Path) -> Option<Vec<PathBuf>> {
    let init = dir.join("init.lua");
    if init.exists() {
        return Some(vec![init]);
    }
    None
}

/// Spec `discoverExtensionsInDir`: direct `.lua` files and
/// one-level subdirectories with an entry point. No recursion beyond one
/// level; a missing or unreadable directory yields nothing.
pub fn discover_extensions_in_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sorted: Vec<std::fs::DirEntry> = entries.flatten().collect();
    sorted.sort_by_key(std::fs::DirEntry::file_name);

    let mut discovered = Vec::new();
    for entry in sorted {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let entry_path = entry.path();

        // 1. Direct files (symlinks with an extension name count as files,
        //    per the spec's dirent check).
        if (file_type.is_file() || file_type.is_symlink()) && is_extension_file(&name) {
            discovered.push(entry_path);
            continue;
        }

        // 2. Subdirectories with an entry point.
        if (file_type.is_dir() || file_type.is_symlink())
            && let Some(found) = resolve_extension_entries(&entry_path)
        {
            discovered.extend(found);
        }
    }
    discovered
}

/// Resolve explicitly configured extension paths relative to `cwd`. A directory
/// resolves to `init.lua` when present, otherwise its direct extension entries;
/// a non-directory is retained so loading can report a missing-path error.
pub fn resolve_explicit_extension_paths(paths: &[String], cwd: &str) -> Vec<String> {
    let process_cwd = paths::process_cwd();
    let resolved_cwd = paths::resolve_path(cwd, &process_cwd, false);
    let mut found = Vec::new();
    for configured in paths {
        let resolved = paths::resolve_path(configured, &resolved_cwd, true);
        let path = Path::new(&resolved);
        if path.is_dir() {
            if let Some(entries) = resolve_extension_entries(path) {
                found.extend(entries);
            } else {
                found.extend(discover_extensions_in_dir(path));
            }
        } else {
            found.push(PathBuf::from(resolved));
        }
    }
    dedupe(found)
}

fn dedupe(found: Vec<PathBuf>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in found {
        let path = path.to_string_lossy().into_owned();
        if seen.insert(path.clone()) {
            result.push(path);
        }
    }
    result
}

/// Product extension path assembly. Pi gives explicit CLI extensions highest
/// precedence, then project-local, global, and configured sources. `--no-extensions`
/// disables discovery/configuration but deliberately retains CLI paths.
pub fn product_extension_paths(
    configured_paths: &[String],
    cli_paths: &[String],
    cwd: &str,
    agent_dir: &str,
    project_trusted: bool,
    no_extensions: bool,
) -> Vec<String> {
    let mut paths = resolve_explicit_extension_paths(cli_paths, cwd)
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if !no_extensions {
        let process_cwd = paths::process_cwd();
        let resolved_cwd = paths::resolve_path(cwd, &process_cwd, false);
        let resolved_agent_dir = paths::resolve_path(agent_dir, &process_cwd, false);
        if project_trusted {
            paths.extend(discover_extensions_in_dir(
                &Path::new(&resolved_cwd)
                    .join(CONFIG_DIR_NAME)
                    .join("extensions"),
            ));
        }
        paths.extend(discover_extensions_in_dir(
            &Path::new(&resolved_agent_dir).join("extensions"),
        ));
        paths.extend(
            resolve_explicit_extension_paths(configured_paths, &resolved_cwd)
                .into_iter()
                .map(PathBuf::from),
        );
    }
    dedupe(paths)
}

/// Backwards-compatible discovery helper used by the host-level tests:
/// project-local → global → configured, with first-path deduplication.
pub fn discover_extension_paths(
    configured_paths: &[String],
    cwd: &str,
    agent_dir: &str,
    project_trusted: bool,
) -> Vec<String> {
    product_extension_paths(
        configured_paths,
        &[],
        cwd,
        agent_dir,
        project_trusted,
        false,
    )
}
