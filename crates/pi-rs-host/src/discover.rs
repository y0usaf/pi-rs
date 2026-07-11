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

/// Spec `discoverAndLoadExtensions` path assembly: project-local
/// `cwd/.pi/extensions` (gated by `project_trusted`), then global
/// `agent_dir/extensions`, then explicitly configured paths (a directory
/// resolves its entry points or falls back to per-file discovery; anything
/// else is taken as a file path). Deduplicated on the resolved path,
/// first occurrence wins.
pub fn discover_extension_paths(
    configured_paths: &[String],
    cwd: &str,
    agent_dir: &str,
    project_trusted: bool,
) -> Vec<String> {
    let process_cwd = paths::process_cwd();
    let resolved_cwd = paths::resolve_path(cwd, &process_cwd, false);
    let resolved_agent_dir = paths::resolve_path(agent_dir, &process_cwd, false);

    let mut seen: HashSet<String> = HashSet::new();
    let mut all: Vec<String> = Vec::new();
    let mut add = |found: Vec<PathBuf>| {
        for p in found {
            let s = p.to_string_lossy().into_owned();
            if seen.insert(s.clone()) {
                all.push(s);
            }
        }
    };

    // 1. Project-local extensions: cwd/CONFIG_DIR_NAME/extensions/
    //    (only when the project is trusted).
    if project_trusted {
        let local = Path::new(&resolved_cwd)
            .join(CONFIG_DIR_NAME)
            .join("extensions");
        add(discover_extensions_in_dir(&local));
    }

    // 2. Global extensions: agent_dir/extensions/
    add(discover_extensions_in_dir(
        &Path::new(&resolved_agent_dir).join("extensions"),
    ));

    // 3. Explicitly configured paths.
    for p in configured_paths {
        let resolved = paths::resolve_path(p, &resolved_cwd, true);
        let path = Path::new(&resolved);
        if path.is_dir() {
            if let Some(found) = resolve_extension_entries(path) {
                add(found);
                continue;
            }
            add(discover_extensions_in_dir(path));
            continue;
        }
        add(vec![PathBuf::from(resolved)]);
    }

    all
}
