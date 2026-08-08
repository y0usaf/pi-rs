//! Resource discovery — provenance, precedence, dedupe, toggles, reload
//! (PLAN 9.7; the resource half of the pinned core/package-manager.ts port).
//!
//! Resources are files of four kinds — Lua extensions, Pi-compatible skill
//! and prompt content (markdown), and theme data (JSON). Every resolved
//! resource carries its provenance (source string or "auto", scope, and
//! whether it came from a package or a top-level root), is sorted by Pi's
//! precedence rank (project-local, project-auto, user-local, user-auto,
//! then package resources), and deduplicated by canonical path with
//! first-wins collisions. Enabled state is a toggle: discovery collects
//! every candidate, then settings overrides (patterns) and selectors
//! (config.enable/disable) decide enabled=false without hiding the file.
//!
//! Reload is cheap by construction: `resolve` is a pure function of the
//! filesystem and the current settings snapshot, so a /reload that re-runs
//! the config pipeline then re-resolves publishes the whole next graph
//! atomically.

use std::collections::{BTreeMap, HashSet};

type ResourceIndex = BTreeMap<String, (Vec<String>, Vec<String>)>;

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::packages::{
    PackageManager, ParsedSource, Scope, base_dir_for_scope, parse_source, resolve_path_from_base,
};
use crate::settings::SharedSettings;

/// Spec RESOURCE_TYPES order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceType {
    Extensions,
    Skills,
    Prompts,
    Themes,
}

impl ResourceType {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ResourceType::Extensions => "extensions",
            ResourceType::Skills => "skills",
            ResourceType::Prompts => "prompts",
            ResourceType::Themes => "themes",
        }
    }

    #[must_use]
    pub fn all() -> [ResourceType; 4] {
        [
            ResourceType::Extensions,
            ResourceType::Skills,
            ResourceType::Prompts,
            ResourceType::Themes,
        ]
    }
}

/// Spec PathMetadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMetadata {
    pub source: String,
    pub scope: Scope,
    pub origin: Origin,
    pub base_dir: Option<PathBuf>,
}

/// Spec origin ("package" | "top-level").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Package,
    TopLevel,
}

/// Spec ResolvedResource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource {
    pub path: String,
    pub enabled: bool,
    pub metadata: PathMetadata,
}

/// Spec ResolvedPaths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPaths {
    pub extensions: Vec<ResolvedResource>,
    pub skills: Vec<ResolvedResource>,
    pub prompts: Vec<ResolvedResource>,
    pub themes: Vec<ResolvedResource>,
}

impl ResolvedPaths {
    #[must_use]
    pub fn for_type(&self, kind: ResourceType) -> &[ResolvedResource] {
        match kind {
            ResourceType::Extensions => &self.extensions,
            ResourceType::Skills => &self.skills,
            ResourceType::Prompts => &self.prompts,
            ResourceType::Themes => &self.themes,
        }
    }
}

/// Spec resourcePrecedenceRank: lower rank = higher precedence.
fn precedence_rank(metadata: &PathMetadata) -> u8 {
    if metadata.origin == Origin::Package {
        return 4;
    }
    let scope_base = if metadata.scope == Scope::Project { 0 } else { 2 };
    scope_base + if metadata.source == "local" { 0 } else { 1 }
}

/// Canonical path (realpath when it resolves, raw otherwise).
fn canonicalize(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Spec toResolvedPaths: sort by precedence then keep first canonical path.
fn to_resolved(resources: Vec<ResolvedResource>) -> Vec<ResolvedResource> {
    let mut resources = resources;
    resources.sort_by_key(|entry| precedence_rank(&entry.metadata));
    let mut seen: HashSet<String> = HashSet::new();
    resources
        .into_iter()
        .filter(|entry| seen.insert(canonicalize(Path::new(&entry.path))))
        .collect()
}

/// Minimal gitignore matcher (the `ignore` npm package subset Pi uses):
/// blank lines and # comments, ! negation, trailing / directories,
/// leading / anchoring, and * / ? / ** globs. Patterns are prefixed with
/// the relative directory when loaded from a subdirectory.
struct IgnoreMatcher {
    rules: Vec<(String, bool, bool)>, // (glob, negated, dir_only)
}

impl IgnoreMatcher {
    fn new() -> Self {
        Self { rules: Vec::new() }
    }

    #[allow(dead_code)]
    fn add_file(&mut self, dir: &Path, root: &Path) {
        let relative = dir.strip_prefix(root).unwrap_or(dir);
        let prefix = if relative.as_os_str().is_empty() {
            String::new()
        } else {
            format!("{}/", relative.to_string_lossy().replace('\\', "/"))
        };
        for file_name in [".gitignore", ".ignore", ".fdignore"] {
            let path = dir.join(file_name);
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.starts_with('#') && !trimmed.starts_with("\\#") {
                    continue;
                }
                let mut pattern = line;
                let mut negated = false;
                if let Some(rest) = pattern.strip_prefix('!') {
                    negated = true;
                    pattern = rest;
                } else if let Some(rest) = pattern.strip_prefix("\\!") {
                    pattern = rest;
                }
                if let Some(rest) = pattern.strip_prefix('/') {
                    pattern = rest;
                }
                let dir_only = pattern.ends_with('/');
                let pattern = pattern.trim_end_matches('/');
                let prefixed = if prefix.is_empty() {
                    pattern.to_owned()
                } else {
                    format!("{prefix}{pattern}")
                };
                self.rules.push((prefixed, negated, dir_only));
            }
        }
    }

    /// Spec ig.ignores(relPath) — a path is ignored when the last matching
    /// rule (negation wins) says so.
    fn ignores(&self, rel_path: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for (pattern, negated, dir_only) in &self.rules {
            if *dir_only && !is_dir {
                continue;
            }
            if glob_match(pattern, rel_path) {
                ignored = !*negated;
            }
        }
        ignored
    }
}

