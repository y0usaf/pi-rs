//! Package sources and the three DESIGN-locked transports (PLAN 9.7).
//!
//! This is the Rust half of the pinned core/package-manager.ts port
//! (DESIGN locked decision: *Extension distribution*). It preserves Pi's
//! source grammar (npm:name[@version], Git URL/ref, local path), the
//! project/user install roots, identity/dedupe (project wins over user,
//! same identity keeps the first entry), offline-cache behavior
//! (PI_OFFLINE), and the install/remove/list/update/config outcomes.
//!
//! Transport contract (locked): npm is retained **only** as an archive
//! registry — the npm command unpacks the tarball into the managed install
//! root, and pi-rs never evaluates package JavaScript or exposes Node
//! module resolution. Package contents stay Lua configuration, modules,
//! and data (package.json's `pi` manifest is inert metadata that points
//! at Lua resources). Git and local transports likewise only materialize
//! files on disk; resource discovery reads .lua/.md/.json data and
//! a `pi` manifest, never JS.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::discover::CONFIG_DIR_NAME;
use crate::settings::SharedSettings;

/// Spec isOfflineModeEnabled(): PI_OFFLINE is truthy for
/// 1/true/yes (case-insensitive).
#[must_use]
pub fn is_offline_mode() -> bool {
    let Ok(value) = std::env::var("PI_OFFLINE") else {
        return false;
    };
    let value = value.to_ascii_lowercase();
    value == "1" || value == "true" || value == "yes"
}

/// Spec SourceScope (package-manager.ts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
    Temporary,
}

impl Scope {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Temporary => "temporary",
        }
    }
}

/// Spec NpmSource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmSource {
    pub spec: String,
    pub name: String,
    pub pinned: bool,
}

/// Spec GitSource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    pub repo: String,
    pub host: String,
    pub path: String,
    pub r#ref: Option<String>,
    pub pinned: bool,
}

/// Spec LocalSource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSource {
    pub path: String,
}

/// Spec ParsedSource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSource {
    Npm(NpmSource),
    Git(GitSource),
    Local(LocalSource),
}

/// Spec parseNpmSpec: name or scoped @scope/name with an optional
/// @version suffix. A bare name containing @ is kept whole.
fn parse_npm_spec(spec: &str) -> (String, Option<String>) {
    // Pi regex: ^(@?[^@]+(?:\/[^@]+)?)(?:@(.+))?$ — the name never
    // contains '@' except the leading scope marker; the version is
    // everything after the first '@' that terminates the name part.
    let name_end = if let Some(rest) = spec.strip_prefix('@') {
        match rest.find('/') {
            Some(slash) => {
                let after_slash = &rest[slash + 1..];
                match after_slash.find('@') {
                    // '@' sits at rest index slash+1+at; spec index is +1.
                    Some(at) => slash + at + 2,
                    None => spec.len(),
                }
            }
            None => spec.len(),
        }
    } else {
        match spec.find('@') {
            Some(at) => at,
            None => spec.len(),
        }
    };
    let name = &spec[..name_end];
    if name.is_empty() || name.ends_with('/') {
        return (spec.to_owned(), None);
    }
    if name_end < spec.len() {
        (name.to_owned(), Some(spec[name_end + 1..].to_owned()))
    } else {
        (name.to_owned(), None)
    }
}

/// Spec isLocalPath (utils/paths.ts): non-local protocol prefixes.
#[must_use]
pub fn is_local_path(value: &str) -> bool {
    let trimmed = value.trim();
    !(trimmed.starts_with("npm:")
        || trimmed.starts_with("git:")
        || trimmed.starts_with("github:")
        || trimmed.starts_with("http:")
        || trimmed.starts_with("https:")
        || trimmed.starts_with("ssh:"))
}

