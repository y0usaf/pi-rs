//! Upstream `core/extensions` and the extension half of `core/resource-loader.ts`: which paths
//! the `packages` setting, the `extensions` setting, `--extension` and the two standard
//! directories name, and the child that loads them.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::CONFIG_DIR_NAME;
use crate::paths::join;
use crate::settings::SettingsManager;

/// A command an extension registered, and the extension that registered it.
#[derive(Clone, Debug)]
pub struct ExtensionCommand {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Default)]
pub struct Extensions {
    pub commands: Vec<ExtensionCommand>,
    /// What loading has to say, one line each.
    pub notices: Vec<String>,
}

/// The child's directory: `PI_EXTENSION_HOST`, else the copy the build installed beside the binary.
fn host_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_EXTENSION_HOST") {
        return Some(PathBuf::from(dir));
    }
    let installed = std::env::current_exe()
        .ok()?
        .parent()?
        .parent()?
        .join("share/pi-rs/extension-host");
    installed.is_dir().then_some(installed)
}

/// A `packages` entry as a path: upstream's `PackageSource`, a string or an object naming one.
fn package_paths(settings: &SettingsManager) -> Vec<String> {
    settings
        .packages()
        .into_iter()
        .filter_map(|entry| match entry {
            Value::String(path) => Some(path),
            Value::Object(fields) => fields
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn resolve(path: &str, cwd: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        join(cwd, path)
    }
}

/// Loads what the settings and the flags name; nothing at all when they name nothing.
pub fn load(
    cwd: &str,
    agent_dir: &str,
    cli: &[String],
    no_extensions: bool,
    settings: &SettingsManager,
) -> Extensions {
    let mut extensions = Extensions::default();
    let dirs = if no_extensions {
        Vec::new()
    } else {
        vec![
            join(cwd, &format!("{CONFIG_DIR_NAME}/extensions")),
            join(agent_dir, "extensions"),
        ]
    };
    let mut paths: Vec<String> = cli.to_vec();
    if !no_extensions {
        paths.extend(package_paths(settings));
        paths.extend(settings.extension_paths());
    }
    for path in cli {
        let resolved = resolve(path, cwd);
        if !Path::new(&resolved).exists() {
            extensions
                .notices
                .push(format!("Extension path does not exist: {resolved}"));
        }
    }
    if paths.is_empty() && !dirs.iter().any(|dir| Path::new(dir).is_dir()) {
        return extensions;
    }
    let Some(dir) = host_dir() else {
        extensions.notices.push(
            "Warning: no extension host is installed, so no extension was loaded; set PI_EXTENSION_HOST"
                .to_owned(),
        );
        return extensions;
    };
    let load = match pi_extensions::Host::start(&dir, cwd)
        .and_then(|mut host| host.load(cwd, agent_dir, &dirs, &paths))
    {
        Ok(load) => load,
        Err(error) => {
            extensions.notices.push(format!("Warning: {error}"));
            return extensions;
        }
    };
    for error in &load.errors {
        extensions.notices.push(format!("Warning: {}", error.error));
    }
    for extension in load.extensions {
        for member in &extension.unsupported {
            extensions.notices.push(format!(
                "Warning: {} uses {member}, which pi-rs does not implement yet",
                extension.path
            ));
        }
        for command in extension.commands {
            extensions.commands.push(ExtensionCommand {
                name: command.name,
                description: command.description,
                path: extension.path.clone(),
            });
        }
    }
    extensions
}