/// Glob matching supporting * (no /), ? (no /), and ** (any depth).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    glob_match_rec(&pattern, 0, &text, 0)
}

fn glob_match_rec(pattern: &[char], p: usize, text: &[char], t: usize) -> bool {
    if p == pattern.len() {
        return t == text.len();
    }
    match pattern[p] {
        '*' => {
            // ** crosses directories; a single * does not.
            if pattern.get(p + 1) == Some(&'*') {
                let mut next = p + 2;
                while next < pattern.len() && pattern[next] == '*' {
                    next += 1;
                }
                // **/ matches zero or more leading directories.
                let mut at = t;
                loop {
                    if glob_match_rec(pattern, next, text, at) {
                        return true;
                    }
                    if at >= text.len() {
                        return false;
                    }
                    at += 1;
                }
            }
            let mut at = t;
            loop {
                if glob_match_rec(pattern, p + 1, text, at) {
                    return true;
                }
                if at >= text.len() || text[at] == '/' {
                    return false;
                }
                at += 1;
            }
        }
        '?' => {
            if t < text.len() && text[t] != '/' {
                glob_match_rec(pattern, p + 1, text, t + 1)
            } else {
                false
            }
        }
        ch => t < text.len() && text[t] == ch && glob_match_rec(pattern, p + 1, text, t + 1),
    }
}

/// Spec collectFiles: recursive collection with ignore rules, dotfiles
/// skipped, node_modules skipped.
fn collect_files(dir: &Path, root: &Path, pattern: &str, ig: &IgnoreMatcher, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let full = entry.path();
        let (is_dir, _is_file) = entry_type(&entry, &full);
        let rel = full.strip_prefix(root).unwrap_or(&full).to_string_lossy().replace('\\', "/");
        if ig.ignores(&rel, is_dir) {
            continue;
        }
        if is_dir {
            collect_files(&full, root, pattern, ig, out);
        } else if is_file(&full) && simple_suffix_match(pattern, &name) {
            out.push(full);
        }
    }
}

fn entry_type(entry: &std::fs::DirEntry, full: &Path) -> (bool, bool) {
    let file_type = entry
        .file_type()
        .or_else(|_| std::fs::metadata(full).map(|m| m.file_type()));
    match file_type {
        Ok(file_type) => (file_type.is_dir(), file_type.is_file()),
        Err(_) => (false, false),
    }
}

fn is_file(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

/// Spec FILE_PATTERNS: extensions .lua (divergence: .ts/.js → .lua),
/// skills/prompts .md, themes .json.
fn file_pattern(kind: ResourceType) -> &'static str {
    match kind {
        ResourceType::Extensions => ".lua",
        ResourceType::Skills | ResourceType::Prompts => ".md",
        ResourceType::Themes => ".json",
    }
}

fn simple_suffix_match(pattern: &str, name: &str) -> bool {
    name.ends_with(pattern)
}

/// Spec resolveExtensionEntries: package.json pi.extensions, then init.lua
/// (divergence: index.ts/index.js → init.lua).
fn resolve_extension_entries(dir: &Path) -> Option<Vec<PathBuf>> {
    let package_json = dir.join("package.json");
    if package_json.exists()
        && let Some(manifest) = read_pi_manifest(&package_json)
            && let Some(Value::Array(extensions)) = manifest.get("extensions") {
                let entries: Vec<PathBuf> = extensions
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|entry| dir.join(entry))
                    .filter(|path| path.exists())
                    .collect();
                if !entries.is_empty() {
                    return Some(entries);
                }
            }
    let init = dir.join("init.lua");
    if init.exists() {
        return Some(vec![init]);
    }
    None
}

fn read_pi_manifest(package_json: &Path) -> Option<serde_json::Map<String, Value>> {
    let content = std::fs::read_to_string(package_json).ok()?;
    let parsed: Value = serde_json::from_str(&content).ok()?;
    let manifest = parsed.get("pi")?;
    manifest.as_object().cloned()
}

/// Spec collectAutoExtensionEntries.
fn collect_auto_extension_entries(dir: &Path) -> Vec<PathBuf> {
    if let Some(entries) = resolve_extension_entries(dir) {
        return entries;
    }
    let mut out = Vec::new();
    let ig = IgnoreMatcher::new();
    collect_auto_extension_entries_inner(dir, dir, &ig, &mut out);
    out
}

fn collect_auto_extension_entries_inner(dir: &Path, root: &Path, ig: &IgnoreMatcher, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let full = entry.path();
        let (is_dir, _is_file) = entry_type(&entry, &full);
        let rel = full.strip_prefix(root).unwrap_or(&full).to_string_lossy().replace('\\', "/");
        if ig.ignores(&rel, is_dir) {
            continue;
        }
        if is_file(&full) && full.extension().map(|e| e == "lua").unwrap_or(false) {
            out.push(full);
        } else if is_dir
            && let Some(entries) = resolve_extension_entries(&full) {
                out.extend(entries);
            }
    }
}

/// Spec collectSkillEntries: SKILL.md at each directory; mode "pi" also
/// collects top-level .md files at the root.
fn collect_skill_entries(dir: &Path, root: &Path, mode: &str, ig: &IgnoreMatcher, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "SKILL.md" {
            let full = entry.path();
            let rel = full.strip_prefix(root).unwrap_or(&full).to_string_lossy().replace('\\', "/");
            if is_file(&full) && !ig.ignores(&rel, false) {
                out.push(full);
            }
            return;
        }
    }
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let full = entry.path();
        let (is_dir, _is_file) = entry_type(&entry, &full);
        let rel = full.strip_prefix(root).unwrap_or(&full).to_string_lossy().replace('\\', "/");
        if ig.ignores(&rel, is_dir) {
            continue;
        }
        if mode == "pi" && dir == root && is_file(&full) && full.extension().map(|e| e == "md").unwrap_or(false) {
            out.push(full);
            continue;
        }
        if is_dir {
            collect_skill_entries(&full, root, mode, ig, out);
        }
    }
}