fn split_ref(url: &str) -> (String, Option<String>) {
    if let Some(scp) = url.strip_prefix("git@")
        && let Some(colon) = scp.find(':') {
            let host = &scp[..colon];
            let path_with_ref = &scp[colon + 1..];
            if let Some(at) = path_with_ref.rfind('@') {
                let repo_path = &path_with_ref[..at];
                let reference = &path_with_ref[at + 1..];
                if !repo_path.is_empty() && !reference.is_empty() {
                    return (format!("git@{host}:{repo_path}"), Some(reference.to_owned()));
                }
            }
            return (url.to_owned(), None);
        }
    if url.contains("://") {
        let mut parts = url.splitn(3, "://");
        let (scheme, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        // The ref separator lives in the pathname, not the authority
        // (userinfo): split at the first '@' after the leading '/'
        // (Pi: URL.pathname.indexOf("@")).
        if let Some(slash) = rest.find('/') {
            let path_with_ref = &rest[slash + 1..];
            if let Some(at) = path_with_ref.find('@') {
                let repo_path = &path_with_ref[..at];
                let reference = &path_with_ref[at + 1..];
                if !repo_path.is_empty() && !reference.is_empty() {
                    return (
                        format!("{scheme}://{}/{repo_path}", &rest[..slash]),
                        Some(reference.to_owned()),
                    );
                }
            }
        }
        return (url.to_owned(), None);
    }
    if let Some(slash) = url.find('/') {
        let host = &url[..slash];
        let path_with_ref = &url[slash + 1..];
        if let Some(at) = path_with_ref.rfind('@') {
            let repo_path = &path_with_ref[..at];
            let reference = &path_with_ref[at + 1..];
            if !repo_path.is_empty() && !reference.is_empty() {
                return (format!("{host}/{repo_path}"), Some(reference.to_owned()));
            }
        }
    }
    (url.to_owned(), None)
}

fn decode_for_validation(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Spec hasUnsafeGitInstallPart.
fn has_unsafe_git_install_part(value: &str, allow_slash: bool) -> bool {
    let Some(decoded) = decode_for_validation(value) else {
        return true;
    };
    for candidate in [value, decoded.as_str()] {
        if candidate.contains('\0') || candidate.contains('\\') || candidate.starts_with('/') {
            return true;
        }
        if !allow_slash && candidate.contains('/') {
            return true;
        }
        if candidate.split('/').any(|part| part == "..") {
            return true;
        }
    }
    false
}

/// Spec buildGitSource.
fn build_git_source(repo: String, host: String, path: String, reference: Option<String>) -> Option<GitSource> {
    if path.starts_with('/') {
        return None;
    }
    let normalized = path
        .strip_suffix(".git")
        .unwrap_or(&path)
        .trim_start_matches('/')
        .to_owned();
    if host.is_empty() || normalized.is_empty() || normalized.split('/').count() < 2 {
        return None;
    }
    if has_unsafe_git_install_part(&host, false) || has_unsafe_git_install_part(&normalized, true) {
        return None;
    }
    Some(GitSource {
        repo,
        host,
        path: normalized,
        r#ref: reference.clone(),
        pinned: reference.is_some(),
    })
}

/// Spec parseGitUrl (utils/git.ts), without the hosted-git-info shortcut
/// table: explicit-protocol URLs, git:-prefixed shorthands, and
/// host/path shorthands whose host carries a dot or is localhost.
/// #ref (hosted-git-info style) and @ref are both split.
#[must_use]
pub fn parse_git_url(source: &str) -> Option<GitSource> {
    let trimmed = source.trim();
    let has_git_prefix = trimmed.starts_with("git:");
    let url = if has_git_prefix { trimmed[4..].trim() } else { trimmed };

    if !has_git_prefix
        && !url.starts_with("https://")
        && !url.starts_with("http://")
        && !url.starts_with("ssh://")
        && !url.starts_with("git://")
    {
        return None;
    }

    let (repo_without_ref, ref_from_hash) = if let Some(hash) = url.find('#') {
        (url[..hash].to_owned(), Some(url[hash + 1..].to_owned()))
    } else {
        (url.to_owned(), None)
    };
    let (repo_no_ref, ref_from_at) = split_ref(&repo_without_ref);
    let reference = ref_from_hash.or(ref_from_at);
    let repo = repo_no_ref;

    if let Some(scp) = repo.strip_prefix("git@")
        && let Some(colon) = scp.find(':') {
            let host = &scp[..colon];
            let path = &scp[colon + 1..];
            return build_git_source(repo.clone(), host.to_owned(), path.to_owned(), reference);
        }
    if repo.starts_with("https://") || repo.starts_with("http://")
        || repo.starts_with("ssh://") || repo.starts_with("git://")
    {
        let after = repo.find("://").map(|index| &repo[index + 3..])?;
        let (host, path) = match after.find('/') {
            Some(slash) => (&after[..slash], &after[slash + 1..]),
            None => (after, ""),
        };
        // Strip userinfo (user@host) — Pi uses URL.hostname.
        let host = match host.rfind('@') {
            Some(at) => &host[at + 1..],
            None => host,
        };
        return build_git_source(repo.clone(), host.to_owned(), path.to_owned(), reference);
    }
    if let Some(slash) = repo.find('/') {
        let host = &repo[..slash];
        let path = &repo[slash + 1..];
        if !host.contains('.') && host != "localhost" {
            return None;
        }
        return build_git_source(format!("https://{repo}"), host.to_owned(), path.to_owned(), reference);
    }
    None
}

/// Spec parseSource: npm: prefix, then local-path check, then git.
#[must_use]
pub fn parse_source(source: &str) -> ParsedSource {
    if let Some(spec) = source.strip_prefix("npm:") {
        let spec = spec.trim().to_owned();
        let (name, version) = parse_npm_spec(&spec);
        return ParsedSource::Npm(NpmSource {
            spec,
            name,
            pinned: version.is_some(),
        });
    }
    if is_local_path(source) {
        return ParsedSource::Local(LocalSource {
            path: source.to_owned(),
        });
    }
    if let Some(git) = parse_git_url(source) {
        return ParsedSource::Git(git);
    }
    ParsedSource::Local(LocalSource {
        path: source.to_owned(),
    })
}

/// Expand ~/~/… and file:// URLs, then make the path absolute against
/// base. Mirrors utils/paths.ts normalizePath + resolvePath.
#[must_use]
pub fn resolve_path_from_base(input: &str, base: &Path) -> PathBuf {
    let trimmed = input.trim();
    let expanded = if trimmed == "~" {
        home_dir()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home_dir().join(rest)
    } else if let Some(rest) = trimmed.strip_prefix("file://") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(trimmed)
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    };
    lexical_normalize(&joined)
}

/// Lexically normalize like Node's path.resolve: resolve "." and
/// ".." components without touching the filesystem.
fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[must_use]
pub fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_owned()))
}

/// Spec resolveManagedPath: refuse paths escaping the install root.
fn resolve_managed_path(root: &Path, parts: &[&str]) -> Result<PathBuf, String> {
    let resolved_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut candidate = resolved_root.clone();
    for part in parts {
        candidate.push(part);
    }
    if candidate != resolved_root && !candidate.starts_with(&resolved_root) {
        return Err(format!("Refusing to use path outside package install root: {}", candidate.display()));
    }
    Ok(candidate)
}

/// Spec getExtensionTempFolder + getTemporaryDir: deterministic 8-char
/// sha256 of prefix-suffix under agentDir/tmp/extensions.
pub fn temporary_dir(agent_dir: &Path, prefix: &str, suffix: Option<&str>) -> PathBuf {
    let root = agent_dir.join("tmp").join("extensions").join(prefix);
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(format!("{prefix}-{}", suffix.unwrap_or("")));
        let hash = hasher.finalize();
        let mut out = String::with_capacity(8);
        for byte in hash.iter().take(4) {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    };
    root.join(digest).join(suffix.unwrap_or(""))
}

/// Spec getNpmInstallRoot.
pub fn npm_install_root(scope: Scope, temporary: bool, cwd: &Path, agent_dir: &Path) -> PathBuf {
    if temporary {
        return temporary_dir(agent_dir, "npm", None);
    }
    match scope {
        Scope::Project => cwd.join(CONFIG_DIR_NAME).join("npm"),
        Scope::User => agent_dir.join("npm"),
        Scope::Temporary => temporary_dir(agent_dir, "npm", None),
    }
}

/// Spec getGitInstallRoot (None for temporary scope).
#[must_use]
pub fn git_install_root(scope: Scope, cwd: &Path, agent_dir: &Path) -> Option<PathBuf> {
    match scope {
        Scope::Temporary => None,
        Scope::Project => Some(cwd.join(CONFIG_DIR_NAME).join("git")),
        Scope::User => Some(agent_dir.join("git")),
    }
}

/// Spec getBaseDirForScope.
#[must_use]
pub fn base_dir_for_scope(scope: Scope, cwd: &Path, agent_dir: &Path) -> PathBuf {
    match scope {
        Scope::Project => cwd.join(CONFIG_DIR_NAME),
        Scope::User => agent_dir.to_path_buf(),
        Scope::Temporary => cwd.to_path_buf(),
    }
}

/// Spec getManagedNpmInstallPath.
pub fn managed_npm_install_path(source: &NpmSource, scope: Scope, cwd: &Path, agent_dir: &Path) -> PathBuf {
    let root = npm_install_root(scope, scope == Scope::Temporary, cwd, agent_dir);
    root.join("node_modules").join(&source.name)
}

/// Spec getGitInstallPath.
pub fn git_install_path(source: &GitSource, scope: Scope, cwd: &Path, agent_dir: &Path) -> Result<PathBuf, String> {
    if scope == Scope::Temporary {
        return Ok(temporary_dir(agent_dir, &format!("git-{}", source.host), Some(&source.path)));
    }
    let install_root = git_install_root(scope, cwd, agent_dir)
        .ok_or_else(|| "Missing git install root".to_owned())?;
    let host = resolve_managed_path(&install_root, &[&source.host])?;
    resolve_managed_path(&host, &[&source.path])
}

/// Spec getPackageIdentity: identity ignores version/ref; SSH and HTTPS
/// forms of the same repo share git:host/path; local paths resolve
/// against the scope base.
#[must_use]
pub fn package_identity(source: &ParsedSource, scope: Scope, cwd: &Path, agent_dir: &Path) -> String {
    match source {
        ParsedSource::Npm(source) => format!("npm:{}", source.name),
        ParsedSource::Git(source) => format!("git:{}/{}", source.host, source.path),
        ParsedSource::Local(source) => {
            let base = base_dir_for_scope(scope, cwd, agent_dir);
            let resolved = resolve_path_from_base(&source.path, &base);
            format!("local:{}", resolved.display())
        }
    }
}

/// One configured package entry (spec ConfiguredPackage).
#[derive(Debug, Clone)]
pub struct ConfiguredPackage {
    pub source: String,
    pub scope: Scope,
    pub filtered: bool,
    pub installed_path: Option<String>,
}

/// The shared package manager. All install roots and persistence follow
/// the pinned package-manager.ts layout; the settings store is the
/// canonical Lua config (packages declared via pi.config.packages).
pub struct PackageManager {
    cwd: PathBuf,
    agent_dir: PathBuf,
    settings: SharedSettings,
}