/// Spec collectAutoPromptEntries / collectAutoThemeEntries.
#[allow(dead_code)]
fn collect_flat_entries(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let full = entry.path();
        if is_file(&full) && full.extension().map(|e| e == extension).unwrap_or(false) {
            out.push(full);
        }
    }
    out
}

/// Spec splitPatterns: plain entries vs override patterns.
fn split_patterns(entries: &[String]) -> (Vec<String>, Vec<String>) {
    let mut plain = Vec::new();
    let mut patterns = Vec::new();
    for entry in entries {
        if is_pattern(entry) {
            patterns.push(entry.clone());
        } else {
            plain.push(entry.clone());
        }
    }
    (plain, patterns)
}

fn is_pattern(entry: &str) -> bool {
    entry.starts_with('!')
        || entry.starts_with('+')
        || entry.starts_with('-')
        || entry.contains('*')
        || entry.contains('?')
}

fn is_override_pattern(entry: &str) -> bool {
    entry.starts_with('!') || entry.starts_with('+') || entry.starts_with('-')
}

/// Relative POSIX path from base to file (spec relative()).
fn rel_path(base: &Path, file: &Path) -> String {
    file.strip_prefix(base)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Spec matchesAnyPattern: minimatch against the relative path, the
/// basename, and the absolute path; SKILL.md matches its parent too.
fn matches_any_pattern(file: &Path, patterns: &[String], base_dir: &Path) -> bool {
    let rel = rel_path(base_dir, file);
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let abs = file.to_string_lossy().replace('\\', "/");
    let is_skill = name == "SKILL.md";
    let parent_rel = if is_skill {
        file.parent().and_then(|p| p.strip_prefix(base_dir).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default()
    } else {
        String::new()
    };
    let parent_name = if is_skill {
        file.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        String::new()
    };
    patterns.iter().any(|pattern| {
        if glob_match(pattern, &rel) || glob_match(pattern, &name) || glob_match(pattern, &abs) {
            return true;
        }
        if !is_skill {
            return false;
        }
        glob_match(pattern, &parent_rel) || glob_match(pattern, &parent_name)
    })
}

fn normalize_exact_pattern(pattern: &str) -> String {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    pattern.replace('\\', "/")
}

/// Spec matchesAnyExactPattern.
fn matches_any_exact_pattern(file: &Path, patterns: &[String], base_dir: &Path) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let rel = rel_path(base_dir, file);
    let abs = file.to_string_lossy().replace('\\', "/");
    let is_skill = file.file_name().map(|n| n == "SKILL.md").unwrap_or(false);
    let parent_rel = if is_skill {
        file.parent().and_then(|p| p.strip_prefix(base_dir).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default()
    } else {
        String::new()
    };
    patterns.iter().any(|pattern| {
        let normalized = normalize_exact_pattern(pattern);
        normalized == rel || normalized == abs || (is_skill && (normalized == parent_rel || normalized.ends_with(&format!("/{}", file.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))))
    })
}

/// Spec applyPatterns.
fn apply_patterns(all_paths: &[PathBuf], patterns: &[String], base_dir: &Path) -> HashSet<PathBuf> {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut force_includes = Vec::new();
    let mut force_excludes = Vec::new();
    for pattern in patterns {
        if let Some(rest) = pattern.strip_prefix('+') {
            force_includes.push(rest.to_owned());
        } else if let Some(rest) = pattern.strip_prefix('-') {
            force_excludes.push(rest.to_owned());
        } else if let Some(rest) = pattern.strip_prefix('!') {
            excludes.push(rest.to_owned());
        } else {
            includes.push(pattern.clone());
        }
    }
    let mut result: Vec<PathBuf> = if includes.is_empty() {
        all_paths.to_vec()
    } else {
        all_paths.iter().filter(|file| matches_any_pattern(file, &includes, base_dir)).cloned().collect()
    };
    if !excludes.is_empty() {
        result.retain(|file| !matches_any_pattern(file, &excludes, base_dir));
    }
    if !force_includes.is_empty() {
        for file in all_paths {
            if !result.contains(file) && matches_any_exact_pattern(file, &force_includes, base_dir) {
                result.push(file.clone());
            }
        }
    }
    if !force_excludes.is_empty() {
        result.retain(|file| !matches_any_exact_pattern(file, &force_excludes, base_dir));
    }
    result.into_iter().collect()
}

/// Spec isEnabledByOverrides.
fn is_enabled_by_overrides(file: &Path, patterns: &[String], base_dir: &Path) -> bool {
    let overrides: Vec<String> = patterns.iter().filter(|p| is_override_pattern(p)).cloned().collect();
    let excludes: Vec<String> = overrides.iter().filter(|p| p.starts_with('!')).map(|p| p[1..].to_owned()).collect();
    let force_includes: Vec<String> = overrides.iter().filter(|p| p.starts_with('+')).map(|p| p[1..].to_owned()).collect();
    let force_excludes: Vec<String> = overrides.iter().filter(|p| p.starts_with('-')).map(|p| p[1..].to_owned()).collect();
    let mut enabled = true;
    if !excludes.is_empty() && matches_any_pattern(file, &excludes, base_dir) {
        enabled = false;
    }
    if !force_includes.is_empty() && matches_any_exact_pattern(file, &force_includes, base_dir) {
        enabled = true;
    }
    if !force_excludes.is_empty() && matches_any_exact_pattern(file, &force_excludes, base_dir) {
        enabled = false;
    }
    enabled
}

/// Resource loader: pure resolution over the filesystem + settings.
pub struct ResourceLoader {
    cwd: PathBuf,
    agent_dir: PathBuf,
    settings: SharedSettings,
    packages: PackageManager,
}

impl ResourceLoader {
    #[must_use]
    pub fn new(cwd: &str, agent_dir: &str, settings: SharedSettings) -> Self {
        Self {
            cwd: PathBuf::from(cwd),
            agent_dir: PathBuf::from(agent_dir),
            settings: settings.clone(),
            packages: PackageManager::new(cwd, agent_dir, settings),
        }
    }



    /// Spec resolve(): packages (project first), local entries, then
    /// auto-discovered resources; selectors applied as toggles.
    pub fn resolve(&self, selectors: &BTreeMap<String, (Vec<String>, Vec<String>)>) -> ResolvedPaths {
        // Snapshot the settings once, then release the lock: the package
        // manager and resource helpers may lock the same store (npm_command,
        // project_trusted) and std Mutex is not reentrant.
        let (all, trusted, global, project) = {
            let settings = match self.settings.lock() {
                Ok(settings) => settings,
                Err(_) => return ResolvedPaths::default(),
            };
            let all = crate::packages::configured_package_sources(&settings);
            let trusted = settings.is_project_trusted();
            let global = settings.get_global_settings();
            let project = settings.get_project_settings();
            (all, trusted, global, project)
        };
        let mut accumulator = Accumulator::default();

        // Packages: project first so project wins collisions (spec).
        let deduped = self.packages.dedupe_packages(&all);
        for (source, scope) in deduped {
            self.resolve_package_source(&source, scope, &mut accumulator);
        }

        let global_base = self.agent_dir.clone();
        let project_base = self.cwd.join(crate::discover::CONFIG_DIR_NAME);
        self.resolve_local_entries_from_settings(&global, &project, &mut accumulator, &global_base, &project_base);
        self.add_auto_discovered(&global, &project, &mut accumulator, &global_base, &project_base, trusted);

        let mut paths = accumulator.into_paths();
        apply_selectors(&mut paths, selectors);
        paths
    }

    fn resolve_package_source(&self, source: &str, scope: Scope, accumulator: &mut Accumulator) {
        let parsed = parse_source(source);
        let metadata = PathMetadata {
            source: source.to_owned(),
            scope,
            origin: Origin::Package,
            base_dir: None,
        };
        match &parsed {
            ParsedSource::Local(local) => {
                let base = base_dir_for_scope(scope, &self.cwd, &self.agent_dir);
                let resolved = resolve_path_from_base(&local.path, &base);
                if !resolved.exists() {
                    return;
                }
                if resolved.is_file() {
                    let mut metadata = metadata.clone();
                    metadata.base_dir = resolved.parent().map(Path::to_path_buf);
                    accumulator.add(ResourceType::Extensions, resolved, metadata, true);
                    return;
                }
                if resolved.is_dir() {
                    let mut metadata = metadata.clone();
                    metadata.base_dir = Some(resolved.clone());
                    if !self.collect_package_resources(&resolved, accumulator, &metadata) {
                        accumulator.add(ResourceType::Extensions, resolved, metadata, true);
                    }
                }
            }
            ParsedSource::Npm(source) => {
                let installed = self.packages.get_npm_install_path(source, scope);
                if installed.exists() {
                    let mut metadata = metadata.clone();
                    metadata.base_dir = Some(installed.clone());
                    self.collect_package_resources(&installed, accumulator, &metadata);
                }
            }
            ParsedSource::Git(source) => {
                if let Ok(installed) = self.packages.get_git_install_path(source, scope)
                    && installed.exists()
                {
                    let mut metadata = metadata.clone();
                    metadata.base_dir = Some(installed.clone());
                    self.collect_package_resources(&installed, accumulator, &metadata);
                }
            }
        }
    }

    /// Spec collectPackageResources: pi manifest entries, or convention
    /// resourceType directories.
    fn collect_package_resources(
        &self,
        package_root: &Path,
        accumulator: &mut Accumulator,
        metadata: &PathMetadata,
    ) -> bool {
        let manifest = read_pi_manifest(&package_root.join("package.json"));
        if let Some(manifest) = manifest {
            for kind in ResourceType::all() {
                if let Some(Value::Array(entries)) = manifest.get(kind.name()) {
                    let entries: Vec<String> = entries.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                    self.add_manifest_entries(package_root, kind, &entries, accumulator, metadata);
                }
            }
            return true;
        }
        let mut has_any_dir = false;
        for kind in ResourceType::all() {
            let dir = package_root.join(kind.name());
            if dir.exists() {
                let files = collect_resource_files(&dir, kind);
                for file in files {
                    accumulator.add(kind, file, metadata.clone(), true);
                }
                has_any_dir = true;
            }
        }
        has_any_dir
    }

    fn add_manifest_entries(
        &self,
        root: &Path,
        kind: ResourceType,
        entries: &[String],
        accumulator: &mut Accumulator,
        metadata: &PathMetadata,
    ) {
        let all_files = self.collect_files_from_manifest_entries(root, kind, entries);
        let patterns: Vec<String> = entries.iter().filter(|e| is_override_pattern(e)).cloned().collect();
        let enabled = if patterns.is_empty() {
            all_files.iter().cloned().collect::<HashSet<PathBuf>>()
        } else {
            apply_patterns(&all_files, &patterns, root)
        };
        for file in all_files {
            if enabled.contains(&file) {
                accumulator.add(kind, file, metadata.clone(), true);
            }
        }
    }

    fn collect_files_from_manifest_entries(&self, root: &Path, kind: ResourceType, entries: &[String]) -> Vec<PathBuf> {
        let source_entries: Vec<&String> = entries.iter().filter(|entry| !is_override_pattern(entry)).collect();
        let mut resolved: Vec<PathBuf> = Vec::new();
        for entry in source_entries {
            if !entry.contains('*') && !entry.contains('?') {
                let path = root.join(entry);
                if path.exists() {
                    resolved.push(path);
                }
                continue;
            }
            let mut found = Vec::new();
            glob_expand(root, entry, &mut found);
            resolved.extend(found);
        }
        self.collect_files_from_paths(resolved, kind)
    }

    fn collect_files_from_paths(&self, paths: Vec<PathBuf>, kind: ResourceType) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for path in paths {
            if path.is_file() {
                files.push(path);
            } else if path.is_dir() {
                files.extend(collect_resource_files(&path, kind));
            }
        }
        files
    }

    fn resolve_local_entries_from_settings(
        &self,
        global: &serde_json::Map<String, Value>,
        project: &serde_json::Map<String, Value>,
        accumulator: &mut Accumulator,
        global_base: &Path,
        project_base: &Path,
    ) {
        for kind in ResourceType::all() {
            let key = kind.name();
            let project_entries = string_array(project, key);
            let global_entries = string_array(global, key);
            let target = kind;
            self.resolve_local_entries(
                &project_entries,
                target,
                accumulator,
                PathMetadata {
                    source: "local".to_owned(),
                    scope: Scope::Project,
                    origin: Origin::TopLevel,
                    base_dir: None,
                },
                project_base,
            );
            self.resolve_local_entries(
                &global_entries,
                target,
                accumulator,
                PathMetadata {
                    source: "local".to_owned(),
                    scope: Scope::User,
                    origin: Origin::TopLevel,
                    base_dir: None,
                },
                global_base,
            );
        }
    }

    /// Spec resolveLocalEntries: plain entries collect files, patterns
    /// decide enabled state.
    fn resolve_local_entries(
        &self,
        entries: &[String],
        kind: ResourceType,
        accumulator: &mut Accumulator,
        metadata: PathMetadata,
        base_dir: &Path,
    ) {
        if entries.is_empty() {
            return;
        }
        let (plain, patterns) = split_patterns(entries);
        let resolved: Vec<PathBuf> = plain.iter().map(|p| resolve_path_from_base(p, base_dir)).collect();
        let all_files = self.collect_files_from_paths(resolved, kind);
        let enabled = apply_patterns(&all_files, &patterns, base_dir);
        for file in all_files {
            accumulator.add(kind, file.clone(), metadata.clone(), enabled.contains(&file));
        }
    }

    /// Spec addAutoDiscoveredResources.
    fn add_auto_discovered(
        &self,
        global: &serde_json::Map<String, Value>,
        project: &serde_json::Map<String, Value>,
        accumulator: &mut Accumulator,
        global_base: &Path,
        project_base: &Path,
        trusted: bool,
    ) {
        let user_metadata = PathMetadata {
            source: "auto".to_owned(),
            scope: Scope::User,
            origin: Origin::TopLevel,
            base_dir: Some(global_base.to_path_buf()),
        };
        let project_metadata = PathMetadata {
            source: "auto".to_owned(),
            scope: Scope::Project,
            origin: Origin::TopLevel,
            base_dir: Some(project_base.to_path_buf()),
        };
        let user_overrides: BTreeMap<String, Vec<String>> = ResourceType::all()
            .into_iter()
            .map(|kind| (kind.name().to_owned(), string_array(global, kind.name())))
            .collect();
        let project_overrides: BTreeMap<String, Vec<String>> = ResourceType::all()
            .into_iter()
            .map(|kind| (kind.name().to_owned(), string_array(project, kind.name())))
            .collect();

        if trusted {
            self.add_auto(ResourceType::Extensions, &project_base.join("extensions"), accumulator, &project_metadata, &project_overrides);
            self.add_auto(ResourceType::Skills, &project_base.join("skills"), accumulator, &project_metadata, &project_overrides);
            self.add_auto(ResourceType::Prompts, &project_base.join("prompts"), accumulator, &project_metadata, &project_overrides);
            self.add_auto(ResourceType::Themes, &project_base.join("themes"), accumulator, &project_metadata, &project_overrides);
        }
        self.add_auto(ResourceType::Extensions, &global_base.join("extensions"), accumulator, &user_metadata, &user_overrides);
        self.add_auto(ResourceType::Skills, &global_base.join("skills"), accumulator, &user_metadata, &user_overrides);
        self.add_auto(ResourceType::Prompts, &global_base.join("prompts"), accumulator, &user_metadata, &user_overrides);
        self.add_auto(ResourceType::Themes, &global_base.join("themes"), accumulator, &user_metadata, &user_overrides);
    }

    fn add_auto(
        &self,
        kind: ResourceType,
        dir: &Path,
        accumulator: &mut Accumulator,
        metadata: &PathMetadata,
        overrides: &BTreeMap<String, Vec<String>>,
    ) {
        let files = collect_resource_files(dir, kind);
        let patterns = overrides.get(kind.name()).cloned().unwrap_or_default();
        let base = metadata.base_dir.clone().unwrap_or_else(|| dir.to_path_buf());
        for file in files {
            let enabled = is_enabled_by_overrides(&file, &patterns, &base);
            accumulator.add(kind, file, metadata.clone(), enabled);
        }
    }
}

fn collect_resource_files(dir: &Path, kind: ResourceType) -> Vec<PathBuf> {
    match kind {
        ResourceType::Skills => {
            let mut out = Vec::new();
            let ig = IgnoreMatcher::new();
            collect_skill_entries(dir, dir, "pi", &ig, &mut out);
            out
        }
        ResourceType::Extensions => collect_auto_extension_entries(dir),
        _ => {
            let mut out = Vec::new();
            let ig = IgnoreMatcher::new();
            collect_files(dir, dir, file_pattern(kind), &ig, &mut out);
            out
        }
    }
}

/// Spec globSync subset: **, *, ? against relative paths under root.
#[allow(clippy::only_used_in_recursion)] // root forwarded to the nested walk; kept for the public signature
fn glob_expand(root: &Path, pattern: &str, out: &mut Vec<PathBuf>) {
    let normalized = pattern.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').collect();
    fn walk(root: &Path, segments: &[&str], current: &Path, rel: &str, out: &mut Vec<PathBuf>) {
        if segments.is_empty() {
            out.push(current.to_path_buf());
            return;
        }
        let segment = segments[0];
        if segment == "**" {
            walk(root, &segments[1..], current, rel, out);
            let Ok(entries) = std::fs::read_dir(current) else {
                return;
            };
            let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let next_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                    walk(root, segments, &entry.path(), &next_rel, out);
                }
            }
            return;
        }
        let Ok(entries) = std::fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !glob_match(segment, &name) {
                continue;
            }
            let next_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if segments.len() == 1 {
                out.push(entry.path());
            } else if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(root, &segments[1..], &entry.path(), &next_rel, out);
            }
        }
    }
    walk(root, &segments, root, "", out);
}

fn string_array(settings: &serde_json::Map<String, Value>, key: &str) -> Vec<String> {
    settings
        .get(key)
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Spec ResourceAccumulator.
#[derive(Default)]
struct Accumulator {
    extensions: Vec<ResolvedResource>,
    skills: Vec<ResolvedResource>,
    prompts: Vec<ResolvedResource>,
    themes: Vec<ResolvedResource>,
}

impl Accumulator {
    fn add(&mut self, kind: ResourceType, path: PathBuf, metadata: PathMetadata, enabled: bool) {
        if path.as_os_str().is_empty() {
            return;
        }
        let resource = ResolvedResource {
            path: path.to_string_lossy().into_owned(),
            enabled,
            metadata,
        };
        let target = match kind {
            ResourceType::Extensions => &mut self.extensions,
            ResourceType::Skills => &mut self.skills,
            ResourceType::Prompts => &mut self.prompts,
            ResourceType::Themes => &mut self.themes,
        };
        if !target.iter().any(|existing| existing.path == resource.path) {
            target.push(resource);
        }
    }

    fn into_paths(self) -> ResolvedPaths {
        ResolvedPaths {
            extensions: to_resolved(self.extensions),
            skills: to_resolved(self.skills),
            prompts: to_resolved(self.prompts),
            themes: to_resolved(self.themes),
        }
    }
}

/// Apply config selectors (kind → (enabled, disabled)) as toggles: an
/// enabled match forces enabled=true, a disabled match forces false.
fn apply_selectors(paths: &mut ResolvedPaths, selectors: &BTreeMap<String, (Vec<String>, Vec<String>)>) {
    for kind in ResourceType::all() {
        let Some((enabled_patterns, disabled_patterns)) = selectors.get(kind.name()) else {
            continue;
        };
        if enabled_patterns.is_empty() && disabled_patterns.is_empty() {
            continue;
        }
        for resource in paths.for_type_mut(kind) {
            let file = Path::new(&resource.path);
            // Exact patterns resolve against the resource's own base dir
            // (agent dir for user auto, project .pi for project auto,
            // package root for package resources), falling back to the
            // absolute path when no base dir is recorded.
            let base = resource.metadata.base_dir.as_deref().unwrap_or_else(|| Path::new(""));
            if !enabled_patterns.is_empty() && matches_any_exact_pattern(file, enabled_patterns, base) {
                resource.enabled = true;
            }
            if !disabled_patterns.is_empty() && matches_any_exact_pattern(file, disabled_patterns, base) {
                resource.enabled = false;
            }
        }
    }
}

impl ResolvedPaths {
    fn for_type_mut(&mut self, kind: ResourceType) -> &mut Vec<ResolvedResource> {
        match kind {
            ResourceType::Extensions => &mut self.extensions,
            ResourceType::Skills => &mut self.skills,
            ResourceType::Prompts => &mut self.prompts,
            ResourceType::Themes => &mut self.themes,
        }
    }
}

// ---------------------------------------------------------------------------
// Lua surface: pi.packages (package manager + resource resolution + the
// declared-resource registry for embedded defaults and config themes).
// ---------------------------------------------------------------------------

const RESOURCE_REGISTRY_KEY: &str = "pi_resource_declarations";

fn resource_registry(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    if let Ok(table) = lua.named_registry_value::<mlua::Table>(RESOURCE_REGISTRY_KEY) {
        return Ok(table);
    }
    let table = lua.create_table()?;
    lua.set_named_registry_value(RESOURCE_REGISTRY_KEY, &table)?;
    Ok(table)
}

/// Default rank for embedded/package declarations (spec precedence 4).
const EMBEDDED_RANK: u8 = 4;

/// Declare one resource into the per-VM registry. `rank` follows Pi's
/// precedence ladder: 0 project-local, 1 project-auto, 2 user-local,
/// 3 user-auto, 4 package. `group` distinguishes config-declared entries
/// (replaced on reload) from package declarations (persistent).
pub fn declare_resource(
    lua: &mlua::Lua,
    kind: &str,
    name: &str,
    value: mlua::Value,
    source: &str,
    rank: u8,
    group: &str,
) -> mlua::Result<()> {
    let table = resource_registry(lua)?;
    let entry = lua.create_table()?;
    entry.set("kind", kind)?;
    entry.set("name", name)?;
    entry.set("value", value)?;
    entry.set("source", source)?;
    entry.set("rank", rank)?;
    entry.set("group", group)?;
    table.push(entry)?;
    Ok(())
}

/// Drop every config-group declaration (used by config.reload so the next
/// theme graph replaces the previous one atomically).
pub fn clear_config_resources(lua: &mlua::Lua) -> mlua::Result<()> {
    let table = resource_registry(lua)?;
    let kept = lua.create_table()?;
    for entry in table.sequence_values::<mlua::Table>() {
        let entry = entry?;
        let group: String = entry.get("group")?;
        if group != "config" {
            kept.push(entry)?;
        }
    }
    for index in (1..=table.raw_len()).rev() {
        table.raw_remove(index)?;
    }
    for entry in kept.sequence_values::<mlua::Table>() {
        table.push(entry?)?;
    }
    Ok(())
}

/// Highest-precedence declaration for (kind, name), or None.
fn resource_value(lua: &mlua::Lua, kind: &str, name: &str) -> mlua::Result<Option<mlua::Value>> {
    let table = resource_registry(lua)?;
    let mut best: Option<(u8, usize, mlua::Value)> = None;
    for (index, entry) in table.sequence_values::<mlua::Table>().enumerate() {
        let entry = entry?;
        let entry_kind: String = entry.get("kind")?;
        let entry_name: String = entry.get("name")?;
        if entry_kind != kind || entry_name != name {
            continue;
        }
        let rank: u8 = entry.get("rank")?;
        let take = match &best {
            None => true,
            Some((best_rank, best_index, _)) => rank < *best_rank || (rank == *best_rank && index < *best_index),
        };
        if take {
            best = Some((rank, index, entry.get("value")?));
        }
    }
    Ok(best.map(|(_, _, value)| value))
}

/// All declarations of a kind, sorted by rank (stable), deduped by name
/// with first-wins collisions.
fn all_resources(lua: &mlua::Lua, kind: &str) -> mlua::Result<Vec<(String, mlua::Value, String, u8)>> {
    let table = resource_registry(lua)?;
    let mut entries: Vec<(String, mlua::Value, String, u8, usize)> = Vec::new();
    for (index, entry) in table.sequence_values::<mlua::Table>().enumerate() {
        let entry = entry?;
        let entry_kind: String = entry.get("kind")?;
        if entry_kind != kind {
            continue;
        }
        let name: String = entry.get("name")?;
        let source: String = entry.get("source")?;
        let rank: u8 = entry.get("rank")?;
        let value = entry.get("value")?;
        entries.push((name, value, source, rank, index));
    }
    entries.sort_by_key(|(_, _, _, rank, index)| (*rank, *index));
    let mut seen = std::collections::HashSet::new();
    Ok(entries
        .into_iter()
        .filter(|(name, _, _, _, _)| seen.insert(name.clone()))
        .map(|(name, value, source, rank, _)| (name, value, source, rank))
        .collect())
}

fn parse_selectors(
    _lua: &mlua::Lua,
    value: Option<mlua::Value>,
) -> mlua::Result<ResourceIndex> {
    let mut out = BTreeMap::new();
    let Some(value) = value else {
        return Ok(out);
    };
    let mlua::Value::Table(table) = value else {
        return Err(mlua::Error::runtime("selectors must be a table"));
    };
    for pair in table.pairs::<String, mlua::Table>() {
        let (kind, selector) = pair?;
        let enabled: Vec<String> = selector
            .get::<Option<mlua::Table>>("enabled")?
            .map(|t| t.sequence_values::<String>().collect::<mlua::Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default();
        let disabled: Vec<String> = selector
            .get::<Option<mlua::Table>>("disabled")?
            .map(|t| t.sequence_values::<String>().collect::<mlua::Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default();
        out.insert(kind, (enabled, disabled));
    }
    Ok(out)
}

fn resolved_to_lua(
    lua: &mlua::Lua,
    resources: &[crate::resource_loader::ResolvedResource],
) -> mlua::Result<mlua::Table> {
    let out = lua.create_table()?;
    for resource in resources {
        let entry = lua.create_table()?;
        entry.set("path", resource.path.as_str())?;
        entry.set("enabled", resource.enabled)?;
        entry.set("source", resource.metadata.source.as_str())?;
        entry.set("scope", resource.metadata.scope.name())?;
        entry.set(
            "origin",
            if resource.metadata.origin == crate::resource_loader::Origin::Package {
                "package"
            } else {
                "top-level"
            },
        )?;
        if let Some(base) = &resource.metadata.base_dir {
            entry.set("base_dir", base.to_string_lossy().into_owned())?;
        }
        out.push(entry)?;
    }
    Ok(out)
}

/// Install pi.packages: package manager + resource resolution + the
/// declared-resource registry used by embedded defaults and config themes.
pub fn install(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    cwd: &str,
    agent_dir: &str,
    _project_trusted: bool,
    settings: SharedSettings,
) -> mlua::Result<()> {
    // The parameter is consumed by the closures registered below.
    let _ = lua;
    resource_registry(lua)?;

    let packages = lua.create_table()?;

    {
        packages.set(
            "declare_resource",
            lua.create_function(move |lua, (kind, name, value): (String, String, mlua::Value)| {
                if kind.is_empty() || name.is_empty() {
                    return Err(mlua::Error::runtime("declare_resource kind and name must be non-empty"));
                }
                let source = crate::api::current_source(lua);
                declare_resource(lua, &kind, &name, value, &source, EMBEDDED_RANK, "package")
            })?,
        )?;
    }
    {
        packages.set(
            "resource",
            lua.create_function(move |lua, (kind, name): (String, String)| resource_value(lua, &kind, &name))?,
        )?;
    }
    {
        packages.set(
            "all_resources",
            lua.create_function(move |lua, kind: String| {
                let entries = all_resources(lua, &kind)?;
                let out = lua.create_table()?;
                for (name, value, source, rank) in entries {
                    let entry = lua.create_table()?;
                    entry.set("name", name)?;
                    entry.set("value", value)?;
                    entry.set("source", source)?;
                    entry.set("rank", rank)?;
                    out.push(entry)?;
                }
                Ok(out)
            })?,
        )?;
    }

    let loader = crate::resource_loader::ResourceLoader::new(cwd, agent_dir, settings.clone());
    {
        let loader = loader.clone_loader();
        packages.set(
            "resolve",
            lua.create_function(move |lua, selectors: Option<mlua::Value>| {
                let selectors = parse_selectors(lua, selectors)?;
                let paths = loader.resolve(&selectors);
                let out = lua.create_table()?;
                out.set("extensions", resolved_to_lua(lua, &paths.extensions)?)?;
                out.set("skills", resolved_to_lua(lua, &paths.skills)?)?;
                out.set("prompts", resolved_to_lua(lua, &paths.prompts)?)?;
                out.set("themes", resolved_to_lua(lua, &paths.themes)?)?;
                Ok(out)
            })?,
        )?;
    }
    {
        let loader = loader.clone_loader();
        packages.set(
            "resolve_extension_sources",
            lua.create_function(
                move |lua, (sources, options): (Vec<String>, Option<mlua::Table>)| {
                    let local = options
                        .as_ref()
                        .map(|t| t.get::<Option<bool>>("local"))
                        .transpose()?
                        .flatten()
                        .unwrap_or(false);
                    let temporary = options
                        .as_ref()
                        .map(|t| t.get::<Option<bool>>("temporary"))
                        .transpose()?
                        .flatten()
                        .unwrap_or(false);
                    let scope = if temporary {
                        Scope::Temporary
                    } else if local {
                        Scope::Project
                    } else {
                        Scope::User
                    };
                    let paths = loader.resolve_extension_sources(&sources, scope);
                    let out = lua.create_table()?;
                    out.set("extensions", resolved_to_lua(lua, &paths.extensions)?)?;
                    out.set("skills", resolved_to_lua(lua, &paths.skills)?)?;
                    out.set("prompts", resolved_to_lua(lua, &paths.prompts)?)?;
                    out.set("themes", resolved_to_lua(lua, &paths.themes)?)?;
                    Ok(out)
                },
            )?,
        )?;
    }

    let manager = crate::packages::PackageManager::new(cwd, agent_dir, settings);
    let scope_from_options = std::sync::Arc::new(
        move |_lua: &mlua::Lua, options: Option<mlua::Table>| -> mlua::Result<Scope> {
            let local = options
                .as_ref()
                .map(|t| t.get::<Option<bool>>("local"))
                .transpose()?
                .flatten()
                .unwrap_or(false);
            Ok(if local { Scope::Project } else { Scope::User })
        },
    );

    macro_rules! manager_method {
        ($lua_name:literal, $method:ident) => {{
            let manager = manager.clone_manager();
            let scope_from_options = std::sync::Arc::clone(&scope_from_options);
            packages.set(
                $lua_name,
                lua.create_function(move |lua, (source, options): (String, Option<mlua::Table>)| {
                    let scope = scope_from_options(lua, options)?;
                    manager
                        .$method(&source, scope)
                        .map_err(mlua::Error::runtime)
                })?,
            )?;
        }};
    }
    manager_method!("install", install);
    manager_method!("install_and_persist", install_and_persist);
    manager_method!("remove", remove);
    manager_method!("remove_and_persist", remove_and_persist);

    {
        let manager = manager.clone_manager();
        packages.set(
            "update",
            lua.create_function(move |_, source: Option<String>| {
                manager.update(source.as_deref()).map_err(mlua::Error::runtime)
            })?,
        )?;
    }
    {
        let manager = manager.clone_manager();
        packages.set(
            "list",
            lua.create_function(move |lua, ()| {
                let configured = manager.list();
                let out = lua.create_table()?;
                for entry in configured {
                    let item = lua.create_table()?;
                    item.set("source", entry.source)?;
                    item.set("scope", entry.scope.name())?;
                    item.set("filtered", entry.filtered)?;
                    item.set("installed_path", entry.installed_path)?;
                    out.push(item)?;
                }
                Ok(out)
            })?,
        )?;
    }
    {
        let manager = manager.clone_manager();
        packages.set(
            "get_installed_path",
            lua.create_function(move |_, (source, scope): (String, String)| {
                let scope = match scope.as_str() {
                    "user" => Scope::User,
                    "project" => Scope::Project,
                    "temporary" => Scope::Temporary,
                    _ => return Err(mlua::Error::runtime(format!("unknown scope: {scope}"))),
                };
                Ok(manager.get_installed_path(&source, scope))
            })?,
        )?;
    }
    {
        let manager = manager.clone_manager();
        let scope_from_options = std::sync::Arc::clone(&scope_from_options);
        packages.set(
            "add_source_to_settings",
            lua.create_function(move |lua, (source, options): (String, Option<mlua::Table>)| {
                let scope = scope_from_options(lua, options)?;
                Ok(manager.add_source_to_settings(&source, scope))
            })?,
        )?;
    }
    {
        let manager = manager.clone_manager();
        let scope_from_options = std::sync::Arc::clone(&scope_from_options);
        packages.set(
            "remove_source_from_settings",
            lua.create_function(move |lua, (source, options): (String, Option<mlua::Table>)| {
                let scope = scope_from_options(lua, options)?;
                Ok(manager.remove_source_from_settings(&source, scope))
            })?,
        )?;
    }

    pi.set("packages", packages)?;
    Ok(())
}

impl ResourceLoader {
    #[must_use]
    pub fn clone_loader(&self) -> Self {
        Self {
            cwd: self.cwd.clone(),
            agent_dir: self.agent_dir.clone(),
            settings: self.settings.clone(),
            packages: self.packages.clone_manager(),
        }
    }

    /// Spec resolveExtensionSources.
    pub fn resolve_extension_sources(&self, sources: &[String], scope: Scope) -> ResolvedPaths {
        let mut accumulator = Accumulator::default();
        for source in sources {
            self.resolve_package_source(source, scope, &mut accumulator);
        }
        accumulator.into_paths()
    }
}