impl PackageManager {
    #[must_use]
    pub fn new(cwd: &str, agent_dir: &str, settings: SharedSettings) -> Self {
        Self {
            cwd: PathBuf::from(cwd),
            agent_dir: PathBuf::from(agent_dir),
            settings,
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_ , crate::settings_manager::SettingsManager>, String> {
        self.settings
            .lock()
            .map_err(|_| "settings store poisoned".to_owned())
    }

    fn project_trusted(&self) -> bool {
        self.lock().map(|s| s.is_project_trusted()).unwrap_or(false)
    }

    /// Spec assertProjectTrustedForScope.
    fn assert_project_trusted_for_scope(&self, scope: Scope) -> Result<(), String> {
        if scope == Scope::Project && !self.project_trusted() {
            return Err("Project is not trusted; refusing to access project package storage".to_owned());
        }
        Ok(())
    }

    /// Spec getNpmCommand.
    fn npm_command(&self) -> (String, Vec<String>) {
        let configured = self.lock().ok().and_then(|s| s.get_npm_command());
        match configured {
            Some(parts) if !parts.is_empty() => {
                let mut parts = parts.into_iter();
                let command = parts.next().unwrap_or_else(|| "npm".to_owned());
                (command, parts.collect())
            }
            _ => ("npm".to_owned(), Vec::new()),
        }
    }

    /// Spec getPackageManagerName: the command after the last --.
    fn package_manager_name(&self) -> String {
        let (command, args) = self.npm_command();
        let mut parts = vec![command];
        parts.extend(args);
        let mut name = "npm".to_owned();
        for (index, part) in parts.iter().enumerate() {
            if part == "--" && index + 1 < parts.len() {
                name = parts[index + 1].clone();
            }
        }
        Path::new(&name)
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .trim_end_matches(".cmd")
                    .trim_end_matches(".exe")
                    .to_owned()
            })
            .unwrap_or_else(|| "npm".to_owned())
    }

    /// Spec getNpmInstallArgs: peer resolution disabled for managed
    /// installs so package managers never solve host-provided pi peers.
    fn npm_install_args(&self, specs: &[String], install_root: &Path) -> Vec<String> {
        let name = self.package_manager_name();
        let mut args = vec!["install".to_owned()];
        args.extend(specs.iter().cloned());
        if name == "bun" {
            args.push("--cwd".to_owned());
            args.push(install_root.to_string_lossy().into_owned());
            args.push("--omit=peer".to_owned());
        } else if name == "pnpm" {
            args.push("--prefix".to_owned());
            args.push(install_root.to_string_lossy().into_owned());
            args.push("--config.auto-install-peers=false".to_owned());
            args.push("--config.strict-peer-dependencies=false".to_owned());
            args.push("--config.strict-dep-builds=false".to_owned());
        } else {
            args.push("--prefix".to_owned());
            args.push(install_root.to_string_lossy().into_owned());
            args.push("--legacy-peer-deps".to_owned());
        }
        args
    }

    /// Spec getGitDependencyInstallArgs.
    fn git_dependency_install_args(&self) -> Vec<String> {
        match self.lock().ok().and_then(|s| s.get_npm_command()) {
            Some(parts) if !parts.is_empty() => vec!["install".to_owned()],
            _ => vec!["install".to_owned(), "--omit=dev".to_owned()],
        }
    }

    /// Spec ensureNpmProject.
    fn ensure_npm_project(&self, install_root: &Path) -> Result<(), String> {
        std::fs::create_dir_all(install_root).map_err(|e| format!("mkdir {}: {e}", install_root.display()))?;
        self.ensure_git_ignore(install_root)?;
        let package_json = install_root.join("package.json");
        if !package_json.exists() {
            let body = "{\n  \"name\": \"pi-extensions\",\n  \"private\": true\n}\n";
            std::fs::write(&package_json, body).map_err(|e| format!("write {}: {e}", package_json.display()))?;
        }
        Ok(())
    }

    /// Spec ensureGitIgnore.
    fn ensure_git_ignore(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let ignore_path = dir.join(".gitignore");
        if !ignore_path.exists() {
            std::fs::write(&ignore_path, "*\n!.gitignore\n")
                .map_err(|e| format!("write {}: {e}", ignore_path.display()))?;
        }
        Ok(())
    }

    /// Run a command, capturing stdout.
    fn run_command(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&Path>,
        env: &[(String, String)],
        _timeout: Option<Duration>,
    ) -> Result<String, String> {
        let mut child = Command::new(command);
        child.args(args);
        if let Some(cwd) = cwd {
            child.current_dir(cwd);
        }
        // Inherit the parent environment (npm/git need PATH, HOME, proxies)
        // and layer the per-command overrides on top.
        for (key, value) in env {
            child.env(key, value);
        }
        let output = child
            .output()
            .map_err(|e| format!("failed to run {command} {}: {e}", args.join(" ")))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            let status = output.status;
            return Err(format!("{command} {} failed with {status}: {}", args.join(" "), if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() }));
        }
        Ok(stdout)
    }

    /// Spec getNpmInstallPath (managed; legacy global lookup falls back
    /// to the managed path when absent).
    pub fn get_npm_install_path(&self, source: &NpmSource, scope: Scope) -> PathBuf {
        let managed = managed_npm_install_path(source, scope, &self.cwd, &self.agent_dir);
        if scope != Scope::User || managed.exists() {
            return managed;
        }
        let legacy = self.legacy_global_npm_install_path(source);
        if legacy.exists() { legacy } else { managed }
    }

    fn legacy_global_npm_install_path(&self, source: &NpmSource) -> PathBuf {
        let (command, args) = self.npm_command();
        let name = self.package_manager_name();
        if name == "bun" {
            let mut bin_args = args.clone();
            bin_args.extend(["pm".to_owned(), "bin".to_owned(), "-g".to_owned()]);
            if let Ok(out) = self.run_command(&command, &bin_args, None, &[], None) {
                let bin_dir = PathBuf::from(out.trim());
                if let Some(parent) = bin_dir.parent() {
                    return parent.join("install").join("global").join("node_modules").join(&source.name);
                }
            }
            return PathBuf::new();
        }
        let mut root_args = args.clone();
        root_args.extend(["root".to_owned(), "-g".to_owned()]);
        if let Ok(out) = self.run_command(&command, &root_args, None, &[], None) {
            return PathBuf::from(out.trim()).join(&source.name);
        }
        PathBuf::new()
    }

    /// Spec getGitInstallPath.
    pub fn get_git_install_path(&self, source: &GitSource, scope: Scope) -> Result<PathBuf, String> {
        git_install_path(source, scope, &self.cwd, &self.agent_dir)
    }

    /// Spec getInstalledPath.
    pub fn get_installed_path(&self, source: &str, scope: Scope) -> Option<String> {
        match parse_source(source) {
            ParsedSource::Npm(source) => {
                let path = self.get_npm_install_path(&source, scope);
                if path.exists() { Some(path.to_string_lossy().into_owned()) } else { None }
            }
            ParsedSource::Git(source) => {
                let path = self.get_git_install_path(&source, scope).ok()?;
                if path.exists() { Some(path.to_string_lossy().into_owned()) } else { None }
            }
            ParsedSource::Local(source) => {
                let base = base_dir_for_scope(scope, &self.cwd, &self.agent_dir);
                let path = resolve_path_from_base(&source.path, &base);
                if path.exists() { Some(path.to_string_lossy().into_owned()) } else { None }
            }
        }
    }

    /// Spec addSourceToSettings.
    pub fn add_source_to_settings(&self, source: &str, scope: Scope) -> bool {
        let normalized = self.normalize_package_source_for_settings(source, scope);
        let mut settings = match self.lock() {
            Ok(settings) => settings,
            Err(_) => return false,
        };
        let settings_map = if scope == Scope::Project {
            settings.get_project_settings()
        } else {
            settings.get_global_settings()
        };
        let mut packages: Vec<Value> = match settings_map.get("packages") {
            Some(Value::Array(values)) => values.clone(),
            _ => Vec::new(),
        };
        let match_index = packages.iter().position(|existing| self.package_sources_match(existing, source, scope));
        if let Some(index) = match_index {
            let existing = &packages[index];
            if self.get_package_source_string(existing) == normalized {
                return false;
            }
            packages[index] = match existing {
                Value::String(_) => Value::String(normalized.clone()),
                Value::Object(map) => {
                    let mut next = map.clone();
                    next.insert("source".to_owned(), Value::String(normalized.clone()));
                    Value::Object(next)
                }
                _ => Value::String(normalized.clone()),
            };
            let result = if scope == Scope::Project {
                settings.set_project_packages(packages)
            } else {
                let _: () = settings.set_packages(packages);
                Ok(())
            };
            return result.is_ok();
        }
        packages.push(Value::String(normalized));
        let result = if scope == Scope::Project {
            settings.set_project_packages(packages)
        } else {
            let _: () = settings.set_packages(packages);
            Ok(())
        };
        result.is_ok()
    }

    /// Spec removeSourceFromSettings.
    pub fn remove_source_from_settings(&self, source: &str, scope: Scope) -> bool {
        let mut settings = match self.lock() {
            Ok(settings) => settings,
            Err(_) => return false,
        };
        let settings_map = if scope == Scope::Project {
            settings.get_project_settings()
        } else {
            settings.get_global_settings()
        };
        let Some(Value::Array(values)) = settings_map.get("packages") else {
            return false;
        };
        let next: Vec<Value> = values
            .iter()
            .filter(|existing| !self.package_sources_match(existing, source, scope))
            .cloned()
            .collect();
        if next.len() == values.len() {
            return false;
        }
        let result = if scope == Scope::Project {
            settings.set_project_packages(next)
        } else {
            let _: () = settings.set_packages(next);
            Ok(())
        };
        result.is_ok()
    }

    fn get_package_source_string(&self, package: &Value) -> String {
        match package {
            Value::String(source) => source.clone(),
            Value::Object(map) => map.get("source").and_then(Value::as_str).unwrap_or("").to_owned(),
            _ => String::new(),
        }
    }

    /// Spec getSourceMatchKeyForInput / getSourceMatchKeyForSettings.
    fn source_match_key(&self, source: &str, scope: Option<Scope>) -> String {
        let parsed = parse_source(source);
        match &parsed {
            ParsedSource::Npm(source) => format!("npm:{}", source.name),
            ParsedSource::Git(source) => format!("git:{}/{}", source.host, source.path),
            ParsedSource::Local(source) => {
                let base = match scope {
                    Some(scope) => base_dir_for_scope(scope, &self.cwd, &self.agent_dir),
                    None => self.cwd.clone(),
                };
                format!("local:{}", resolve_path_from_base(&source.path, &base).display())
            }
        }
    }

    fn package_sources_match(&self, existing: &Value, input_source: &str, scope: Scope) -> bool {
        let left = self.source_match_key(&self.get_package_source_string(existing), Some(scope));
        let right = self.source_match_key(input_source, None);
        left == right
    }

    /// Spec normalizePackageSourceForSettings: local sources are stored
    /// relative to the scope base.
    fn normalize_package_source_for_settings(&self, source: &str, scope: Scope) -> String {
        let parsed = parse_source(source);
        let ParsedSource::Local(local) = parsed else {
            return source.to_owned();
        };
        let base = base_dir_for_scope(scope, &self.cwd, &self.agent_dir);
        let resolved = resolve_path_from_base(&local.path, &self.cwd);
        let relative = pathdiff(&base, &resolved);
        if relative.is_empty() { ".".to_owned() } else { relative }
    }

    /// Spec getPackageIdentity on a raw source string.
    pub fn get_package_identity(&self, source: &str, scope: Option<Scope>) -> String {
        let parsed = parse_source(source);
        match scope {
            Some(scope) => package_identity(&parsed, scope, &self.cwd, &self.agent_dir),
            None => package_identity(&parsed, Scope::User, &self.cwd, &self.agent_dir),
        }
    }

    /// Spec dedupePackages: project wins over user for the same
    /// identity; same scope keeps the first entry.
    #[must_use]
    pub fn dedupe_packages(&self, packages: &[(String, Scope)]) -> Vec<(String, Scope)> {
        let mut seen: std::collections::HashMap<String, (String, Scope)> = std::collections::HashMap::new();
        for (source, scope) in packages {
            let identity = self.get_package_identity(source, Some(*scope));
            match seen.get(&identity) {
                None => {
                    seen.insert(identity, (source.clone(), *scope));
                }
                Some((_, existing_scope)) if *scope == Scope::Project && *existing_scope == Scope::User => {
                    seen.insert(identity, (source.clone(), *scope));
                }
                Some(_) => {}
            }
        }
        seen.into_values().collect()
    }

    /// Spec listConfiguredPackages: user first, then project.
    pub fn list(&self) -> Vec<ConfiguredPackage> {
        let mut result = Vec::new();
        let global = self.lock().map(|s| s.get_global_settings()).unwrap_or_default();
        let project = self.lock().map(|s| s.get_project_settings()).unwrap_or_default();
        for scope in [Scope::User, Scope::Project] {
            let packages = if scope == Scope::User { &global } else { &project };
            let Some(Value::Array(values)) = packages.get("packages") else {
                continue;
            };
            for package in values {
                let source = self.get_package_source_string(package);
                result.push(ConfiguredPackage {
                    source: source.clone(),
                    scope,
                    filtered: matches!(package, Value::Object(_)),
                    installed_path: self.get_installed_path(&source, scope),
                });
            }
        }
        result
    }

    /// Spec install: local sources only validate existence.
    pub fn install(&self, source: &str, scope: Scope) -> Result<(), String> {
        self.assert_project_trusted_for_scope(scope)?;
        match parse_source(source) {
            ParsedSource::Npm(source) => self.install_npm(&source, scope, scope == Scope::Temporary),
            ParsedSource::Git(source) => self.install_git(&source, scope),
            ParsedSource::Local(source) => {
                let resolved = resolve_path_from_base(&source.path, &self.cwd);
                if !resolved.exists() {
                    return Err(format!("Path does not exist: {}", resolved.display()));
                }
                Ok(())
            }
        }
    }

    /// Spec installAndPersist.
    pub fn install_and_persist(&self, source: &str, scope: Scope) -> Result<(), String> {
        self.install(source, scope)?;
        self.add_source_to_settings(source, scope);
        Ok(())
    }

    /// Spec remove.
    pub fn remove(&self, source: &str, scope: Scope) -> Result<(), String> {
        self.assert_project_trusted_for_scope(scope)?;
        match parse_source(source) {
            ParsedSource::Npm(source) => self.uninstall_npm(&source, scope),
            ParsedSource::Git(source) => self.remove_git(&source, scope),
            ParsedSource::Local(_) => Ok(()),
        }
    }

    /// Spec removeAndPersist.
    pub fn remove_and_persist(&self, source: &str, scope: Scope) -> Result<bool, String> {
        self.remove(source, scope)?;
        Ok(self.remove_source_from_settings(source, scope))
    }

    /// Spec update: pinned npm versions are fixed; pinned git refs
    /// reconcile the configured checkout.
    pub fn update(&self, source: Option<&str>) -> Result<(), String> {
        if is_offline_mode() {
            return Ok(());
        }
        let identity = source.map(|source| self.get_package_identity(source, None));
        let mut matched = false;
        let mut update_sources: Vec<(String, Scope)> = Vec::new();
        for scope in [Scope::User, Scope::Project] {
            let settings = self
                .lock()
                .map(|s| if scope == Scope::User { s.get_global_settings() } else { s.get_project_settings() })
                .unwrap_or_default();
            let Some(Value::Array(values)) = settings.get("packages") else {
                continue;
            };
            for package in values {
                let source_str = self.get_package_source_string(package);
                let candidate = self.get_package_identity(&source_str, Some(scope));
                if let Some(identity) = &identity
                    && &candidate != identity {
                        continue;
                    }
                matched = true;
                update_sources.push((source_str, scope));
            }
        }
        if source.is_some() && !matched {
            return Err(format!("No matching package found for {}", source.unwrap_or("")));
        }
        for (source, scope) in update_sources {
            let parsed = parse_source(&source);
            match &parsed {
                ParsedSource::Npm(npm) if !npm.pinned => self.install_npm(npm, scope, false)?,
                ParsedSource::Git(git) => self.update_git(git, scope)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn install_npm(&self, source: &NpmSource, scope: Scope, temporary: bool) -> Result<(), String> {
        let install_root = npm_install_root(scope, temporary, &self.cwd, &self.agent_dir);
        self.ensure_npm_project(&install_root)?;
        let (command, mut args) = self.npm_command();
        args.extend(self.npm_install_args(std::slice::from_ref(&source.spec), &install_root));
        self.run_command(&command, &args, None, &[], None)?;
        Ok(())
    }

    fn uninstall_npm(&self, source: &NpmSource, scope: Scope) -> Result<(), String> {
        let install_root = npm_install_root(scope, false, &self.cwd, &self.agent_dir);
        if !install_root.exists() {
            return Ok(());
        }
        let (command, mut args) = self.npm_command();
        if self.package_manager_name() == "bun" {
            args.push("uninstall".to_owned());
            args.push(source.name.clone());
            args.push("--cwd".to_owned());
            args.push(install_root.to_string_lossy().into_owned());
        } else {
            args.push("uninstall".to_owned());
            args.push(source.name.clone());
            args.push("--prefix".to_owned());
            args.push(install_root.to_string_lossy().into_owned());
        }
        self.run_command(&command, &args, None, &[], None)?;
        Ok(())
    }

    fn install_git(&self, source: &GitSource, scope: Scope) -> Result<(), String> {
        let target = self.get_git_install_path(source, scope)?;
        if target.exists() {
            return if let Some(reference) = &source.r#ref {
                self.ensure_git_ref(&target, &["fetch".to_owned(), "origin".to_owned(), reference.clone()], "FETCH_HEAD")
            } else {
                let fetch_args = self.local_git_update_fetch_args(&target)?;
                self.ensure_git_ref(&target, &fetch_args, "@{upstream}")
            };
        }
        if let Some(root) = git_install_root(scope, &self.cwd, &self.agent_dir) {
            self.ensure_git_ignore(&root)?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        self.run_command(
            "git",
            &["clone".to_owned(), source.repo.clone(), target.to_string_lossy().into_owned()],
            None,
            &[],
            None,
        )?;
        if let Some(reference) = &source.r#ref {
            self.run_command("git", &["checkout".to_owned(), reference.clone()], Some(&target), &[], None)?;
        }
        let package_json = target.join("package.json");
        if package_json.exists() {
            let (command, mut args) = self.npm_command();
            args.extend(self.git_dependency_install_args());
            self.run_command(&command, &args, Some(&target), &[], None)?;
        }
        Ok(())
    }

    fn update_git(&self, source: &GitSource, scope: Scope) -> Result<(), String> {
        let target = self.get_git_install_path(source, scope)?;
        if !target.exists() {
            return self.install_git(source, scope);
        }
        if let Some(reference) = &source.r#ref {
            return self.ensure_git_ref(&target, &["fetch".to_owned(), "origin".to_owned(), reference.clone()], "FETCH_HEAD");
        }
        let fetch_args = self.local_git_update_fetch_args(&target)?;
        self.ensure_git_ref(&target, &fetch_args, "@{upstream}")
    }

    /// Spec getLocalGitUpdateTarget: local upstream branch fetch args.
    fn local_git_update_fetch_args(&self, target: &Path) -> Result<Vec<String>, String> {
        let upstream = self
            .run_command(
                "git",
                &["rev-parse".to_owned(), "--abbrev-ref".to_owned(), "@{upstream}".to_owned()],
                Some(target),
                &[],
                None,
            )
            .map_err(|_| "Unsupported upstream remote".to_owned())?;
        let trimmed = upstream.trim().to_owned();
        if !trimmed.starts_with("origin/") {
            return Err(format!("Unsupported upstream remote: {trimmed}"));
        }
        let branch = trimmed.strip_prefix("origin/").unwrap_or("").to_owned();
        if branch.is_empty() {
            return Err("Missing upstream branch name".to_owned());
        }
        Ok(vec![
            "fetch".to_owned(),
            "--prune".to_owned(),
            "--no-tags".to_owned(),
            "origin".to_owned(),
            format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"),
        ])
    }

    /// Spec ensureGitRef: fetch the ref, hard-reset, clean untracked.
    fn ensure_git_ref(&self, target: &Path, fetch_args: &[String], reference: &str) -> Result<(), String> {
        self.run_command("git", fetch_args, Some(target), &[], None)?;
        let local_head = self.run_command("git", &["rev-parse".to_owned(), "HEAD".to_owned()], Some(target), &[], None)?;
        let commit_ref = format!("{reference}^{{commit}}");
        let target_head = self.run_command("git", &["rev-parse".to_owned(), commit_ref.clone()], Some(target), &[], None)?;
        if local_head.trim() == target_head.trim() {
            return Ok(());
        }
        self.run_command("git", &["reset".to_owned(), "--hard".to_owned(), commit_ref], Some(target), &[], None)?;
        self.run_command("git", &["clean".to_owned(), "-fdx".to_owned()], Some(target), &[], None)?;
        let package_json = target.join("package.json");
        if package_json.exists() {
            let (command, mut args) = self.npm_command();
            args.extend(self.git_dependency_install_args());
            self.run_command(&command, &args, Some(target), &[], None)?;
        }
        Ok(())
    }

    fn remove_git(&self, source: &GitSource, scope: Scope) -> Result<(), String> {
        let target = self.get_git_install_path(source, scope)?;
        if !target.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&target).map_err(|e| format!("rm {}: {e}", target.display()))?;
        self.prune_empty_git_parents(&target, git_install_root(scope, &self.cwd, &self.agent_dir).as_deref());
        Ok(())
    }

    /// Spec pruneEmptyGitParents.
    fn prune_empty_git_parents(&self, target_dir: &Path, install_root: Option<&Path>) {
        let Some(install_root) = install_root else { return };
        let resolved_root = install_root.canonicalize().unwrap_or_else(|_| install_root.to_path_buf());
        let mut current = target_dir.parent().map(Path::to_path_buf);
        while let Some(dir) = current {
            if !dir.starts_with(&resolved_root) || dir == resolved_root {
                break;
            }
            let empty = match std::fs::read_dir(&dir) {
                Ok(mut entries) => entries.next().is_none(),
                Err(_) => {
                    current = dir.parent().map(Path::to_path_buf);
                    continue;
                }
            };
            if !empty {
                break;
            }
            let _ = std::fs::remove_dir(&dir);
            current = dir.parent().map(Path::to_path_buf);
        }
    }
}

/// Relative path from base to target (both absolute); empty string when
/// they are the same directory.
fn pathdiff(base: &Path, target: &Path) -> String {
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let target = target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
    if base == target {
        return String::new();
    }
    let mut base_parts: Vec<&std::ffi::OsStr> = base.components().map(|c| c.as_os_str()).collect();
    let mut target_parts: Vec<&std::ffi::OsStr> = target.components().map(|c| c.as_os_str()).collect();
    while !base_parts.is_empty() && !target_parts.is_empty() && base_parts[0] == target_parts[0] {
        base_parts.remove(0);
        target_parts.remove(0);
    }
    let mut out = vec!["..".to_owned(); base_parts.len()];
    out.extend(target_parts.iter().map(|part| part.to_string_lossy().into_owned()));
    out.join("/")
}

/// All configured package sources (project first so project wins dedupe
/// order matches the spec's resolve).
#[must_use]
pub fn configured_package_sources(settings: &crate::settings_manager::SettingsManager) -> Vec<(String, Scope)> {
    let mut sources = Vec::new();
    let global = settings.get_global_settings();
    let project = settings.get_project_settings();
    for (settings_map, scope) in [(&project, Scope::Project), (&global, Scope::User)] {
        if let Some(Value::Array(values)) = settings_map.get("packages") {
            for value in values {
                let source = match value {
                    Value::String(source) => source.clone(),
                    Value::Object(map) => map.get("source").and_then(Value::as_str).unwrap_or("").to_owned(),
                    _ => String::new(),
                };
                if !source.is_empty() {
                    sources.push((source, scope));
                }
            }
        }
    }
    sources
}

impl PackageManager {
    /// Cheap clone for handing the same manager to concurrent closures.
    #[must_use]
    pub fn clone_manager(&self) -> Self {
        Self {
            cwd: self.cwd.clone(),
            agent_dir: self.agent_dir.clone(),
            settings: self.settings.clone(),
        }
    }
}
