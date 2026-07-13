//! Pi-compatible settings outcomes over pi-rs's canonical Lua configuration.
//!
//! Global declarations live in `<agentDir>/config.lua`; trusted project
//! declarations in `<cwd>/.pi/config.lua` override them with Pi's one-level
//! nested merge. Interactive setters update one generated block atomically
//! while preserving all user-authored bytes outside that block. JSON settings
//! files are never read or written.
//!
//! The typed accessors retain the pinned SettingsManager outcome contract.
//! Writes are synchronous in this port; [`SettingsManager::flush`] is a no-op.
//! Every reload evaluates both scopes before publishing either, so a syntax or
//! runtime failure keeps the previous complete live snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use pi_rs_ai_types::ModelThinkingLevel;
use serde_json::Value;
use thiserror::Error;

use crate::discover::CONFIG_DIR_NAME;

/// Spec: `expandTildePath` (via `normalizePath`) — `~` and `~/...`
/// expand to the home directory (`utils/paths.ts`; the coding agent's
/// `config.ts` re-exports it).
pub fn expand_tilde_path(path: &str) -> PathBuf {
    let home = || PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_owned()));
    if path == "~" {
        return home();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home().join(rest);
    }
    PathBuf::from(path)
}

fn get_agent_dir() -> PathBuf {
    PathBuf::from(crate::discover::agent_dir())
}

/// Spec: `DEFAULT_HTTP_IDLE_TIMEOUT_MS = 300_000` (`core/http-dispatcher.ts`;
/// hosted here because the settings port consumes it — `pi-rs-app`'s
/// spec-shaped `core::http_dispatcher` re-exports it).
pub const DEFAULT_HTTP_IDLE_TIMEOUT_MS: u64 = 300_000;

fn parse_http_idle_timeout_number(value: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(value.floor() as u64)
}

/// Spec: `parseHttpIdleTimeoutMs(value)` (`core/http-dispatcher.ts`) —
/// `"disabled"` → 0, numeric strings via `Number()`, non-negative finite
/// numbers floored; everything else `undefined`.
pub fn parse_http_idle_timeout_ms(value: &Value) -> Option<u64> {
    match value {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.eq_ignore_ascii_case("disabled") {
                return Some(0);
            }
            if trimmed.is_empty() {
                return None;
            }
            parse_http_idle_timeout_number(trimmed.parse::<f64>().ok()?)
        }
        Value::Number(n) => parse_http_idle_timeout_number(n.as_f64()?),
        _ => None,
    }
}

/// Spec: `isValidThinkingLevel` (`cli/args.ts`; hosted here because the
/// settings port consumes it — `pi-rs-app`'s spec-shaped
/// `core::model_resolver` re-exports it).
pub fn parse_thinking_level(level: &str) -> Option<ModelThinkingLevel> {
    match level {
        "off" => Some(ModelThinkingLevel::Off),
        "minimal" => Some(ModelThinkingLevel::Minimal),
        "low" => Some(ModelThinkingLevel::Low),
        "medium" => Some(ModelThinkingLevel::Medium),
        "high" => Some(ModelThinkingLevel::High),
        "xhigh" => Some(ModelThinkingLevel::XHigh),
        "max" => Some(ModelThinkingLevel::Max),
        _ => None,
    }
}

/// Spec: `Settings` — a JSON object; typed access is method-level.
pub type Settings = serde_json::Map<String, Value>;

/// Spec: `SettingsScope`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsScope {
    Global,
    Project,
}

impl std::fmt::Display for SettingsScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsScope::Global => f.write_str("global"),
            SettingsScope::Project => f.write_str("project"),
        }
    }
}

/// Errors from the storage/parse layer. The manager records most of
/// these via `recordError` and keeps going (spec behavior); `Err` is
/// reserved for the operations the spec lets throw (project writes
/// while untrusted, invalid timeout settings).
#[derive(Debug, Error)]
pub enum SettingsManagerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

/// Spec: `SettingsError`.
#[derive(Clone, Debug)]
pub struct SettingsError {
    pub scope: SettingsScope,
    pub error: String,
}

/// Spec: `SettingsManagerCreateOptions`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SettingsManagerCreateOptions {
    /// Defaults to `true` (spec: `options.projectTrusted ?? true`).
    pub project_trusted: Option<bool>,
}

// ---------------------------------------------------------------------------
// Storage backends
// ---------------------------------------------------------------------------

/// Held mkdir lock (`<config.lua>.lock`); removed on drop.
struct FileLock(PathBuf);

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Spec: `proper-lockfile`'s default `stale: 10000` — a lock directory
/// older than 10s is abandoned and may be broken.
fn break_stale_lock(lock_path: &Path) {
    let Ok(meta) = std::fs::metadata(lock_path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    if modified
        .elapsed()
        .map(|age| age >= Duration::from_millis(10_000))
        .unwrap_or(false)
    {
        let _ = std::fs::remove_dir(lock_path);
    }
}

fn try_acquire(lock_path: &Path) -> Option<FileLock> {
    break_stale_lock(lock_path);
    match std::fs::create_dir(lock_path) {
        Ok(()) => Some(FileLock(lock_path.to_path_buf())),
        Err(_) => None,
    }
}

/// Acquire the per-config mutation lock (10 attempts, 20ms apart).
fn acquire_lock_sync_with_retry(path: &Path) -> Result<FileLock, SettingsManagerError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.lua");
    let lock_path = path.with_file_name(format!("{file_name}.lock"));
    for attempt in 1..=10u32 {
        if let Some(lock) = try_acquire(&lock_path) {
            return Ok(lock);
        }
        if attempt < 10 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    Err(SettingsManagerError::Message(format!(
        "Failed to acquire config lock: {}",
        lock_path.display()
    )))
}

/// Spec: `SettingsStorage` — `FileSettingsStorage` +
/// `InMemorySettingsStorage` (the two implementations the spec ships,
/// collapsed to an enum as in `auth_storage`).
pub enum SettingsStorage {
    File {
        global_settings_path: PathBuf,
        project_settings_path: PathBuf,
    },
    InMemory {
        global: Option<String>,
        project: Option<String>,
    },
}

impl SettingsStorage {
    /// Canonical Lua configuration paths.
    pub fn file(cwd: &Path, agent_dir: &Path) -> Self {
        SettingsStorage::File {
            global_settings_path: agent_dir.join("config.lua"),
            project_settings_path: cwd.join(CONFIG_DIR_NAME).join("config.lua"),
        }
    }

    /// Spec: `new InMemorySettingsStorage()`.
    pub fn in_memory() -> Self {
        SettingsStorage::InMemory {
            global: None,
            project: None,
        }
    }

    /// Spec: `withLock(scope, fn)` — the closure receives the current
    /// content and returns the next content (or `None` to skip the
    /// write). The file backend only locks/creates directories when the
    /// file exists or a write is needed.
    pub fn with_lock(
        &mut self,
        scope: SettingsScope,
        f: impl FnOnce(Option<&str>) -> Result<Option<String>, SettingsManagerError>,
    ) -> Result<(), SettingsManagerError> {
        match self {
            SettingsStorage::File {
                global_settings_path,
                project_settings_path,
            } => {
                let path = match scope {
                    SettingsScope::Global => global_settings_path,
                    SettingsScope::Project => project_settings_path,
                };
                let file_exists = path.exists();
                let mut lock = if file_exists {
                    Some(acquire_lock_sync_with_retry(path)?)
                } else {
                    None
                };
                let current = if file_exists {
                    Some(std::fs::read_to_string(&path)?)
                } else {
                    None
                };
                let next = f(current.as_deref())?;
                if let Some(next) = next {
                    if let Some(dir) = path.parent()
                        && !dir.exists()
                    {
                        std::fs::create_dir_all(dir)?;
                    }
                    if lock.is_none() {
                        lock = Some(acquire_lock_sync_with_retry(path)?);
                    }
                    let temp_path = path.with_file_name(format!(
                        ".{}.tmp",
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("config.lua")
                    ));
                    std::fs::write(&temp_path, next)?;
                    std::fs::rename(&temp_path, path)?;
                }
                drop(lock);
                Ok(())
            }
            SettingsStorage::InMemory { global, project } => {
                let slot = match scope {
                    SettingsScope::Global => global,
                    SettingsScope::Project => project,
                };
                let next = f(slot.as_deref())?;
                if let Some(next) = next {
                    *slot = Some(next);
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Merge + migration
// ---------------------------------------------------------------------------

/// Spec: `deepMergeSettings` — overrides win; nested objects merge one
/// level (`{ ...base, ...override }`); arrays and primitives replace.
pub fn deep_merge_settings(base: &Settings, overrides: &Settings) -> Settings {
    let mut result = base.clone();
    for (key, override_value) in overrides {
        if let (Some(Value::Object(base_obj)), Value::Object(over_obj)) =
            (base.get(key), override_value)
        {
            let mut merged = base_obj.clone();
            for (k, v) in over_obj {
                merged.insert(k.clone(), v.clone());
            }
            result.insert(key.clone(), Value::Object(merged));
        } else {
            result.insert(key.clone(), override_value.clone());
        }
    }
    result
}

/// Spec: `migrateSettings` — old settings formats to new.
fn migrate_settings(settings: &mut Settings) {
    // Migrate queueMode -> steeringMode
    if settings.contains_key("queueMode")
        && !settings.contains_key("steeringMode")
        && let Some(value) = settings.remove("queueMode")
    {
        settings.insert("steeringMode".to_owned(), value);
    }

    // Migrate legacy websockets boolean -> transport enum
    if !settings.contains_key("transport")
        && let Some(Value::Bool(websockets)) = settings.get("websockets")
    {
        let transport = if *websockets { "websocket" } else { "sse" };
        settings.insert("transport".to_owned(), Value::String(transport.to_owned()));
        settings.remove("websockets");
    }

    // Migrate old skills object format to new array format
    if let Some(Value::Object(skills)) = settings.get("skills").cloned() {
        if let Some(enable) = skills.get("enableSkillCommands")
            && !settings.contains_key("enableSkillCommands")
        {
            settings.insert("enableSkillCommands".to_owned(), enable.clone());
        }
        match skills.get("customDirectories") {
            Some(Value::Array(dirs)) if !dirs.is_empty() => {
                settings.insert("skills".to_owned(), Value::Array(dirs.clone()));
            }
            _ => {
                settings.remove("skills");
            }
        }
    }

    // Migrate retry.maxDelayMs -> retry.provider.maxRetryDelayMs
    if let Some(Value::Object(retry)) = settings.get_mut("retry") {
        if let Some(max_delay @ Value::Number(_)) = retry.get("maxDelayMs").cloned() {
            let provider_has_value = retry
                .get("provider")
                .and_then(Value::as_object)
                .and_then(|p| p.get("maxRetryDelayMs"))
                .is_some_and(|v| !v.is_null());
            if !provider_has_value {
                let mut provider = retry
                    .get("provider")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                provider.insert("maxRetryDelayMs".to_owned(), max_delay);
                retry.insert("provider".to_owned(), Value::Object(provider));
            }
        }
        retry.remove("maxDelayMs");
    }
}

/// Spec: `parseTimeoutSetting(value, settingName)` — absent is `None`;
/// present-but-invalid throws.
fn parse_timeout_setting(
    value: Option<&Value>,
    setting_name: &str,
) -> Result<Option<u64>, SettingsManagerError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match parse_http_idle_timeout_ms(value) {
        Some(timeout_ms) => Ok(Some(timeout_ms)),
        None => Err(SettingsManagerError::Message(format!(
            "Invalid {setting_name} setting: {value}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Typed accessor result shapes
// ---------------------------------------------------------------------------

/// Spec: `getCompactionSettings()` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}

/// Spec: `getBranchSummarySettings()` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchSummarySettings {
    pub reserve_tokens: u64,
    pub skip_prompt: bool,
}

/// Spec: `getRetrySettings()` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetrySettings {
    pub enabled: bool,
    pub max_retries: u64,
    pub base_delay_ms: u64,
}

/// Spec: `getProviderRetrySettings()` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderRetrySettings {
    pub timeout_ms: Option<u64>,
    pub max_retries: Option<u64>,
    pub max_retry_delay_ms: u64,
}

// ---------------------------------------------------------------------------
// SettingsManager
// ---------------------------------------------------------------------------

/// Spec: `SettingsManager`.
pub struct SettingsManager {
    storage: SettingsStorage,
    global_settings: Settings,
    project_settings: Settings,
    settings: Settings,
    project_trusted: bool,
    modified_fields: BTreeSet<String>,
    modified_nested_fields: BTreeMap<String, BTreeSet<String>>,
    modified_project_fields: BTreeSet<String>,
    modified_project_nested_fields: BTreeMap<String, BTreeSet<String>>,
    global_load_error: Option<String>,
    project_load_error: Option<String>,
    errors: Vec<SettingsError>,
}

impl SettingsManager {
    /// Spec: `SettingsManager.create(cwd, agentDir?, options?)`.
    pub fn create(
        cwd: &Path,
        agent_dir: Option<PathBuf>,
        options: SettingsManagerCreateOptions,
    ) -> Self {
        let agent_dir = agent_dir.unwrap_or_else(get_agent_dir);
        Self::from_storage(SettingsStorage::file(cwd, &agent_dir), options)
    }

    /// Spec: `SettingsManager.fromStorage(storage, options?)`.
    pub fn from_storage(
        mut storage: SettingsStorage,
        options: SettingsManagerCreateOptions,
    ) -> Self {
        let project_trusted = options.project_trusted.unwrap_or(true);
        let (global_settings, global_load_error) =
            Self::try_load_from_storage(&mut storage, SettingsScope::Global, true);
        let (project_settings, project_load_error) =
            Self::try_load_from_storage(&mut storage, SettingsScope::Project, project_trusted);

        let mut errors = Vec::new();
        if let Some(error) = &global_load_error {
            errors.push(SettingsError {
                scope: SettingsScope::Global,
                error: error.clone(),
            });
        }
        if let Some(error) = &project_load_error {
            errors.push(SettingsError {
                scope: SettingsScope::Project,
                error: error.clone(),
            });
        }

        let settings = deep_merge_settings(&global_settings, &project_settings);
        Self {
            storage,
            global_settings,
            project_settings,
            settings,
            project_trusted,
            modified_fields: BTreeSet::new(),
            modified_nested_fields: BTreeMap::new(),
            modified_project_fields: BTreeSet::new(),
            modified_project_nested_fields: BTreeMap::new(),
            global_load_error,
            project_load_error,
            errors,
        }
    }

    /// Create an in-memory manager through the same Lua declaration format.
    pub fn in_memory(settings: Settings) -> Self {
        let mut storage = SettingsStorage::in_memory();
        let mut initial = settings;
        migrate_settings(&mut initial);
        let source = crate::config::update_managed_settings("", &initial);
        let _ = storage.with_lock(SettingsScope::Global, |_| Ok(Some(source)));
        Self::from_storage(storage, SettingsManagerCreateOptions::default())
    }

    fn load_from_storage(
        storage: &mut SettingsStorage,
        scope: SettingsScope,
        project_trusted: bool,
    ) -> Result<Settings, SettingsManagerError> {
        if scope == SettingsScope::Project && !project_trusted {
            return Ok(Settings::new());
        }

        let mut content: Option<String> = None;
        storage.with_lock(scope, |current| {
            content = current.map(str::to_owned);
            Ok(None)
        })?;

        let Some(content) = content else {
            return Ok(Settings::new());
        };
        if content.is_empty() {
            return Ok(Settings::new());
        }
        let source_name = match scope {
            SettingsScope::Global => "global config.lua",
            SettingsScope::Project => "project .pi/config.lua",
        };
        let mut settings = crate::config::evaluate(&content, source_name)
            .map_err(SettingsManagerError::Message)?
            .settings;
        migrate_settings(&mut settings);
        Ok(settings)
    }

    /// Spec: `tryLoadFromStorage`.
    fn try_load_from_storage(
        storage: &mut SettingsStorage,
        scope: SettingsScope,
        project_trusted: bool,
    ) -> (Settings, Option<String>) {
        match Self::load_from_storage(storage, scope, project_trusted) {
            Ok(settings) => (settings, None),
            Err(error) => (Settings::new(), Some(error.to_string())),
        }
    }

    /// Spec: `getGlobalSettings()`.
    pub fn get_global_settings(&self) -> Settings {
        self.global_settings.clone()
    }

    /// Spec: `getProjectSettings()`.
    pub fn get_project_settings(&self) -> Settings {
        self.project_settings.clone()
    }

    /// Spec: `isProjectTrusted()`.
    pub fn is_project_trusted(&self) -> bool {
        self.project_trusted
    }

    /// Spec: `setProjectTrusted(trusted)`.
    pub fn set_project_trusted(&mut self, trusted: bool) {
        if self.project_trusted == trusted {
            return;
        }

        self.project_trusted = trusted;
        self.modified_project_fields.clear();
        self.modified_project_nested_fields.clear();

        if !trusted {
            self.project_settings = Settings::new();
            self.project_load_error = None;
            self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);
            return;
        }

        let (project_settings, project_load_error) =
            Self::try_load_from_storage(&mut self.storage, SettingsScope::Project, trusted);
        self.project_settings = project_settings;
        if let Some(error) = &project_load_error {
            self.record_error(SettingsScope::Project, error);
        }
        self.project_load_error = project_load_error;
        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);
    }

    /// Atomically reload both declaration scopes. Either both publish or neither does.
    pub fn try_reload(&mut self) -> Result<(), SettingsManagerError> {
        let global = Self::load_from_storage(&mut self.storage, SettingsScope::Global, true);
        let project = Self::load_from_storage(
            &mut self.storage,
            SettingsScope::Project,
            self.project_trusted,
        );
        let (global_settings, project_settings) = match (global, project) {
            (Ok(global), Ok(project)) => (global, project),
            (Err(error), Ok(_)) => {
                self.global_load_error = Some(error.to_string());
                self.record_error(SettingsScope::Global, &error);
                return Err(error);
            }
            (Ok(_), Err(error)) => {
                self.project_load_error = Some(error.to_string());
                self.record_error(SettingsScope::Project, &error);
                return Err(error);
            }
            (Err(global), Err(project)) => {
                self.global_load_error = Some(global.to_string());
                self.project_load_error = Some(project.to_string());
                self.record_error(SettingsScope::Global, &global);
                self.record_error(SettingsScope::Project, &project);
                return Err(SettingsManagerError::Message(format!(
                    "global config: {global}; project config: {project}"
                )));
            }
        };

        self.global_settings = global_settings;
        self.project_settings = project_settings;
        self.global_load_error = None;
        self.project_load_error = None;
        self.modified_fields.clear();
        self.modified_nested_fields.clear();
        self.modified_project_fields.clear();
        self.modified_project_nested_fields.clear();
        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);
        Ok(())
    }

    /// Compatibility wrapper for callers that consume diagnostics via `drain_errors`.
    pub fn reload(&mut self) {
        let _ = self.try_reload();
    }

    /// Spec: `applyOverrides(overrides)` — on top of merged settings.
    pub fn apply_overrides(&mut self, overrides: &Settings) {
        self.settings = deep_merge_settings(&self.settings, overrides);
    }

    /// Spec: `flush()` — a no-op here: the spec's write queue is
    /// synchronous in this port (module doc).
    pub fn flush(&self) {}

    /// Spec: `drainErrors()`.
    pub fn drain_errors(&mut self) -> Vec<SettingsError> {
        std::mem::take(&mut self.errors)
    }

    // -- internal mechanics -------------------------------------------------

    fn mark_modified(&mut self, field: &str, nested_key: Option<&str>) {
        self.modified_fields.insert(field.to_owned());
        if let Some(nested_key) = nested_key {
            self.modified_nested_fields
                .entry(field.to_owned())
                .or_default()
                .insert(nested_key.to_owned());
        }
    }

    fn mark_project_modified(&mut self, field: &str) {
        self.modified_project_fields.insert(field.to_owned());
    }

    /// Spec: `assertProjectTrustedForWrite`.
    fn assert_project_trusted_for_write(&self) -> Result<(), SettingsManagerError> {
        if !self.project_trusted {
            return Err(SettingsManagerError::Message(
                "Project is not trusted; refusing to write project settings".to_owned(),
            ));
        }
        Ok(())
    }

    fn record_error(&mut self, scope: SettingsScope, error: impl std::fmt::Display) {
        self.errors.push(SettingsError {
            scope,
            error: error.to_string(),
        });
    }

    fn clear_modified_scope(&mut self, scope: SettingsScope) {
        match scope {
            SettingsScope::Global => {
                self.modified_fields.clear();
                self.modified_nested_fields.clear();
            }
            SettingsScope::Project => {
                self.modified_project_fields.clear();
                self.modified_project_nested_fields.clear();
            }
        }
    }

    /// Spec: `persistScopedSettings` under `enqueueWrite` (synchronous
    /// here; errors recorded, not thrown).
    fn enqueue_write(
        &mut self,
        scope: SettingsScope,
        snapshot: Settings,
        modified_fields: BTreeSet<String>,
        modified_nested_fields: BTreeMap<String, BTreeSet<String>>,
    ) {
        if scope == SettingsScope::Project && self.assert_project_trusted_for_write().is_err() {
            self.record_error(
                scope,
                "Project is not trusted; refusing to write project settings",
            );
            return;
        }

        let result = self.storage.with_lock(scope, |current| {
            let source = current.unwrap_or("");
            let source_name = match scope {
                SettingsScope::Global => "global config.lua",
                SettingsScope::Project => "project .pi/config.lua",
            };
            if !source.is_empty() {
                // Refuse to mutate a concurrently broken user file.
                crate::config::evaluate(source, source_name)
                    .map_err(SettingsManagerError::Message)?;
            }
            // The generated block owns only fields changed interactively. Do not
            // copy evaluated user declarations into it and shadow future edits.
            let mut merged =
                crate::config::managed_settings(source).map_err(SettingsManagerError::Message)?;

            for field in &modified_fields {
                let value = snapshot.get(field);
                if let Some(nested_modified) = modified_nested_fields.get(field)
                    && let Some(Value::Object(in_memory_nested)) = value
                {
                    let mut merged_nested = merged
                        .get(field)
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    for nested_key in nested_modified {
                        match in_memory_nested.get(nested_key) {
                            Some(nested_value) => {
                                merged_nested.insert(nested_key.clone(), nested_value.clone());
                            }
                            None => {
                                merged_nested.remove(nested_key);
                            }
                        }
                    }
                    merged.insert(field.clone(), Value::Object(merged_nested));
                } else {
                    match value {
                        Some(value) => {
                            merged.insert(field.clone(), value.clone());
                        }
                        // Spec: `JSON.stringify` drops undefined values.
                        None => {
                            merged.remove(field);
                        }
                    }
                }
            }

            Ok(Some(crate::config::update_managed_settings(
                source, &merged,
            )))
        });

        match result {
            Ok(()) => self.clear_modified_scope(scope),
            Err(error) => self.record_error(scope, error),
        }
    }

    /// Spec: `save()` — global scope.
    fn save(&mut self) {
        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);

        if self.global_load_error.is_some() {
            return;
        }

        let snapshot = self.global_settings.clone();
        let modified_fields = self.modified_fields.clone();
        let modified_nested_fields = self.modified_nested_fields.clone();
        self.enqueue_write(
            SettingsScope::Global,
            snapshot,
            modified_fields,
            modified_nested_fields,
        );
    }

    /// Spec: `saveProjectSettings(settings)`.
    fn save_project_settings(&mut self, settings: Settings) -> Result<(), SettingsManagerError> {
        self.assert_project_trusted_for_write()?;
        self.project_settings = settings;
        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);

        if self.project_load_error.is_some() {
            return Ok(());
        }

        let snapshot = self.project_settings.clone();
        let modified_fields = self.modified_project_fields.clone();
        let modified_nested_fields = self.modified_project_nested_fields.clone();
        self.enqueue_write(
            SettingsScope::Project,
            snapshot,
            modified_fields,
            modified_nested_fields,
        );
        Ok(())
    }

    /// Spec: `updateProjectSettings(field, update)`.
    fn update_project_settings(
        &mut self,
        field: &str,
        value: Value,
    ) -> Result<(), SettingsManagerError> {
        self.assert_project_trusted_for_write()?;
        let mut project_settings = self.project_settings.clone();
        project_settings.insert(field.to_owned(), value);
        self.mark_project_modified(field);
        self.save_project_settings(project_settings)
    }

    // -- typed accessor helpers ---------------------------------------------

    fn merged_str(&self, key: &str) -> Option<&str> {
        self.settings.get(key).and_then(Value::as_str)
    }

    fn merged_bool(&self, key: &str, default: bool) -> bool {
        self.settings
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    fn nested_value(&self, key: &str, nested_key: &str) -> Option<&Value> {
        self.settings
            .get(key)
            .and_then(Value::as_object)
            .and_then(|obj| obj.get(nested_key))
    }

    fn nested_bool(&self, key: &str, nested_key: &str, default: bool) -> bool {
        self.nested_value(key, nested_key)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    fn nested_u64(&self, key: &str, nested_key: &str, default: u64) -> u64 {
        self.nested_value(key, nested_key)
            .and_then(Value::as_u64)
            .unwrap_or(default)
    }

    fn string_array(&self, key: &str) -> Vec<String> {
        self.settings
            .get(key)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn set_global(&mut self, key: &str, value: Value) {
        self.global_settings.insert(key.to_owned(), value);
        self.mark_modified(key, None);
        self.save();
    }

    /// `None` clears the key (spec: assigning `undefined`, dropped by
    /// `JSON.stringify`).
    fn set_global_opt(&mut self, key: &str, value: Option<Value>) {
        match value {
            Some(value) => {
                self.global_settings.insert(key.to_owned(), value);
            }
            None => {
                self.global_settings.remove(key);
            }
        }
        self.mark_modified(key, None);
        self.save();
    }

    fn set_global_nested(&mut self, key: &str, nested_key: &str, value: Value) {
        let entry = self
            .global_settings
            .entry(key.to_owned())
            .or_insert_with(|| Value::Object(Settings::new()));
        if !entry.is_object() {
            *entry = Value::Object(Settings::new());
        }
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(nested_key.to_owned(), value);
        }
        self.mark_modified(key, Some(nested_key));
        self.save();
    }

    // -- typed accessors (spec order) ---------------------------------------

    /// Spec: `getLastChangelogVersion()`.
    pub fn get_last_changelog_version(&self) -> Option<String> {
        self.merged_str("lastChangelogVersion").map(str::to_owned)
    }

    /// Spec: `setLastChangelogVersion(version)`.
    pub fn set_last_changelog_version(&mut self, version: &str) {
        self.set_global("lastChangelogVersion", Value::String(version.to_owned()));
    }

    /// Spec: `getSessionDir()` — normalized (`~` expanded).
    pub fn get_session_dir(&self) -> Option<PathBuf> {
        self.merged_str("sessionDir").map(expand_tilde_path)
    }

    /// Spec: `getDefaultProvider()`.
    pub fn get_default_provider(&self) -> Option<String> {
        self.merged_str("defaultProvider").map(str::to_owned)
    }

    /// Spec: `getDefaultModel()`.
    pub fn get_default_model(&self) -> Option<String> {
        self.merged_str("defaultModel").map(str::to_owned)
    }

    /// Spec: `setDefaultProvider(provider)`.
    pub fn set_default_provider(&mut self, provider: &str) {
        self.set_global("defaultProvider", Value::String(provider.to_owned()));
    }

    /// Spec: `setDefaultModel(modelId)`.
    pub fn set_default_model(&mut self, model_id: &str) {
        self.set_global("defaultModel", Value::String(model_id.to_owned()));
    }

    /// Spec: `setDefaultModelAndProvider(provider, modelId)`.
    pub fn set_default_model_and_provider(&mut self, provider: &str, model_id: &str) {
        self.global_settings.insert(
            "defaultProvider".to_owned(),
            Value::String(provider.to_owned()),
        );
        self.global_settings.insert(
            "defaultModel".to_owned(),
            Value::String(model_id.to_owned()),
        );
        self.mark_modified("defaultProvider", None);
        self.mark_modified("defaultModel", None);
        self.save();
    }

    /// Spec: `getSteeringMode()` — default `"one-at-a-time"`.
    pub fn get_steering_mode(&self) -> String {
        match self.merged_str("steeringMode") {
            Some(mode) if !mode.is_empty() => mode.to_owned(),
            _ => "one-at-a-time".to_owned(),
        }
    }

    /// Spec: `setSteeringMode(mode)`.
    pub fn set_steering_mode(&mut self, mode: &str) {
        self.set_global("steeringMode", Value::String(mode.to_owned()));
    }

    /// Spec: `getFollowUpMode()` — default `"one-at-a-time"`.
    pub fn get_follow_up_mode(&self) -> String {
        match self.merged_str("followUpMode") {
            Some(mode) if !mode.is_empty() => mode.to_owned(),
            _ => "one-at-a-time".to_owned(),
        }
    }

    /// Spec: `setFollowUpMode(mode)`.
    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.set_global("followUpMode", Value::String(mode.to_owned()));
    }

    /// Spec: `getTheme()`.
    pub fn get_theme(&self) -> Option<String> {
        self.merged_str("theme").map(str::to_owned)
    }

    /// Spec: `setTheme(theme)`.
    pub fn set_theme(&mut self, theme: &str) {
        self.set_global("theme", Value::String(theme.to_owned()));
    }

    /// Spec: `getDefaultThinkingLevel()`.
    pub fn get_default_thinking_level(&self) -> Option<ModelThinkingLevel> {
        self.merged_str("defaultThinkingLevel")
            .and_then(parse_thinking_level)
    }

    /// Spec: `setDefaultThinkingLevel(level)`.
    pub fn set_default_thinking_level(&mut self, level: ModelThinkingLevel) {
        if let Ok(value) = serde_json::to_value(level) {
            self.set_global("defaultThinkingLevel", value);
        }
    }

    /// Spec: `getTransport()` — default `"auto"`.
    pub fn get_transport(&self) -> String {
        self.merged_str("transport").unwrap_or("auto").to_owned()
    }

    /// Spec: `setTransport(transport)`.
    pub fn set_transport(&mut self, transport: &str) {
        self.set_global("transport", Value::String(transport.to_owned()));
    }

    /// Spec: `getCompactionEnabled()` — default `true`.
    pub fn get_compaction_enabled(&self) -> bool {
        self.nested_bool("compaction", "enabled", true)
    }

    /// Spec: `setCompactionEnabled(enabled)`.
    pub fn set_compaction_enabled(&mut self, enabled: bool) {
        self.set_global_nested("compaction", "enabled", Value::Bool(enabled));
    }

    /// Spec: `getCompactionReserveTokens()` — default 16384.
    pub fn get_compaction_reserve_tokens(&self) -> u64 {
        self.nested_u64("compaction", "reserveTokens", 16384)
    }

    /// Spec: `getCompactionKeepRecentTokens()` — default 20000.
    pub fn get_compaction_keep_recent_tokens(&self) -> u64 {
        self.nested_u64("compaction", "keepRecentTokens", 20000)
    }

    /// Spec: `getCompactionSettings()`.
    pub fn get_compaction_settings(&self) -> CompactionSettings {
        CompactionSettings {
            enabled: self.get_compaction_enabled(),
            reserve_tokens: self.get_compaction_reserve_tokens(),
            keep_recent_tokens: self.get_compaction_keep_recent_tokens(),
        }
    }

    /// Spec: `getBranchSummarySettings()`.
    pub fn get_branch_summary_settings(&self) -> BranchSummarySettings {
        BranchSummarySettings {
            reserve_tokens: self.nested_u64("branchSummary", "reserveTokens", 16384),
            skip_prompt: self.get_branch_summary_skip_prompt(),
        }
    }

    /// Spec: `getBranchSummarySkipPrompt()` — default `false`.
    pub fn get_branch_summary_skip_prompt(&self) -> bool {
        self.nested_bool("branchSummary", "skipPrompt", false)
    }

    /// Spec: `getRetryEnabled()` — default `true`.
    pub fn get_retry_enabled(&self) -> bool {
        self.nested_bool("retry", "enabled", true)
    }

    /// Spec: `setRetryEnabled(enabled)`.
    pub fn set_retry_enabled(&mut self, enabled: bool) {
        self.set_global_nested("retry", "enabled", Value::Bool(enabled));
    }

    /// Spec: `getRetrySettings()`.
    pub fn get_retry_settings(&self) -> RetrySettings {
        RetrySettings {
            enabled: self.get_retry_enabled(),
            max_retries: self.nested_u64("retry", "maxRetries", 3),
            base_delay_ms: self.nested_u64("retry", "baseDelayMs", 2000),
        }
    }

    /// Spec: `getHttpIdleTimeoutMs()` — throws on invalid values.
    pub fn get_http_idle_timeout_ms(&self) -> Result<u64, SettingsManagerError> {
        Ok(
            parse_timeout_setting(self.settings.get("httpIdleTimeoutMs"), "httpIdleTimeoutMs")?
                .unwrap_or(DEFAULT_HTTP_IDLE_TIMEOUT_MS),
        )
    }

    /// Spec: `setHttpIdleTimeoutMs(timeoutMs)` — the spec's
    /// finite/non-negative validation is unrepresentable for `u64`.
    pub fn set_http_idle_timeout_ms(&mut self, timeout_ms: u64) {
        self.set_global("httpIdleTimeoutMs", Value::from(timeout_ms));
    }

    /// Spec: `getProviderRetrySettings()`.
    pub fn get_provider_retry_settings(&self) -> ProviderRetrySettings {
        let provider = self
            .nested_value("retry", "provider")
            .and_then(Value::as_object);
        let field = |key: &str| provider.and_then(|p| p.get(key)).and_then(Value::as_u64);
        ProviderRetrySettings {
            timeout_ms: field("timeoutMs"),
            max_retries: field("maxRetries"),
            max_retry_delay_ms: field("maxRetryDelayMs").unwrap_or(60000),
        }
    }

    /// Spec: `getWebSocketConnectTimeoutMs()` — throws on invalid values.
    pub fn get_websocket_connect_timeout_ms(&self) -> Result<Option<u64>, SettingsManagerError> {
        parse_timeout_setting(
            self.settings.get("websocketConnectTimeoutMs"),
            "websocketConnectTimeoutMs",
        )
    }

    /// Spec: `getHideThinkingBlock()` — default `false`.
    pub fn get_hide_thinking_block(&self) -> bool {
        self.merged_bool("hideThinkingBlock", false)
    }

    /// Spec: `setHideThinkingBlock(hide)`.
    pub fn set_hide_thinking_block(&mut self, hide: bool) {
        self.set_global("hideThinkingBlock", Value::Bool(hide));
    }

    /// Spec: `getShellPath()`.
    pub fn get_shell_path(&self) -> Option<String> {
        self.merged_str("shellPath").map(str::to_owned)
    }

    /// Spec: `setShellPath(path)`.
    pub fn set_shell_path(&mut self, path: Option<&str>) {
        self.set_global_opt("shellPath", path.map(|p| Value::String(p.to_owned())));
    }

    /// Spec: `getQuietStartup()` — default `false`.
    pub fn get_quiet_startup(&self) -> bool {
        self.merged_bool("quietStartup", false)
    }

    /// Spec: `setQuietStartup(quiet)`.
    pub fn set_quiet_startup(&mut self, quiet: bool) {
        self.set_global("quietStartup", Value::Bool(quiet));
    }

    /// Spec: `getDefaultProjectTrust()` — global setting only;
    /// `"ask" | "always" | "never"`, invalid values read as `"ask"`.
    pub fn get_default_project_trust(&self) -> &'static str {
        match self
            .global_settings
            .get("defaultProjectTrust")
            .and_then(Value::as_str)
        {
            Some("always") => "always",
            Some("never") => "never",
            _ => "ask",
        }
    }

    /// Spec: `setDefaultProjectTrust(defaultProjectTrust)`.
    pub fn set_default_project_trust(&mut self, default_project_trust: &str) {
        self.set_global(
            "defaultProjectTrust",
            Value::String(default_project_trust.to_owned()),
        );
    }

    /// Spec: `getShellCommandPrefix()`.
    pub fn get_shell_command_prefix(&self) -> Option<String> {
        self.merged_str("shellCommandPrefix").map(str::to_owned)
    }

    /// Spec: `setShellCommandPrefix(prefix)`.
    pub fn set_shell_command_prefix(&mut self, prefix: Option<&str>) {
        self.set_global_opt(
            "shellCommandPrefix",
            prefix.map(|p| Value::String(p.to_owned())),
        );
    }

    /// Spec: `getNpmCommand()`.
    pub fn get_npm_command(&self) -> Option<Vec<String>> {
        self.settings.get("npmCommand").and_then(Value::as_array)?;
        Some(self.string_array("npmCommand"))
    }

    /// Spec: `setNpmCommand(command)`.
    pub fn set_npm_command(&mut self, command: Option<&[String]>) {
        self.set_global_opt(
            "npmCommand",
            command.map(|c| Value::Array(c.iter().map(|s| Value::String(s.clone())).collect())),
        );
    }

    /// Spec: `getCollapseChangelog()` — default `false`.
    pub fn get_collapse_changelog(&self) -> bool {
        self.merged_bool("collapseChangelog", false)
    }

    /// Spec: `setCollapseChangelog(collapse)`.
    pub fn set_collapse_changelog(&mut self, collapse: bool) {
        self.set_global("collapseChangelog", Value::Bool(collapse));
    }

    /// Spec: `getEnableInstallTelemetry()` — default `true`.
    pub fn get_enable_install_telemetry(&self) -> bool {
        self.merged_bool("enableInstallTelemetry", true)
    }

    /// Spec: `setEnableInstallTelemetry(enabled)`.
    pub fn set_enable_install_telemetry(&mut self, enabled: bool) {
        self.set_global("enableInstallTelemetry", Value::Bool(enabled));
    }

    /// Spec: `getPackages()` — `PackageSource` is string-or-object, kept
    /// as raw values.
    pub fn get_packages(&self) -> Vec<Value> {
        self.settings
            .get("packages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// Spec: `setPackages(packages)`.
    pub fn set_packages(&mut self, packages: Vec<Value>) {
        self.set_global("packages", Value::Array(packages));
    }

    /// Spec: `setProjectPackages(packages)`.
    pub fn set_project_packages(
        &mut self,
        packages: Vec<Value>,
    ) -> Result<(), SettingsManagerError> {
        self.update_project_settings("packages", Value::Array(packages))
    }

    /// Spec: `getExtensionPaths()`.
    pub fn get_extension_paths(&self) -> Vec<String> {
        self.string_array("extensions")
    }

    /// Spec: `setExtensionPaths(paths)`.
    pub fn set_extension_paths(&mut self, paths: &[String]) {
        self.set_global("extensions", string_vec_value(paths));
    }

    /// Spec: `setProjectExtensionPaths(paths)`.
    pub fn set_project_extension_paths(
        &mut self,
        paths: &[String],
    ) -> Result<(), SettingsManagerError> {
        self.update_project_settings("extensions", string_vec_value(paths))
    }

    /// Spec: `getSkillPaths()`.
    pub fn get_skill_paths(&self) -> Vec<String> {
        self.string_array("skills")
    }

    /// Spec: `setSkillPaths(paths)`.
    pub fn set_skill_paths(&mut self, paths: &[String]) {
        self.set_global("skills", string_vec_value(paths));
    }

    /// Spec: `setProjectSkillPaths(paths)`.
    pub fn set_project_skill_paths(
        &mut self,
        paths: &[String],
    ) -> Result<(), SettingsManagerError> {
        self.update_project_settings("skills", string_vec_value(paths))
    }

    /// Spec: `getPromptTemplatePaths()`.
    pub fn get_prompt_template_paths(&self) -> Vec<String> {
        self.string_array("prompts")
    }

    /// Spec: `setPromptTemplatePaths(paths)`.
    pub fn set_prompt_template_paths(&mut self, paths: &[String]) {
        self.set_global("prompts", string_vec_value(paths));
    }

    /// Spec: `setProjectPromptTemplatePaths(paths)`.
    pub fn set_project_prompt_template_paths(
        &mut self,
        paths: &[String],
    ) -> Result<(), SettingsManagerError> {
        self.update_project_settings("prompts", string_vec_value(paths))
    }

    /// Spec: `getThemePaths()`.
    pub fn get_theme_paths(&self) -> Vec<String> {
        self.string_array("themes")
    }

    /// Spec: `setThemePaths(paths)`.
    pub fn set_theme_paths(&mut self, paths: &[String]) {
        self.set_global("themes", string_vec_value(paths));
    }

    /// Spec: `setProjectThemePaths(paths)`.
    pub fn set_project_theme_paths(
        &mut self,
        paths: &[String],
    ) -> Result<(), SettingsManagerError> {
        self.update_project_settings("themes", string_vec_value(paths))
    }

    /// Spec: `getEnableSkillCommands()` — default `true`.
    pub fn get_enable_skill_commands(&self) -> bool {
        self.merged_bool("enableSkillCommands", true)
    }

    /// Spec: `setEnableSkillCommands(enabled)`.
    pub fn set_enable_skill_commands(&mut self, enabled: bool) {
        self.set_global("enableSkillCommands", Value::Bool(enabled));
    }

    /// Spec: `getThinkingBudgets()` — raw object, decoded by its
    /// consumer.
    pub fn get_thinking_budgets(&self) -> Option<Value> {
        self.settings.get("thinkingBudgets").cloned()
    }

    /// Spec: `getShowImages()` — default `true`.
    pub fn get_show_images(&self) -> bool {
        self.nested_bool("terminal", "showImages", true)
    }

    /// Spec: `setShowImages(show)`.
    pub fn set_show_images(&mut self, show: bool) {
        self.set_global_nested("terminal", "showImages", Value::Bool(show));
    }

    /// Spec: `getImageWidthCells()` — default 60, clamped ≥ 1.
    pub fn get_image_width_cells(&self) -> u64 {
        match self
            .nested_value("terminal", "imageWidthCells")
            .and_then(Value::as_f64)
        {
            Some(width) if width.is_finite() => (width.floor() as i64).max(1) as u64,
            _ => 60,
        }
    }

    /// Spec: `setImageWidthCells(width)` — clamped ≥ 1.
    pub fn set_image_width_cells(&mut self, width: u64) {
        self.set_global_nested("terminal", "imageWidthCells", Value::from(width.max(1)));
    }

    /// Spec: `getClearOnShrink()` — settings, then `PI_CLEAR_ON_SHRINK`,
    /// then `false`.
    pub fn get_clear_on_shrink(&self) -> bool {
        if let Some(value) = self
            .nested_value("terminal", "clearOnShrink")
            .and_then(Value::as_bool)
        {
            return value;
        }
        std::env::var("PI_CLEAR_ON_SHRINK").as_deref() == Ok("1")
    }

    /// Spec: `setClearOnShrink(enabled)`.
    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.set_global_nested("terminal", "clearOnShrink", Value::Bool(enabled));
    }

    /// Spec: `getShowTerminalProgress()` — default `false`.
    pub fn get_show_terminal_progress(&self) -> bool {
        self.nested_bool("terminal", "showTerminalProgress", false)
    }

    /// Spec: `setShowTerminalProgress(enabled)`.
    pub fn set_show_terminal_progress(&mut self, enabled: bool) {
        self.set_global_nested("terminal", "showTerminalProgress", Value::Bool(enabled));
    }

    /// Spec: `getImageAutoResize()` — default `true`.
    pub fn get_image_auto_resize(&self) -> bool {
        self.nested_bool("images", "autoResize", true)
    }

    /// Spec: `setImageAutoResize(enabled)`.
    pub fn set_image_auto_resize(&mut self, enabled: bool) {
        self.set_global_nested("images", "autoResize", Value::Bool(enabled));
    }

    /// Spec: `getBlockImages()` — default `false`.
    pub fn get_block_images(&self) -> bool {
        self.nested_bool("images", "blockImages", false)
    }

    /// Spec: `setBlockImages(blocked)`.
    pub fn set_block_images(&mut self, blocked: bool) {
        self.set_global_nested("images", "blockImages", Value::Bool(blocked));
    }

    /// Spec: `getEnabledModels()`.
    pub fn get_enabled_models(&self) -> Option<Vec<String>> {
        self.settings
            .get("enabledModels")
            .and_then(Value::as_array)?;
        Some(self.string_array("enabledModels"))
    }

    /// Spec: `setEnabledModels(patterns)`.
    pub fn set_enabled_models(&mut self, patterns: Option<&[String]>) {
        self.set_global_opt("enabledModels", patterns.map(string_vec_value));
    }

    /// Spec: `getDoubleEscapeAction()` — default `"tree"`.
    pub fn get_double_escape_action(&self) -> String {
        self.merged_str("doubleEscapeAction")
            .unwrap_or("tree")
            .to_owned()
    }

    /// Spec: `setDoubleEscapeAction(action)`.
    pub fn set_double_escape_action(&mut self, action: &str) {
        self.set_global("doubleEscapeAction", Value::String(action.to_owned()));
    }

    /// Spec: `getTreeFilterMode()` — validated; default `"default"`.
    pub fn get_tree_filter_mode(&self) -> String {
        const VALID: &[&str] = &["default", "no-tools", "user-only", "labeled-only", "all"];
        match self.merged_str("treeFilterMode") {
            Some(mode) if VALID.contains(&mode) => mode.to_owned(),
            _ => "default".to_owned(),
        }
    }

    /// Spec: `setTreeFilterMode(mode)`.
    pub fn set_tree_filter_mode(&mut self, mode: &str) {
        self.set_global("treeFilterMode", Value::String(mode.to_owned()));
    }

    /// Spec: `getShowHardwareCursor()` — settings, then
    /// `PI_HARDWARE_CURSOR`.
    pub fn get_show_hardware_cursor(&self) -> bool {
        if let Some(value) = self
            .settings
            .get("showHardwareCursor")
            .and_then(Value::as_bool)
        {
            return value;
        }
        std::env::var("PI_HARDWARE_CURSOR").as_deref() == Ok("1")
    }

    /// Spec: `setShowHardwareCursor(enabled)`.
    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.set_global("showHardwareCursor", Value::Bool(enabled));
    }

    /// Spec: `getEditorPaddingX()` — default 0.
    pub fn get_editor_padding_x(&self) -> u64 {
        self.settings
            .get("editorPaddingX")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }

    /// Spec: `setEditorPaddingX(padding)` — clamped to 0..=3.
    pub fn set_editor_padding_x(&mut self, padding: u64) {
        self.set_global("editorPaddingX", Value::from(padding.min(3)));
    }

    /// Spec: `getAutocompleteMaxVisible()` — default 5.
    pub fn get_autocomplete_max_visible(&self) -> u64 {
        self.settings
            .get("autocompleteMaxVisible")
            .and_then(Value::as_u64)
            .unwrap_or(5)
    }

    /// Spec: `setAutocompleteMaxVisible(maxVisible)` — clamped to 3..=20.
    pub fn set_autocomplete_max_visible(&mut self, max_visible: u64) {
        self.set_global(
            "autocompleteMaxVisible",
            Value::from(max_visible.clamp(3, 20)),
        );
    }

    /// Spec: `getCodeBlockIndent()` — default two spaces.
    pub fn get_code_block_indent(&self) -> String {
        self.nested_value("markdown", "codeBlockIndent")
            .and_then(Value::as_str)
            .unwrap_or("  ")
            .to_owned()
    }

    /// Spec: `getWarnings()` — raw object clone.
    pub fn get_warnings(&self) -> Settings {
        self.settings
            .get("warnings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    /// Spec: `setWarnings(warnings)`.
    pub fn set_warnings(&mut self, warnings: Settings) {
        self.set_global("warnings", Value::Object(warnings));
    }
}

fn string_vec_value(values: &[String]) -> Value {
    Value::Array(values.iter().map(|s| Value::String(s.clone())).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    fn obj(value: Value) -> Settings {
        match value {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[test]
    fn deep_merge_project_wins_nested_one_level() {
        let global = obj(json!({
            "theme": "dark",
            "compaction": { "enabled": true, "reserveTokens": 1 },
            "extensions": ["a"]
        }));
        let project = obj(json!({
            "compaction": { "enabled": false },
            "extensions": ["b"]
        }));
        let merged = deep_merge_settings(&global, &project);
        assert_eq!(merged.get("theme"), Some(&json!("dark")));
        assert_eq!(
            merged.get("compaction"),
            Some(&json!({ "enabled": false, "reserveTokens": 1 }))
        );
        // Arrays replace, never merge.
        assert_eq!(merged.get("extensions"), Some(&json!(["b"])));
    }

    #[test]
    fn migrations_match_spec() {
        let mut settings = obj(json!({
            "queueMode": "all",
            "websockets": true,
            "skills": { "enableSkillCommands": false, "customDirectories": ["x"] },
            "retry": { "maxDelayMs": 1234, "provider": { "timeoutMs": 5 } }
        }));
        migrate_settings(&mut settings);
        assert_eq!(settings.get("steeringMode"), Some(&json!("all")));
        assert!(!settings.contains_key("queueMode"));
        assert_eq!(settings.get("transport"), Some(&json!("websocket")));
        assert!(!settings.contains_key("websockets"));
        assert_eq!(settings.get("enableSkillCommands"), Some(&json!(false)));
        assert_eq!(settings.get("skills"), Some(&json!(["x"])));
        assert_eq!(
            settings.get("retry"),
            Some(&json!({ "provider": { "timeoutMs": 5, "maxRetryDelayMs": 1234 } }))
        );
    }

    #[test]
    fn migration_skills_object_without_dirs_is_dropped() {
        let mut settings = obj(json!({ "skills": { "enableSkillCommands": true } }));
        migrate_settings(&mut settings);
        assert!(!settings.contains_key("skills"));
    }

    #[test]
    fn in_memory_defaults_and_setters() {
        let mut manager = SettingsManager::in_memory(Settings::new());
        assert_eq!(manager.get_steering_mode(), "one-at-a-time");
        assert_eq!(manager.get_transport(), "auto");
        assert!(manager.get_compaction_enabled());
        assert_eq!(manager.get_compaction_reserve_tokens(), 16384);
        assert_eq!(manager.get_retry_settings().max_retries, 3);
        assert_eq!(
            manager.get_provider_retry_settings().max_retry_delay_ms,
            60000
        );
        assert_eq!(manager.get_http_idle_timeout_ms().unwrap(), 300_000);
        assert_eq!(manager.get_default_project_trust(), "ask");
        assert_eq!(manager.get_double_escape_action(), "tree");
        assert_eq!(manager.get_tree_filter_mode(), "default");
        assert_eq!(manager.get_autocomplete_max_visible(), 5);
        assert_eq!(manager.get_code_block_indent(), "  ");

        manager.set_default_model_and_provider("anthropic", "claude-opus-4-8");
        assert_eq!(manager.get_default_provider().as_deref(), Some("anthropic"));
        assert_eq!(
            manager.get_default_model().as_deref(),
            Some("claude-opus-4-8")
        );
        manager.set_default_thinking_level(ModelThinkingLevel::High);
        assert_eq!(
            manager.get_default_thinking_level(),
            Some(ModelThinkingLevel::High)
        );
        assert!(manager.drain_errors().is_empty());
    }

    #[test]
    fn in_memory_initial_settings_are_migrated() {
        let manager = SettingsManager::in_memory(obj(json!({ "queueMode": "all" })));
        assert_eq!(manager.get_steering_mode(), "all");
    }

    #[test]
    fn invalid_timeout_setting_errors() {
        let manager = SettingsManager::in_memory(obj(json!({ "httpIdleTimeoutMs": "nope" })));
        assert!(manager.get_http_idle_timeout_ms().is_err());
        assert_eq!(
            SettingsManager::in_memory(obj(json!({ "httpIdleTimeoutMs": "disabled" })))
                .get_http_idle_timeout_ms()
                .unwrap(),
            0
        );
        assert_eq!(
            SettingsManager::in_memory(Settings::new())
                .get_websocket_connect_timeout_ms()
                .unwrap(),
            None
        );
    }

    #[test]
    fn session_dir_expands_tilde() {
        let manager = SettingsManager::in_memory(obj(json!({ "sessionDir": "~/sessions" })));
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            manager.get_session_dir(),
            Some(PathBuf::from(home).join("sessions"))
        );
    }

    #[test]
    fn file_storage_persists_only_modified_fields() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let initial = obj(json!({
            "theme": "dark", "unknownKey": 42,
            "compaction": { "reserveTokens": 99 }
        }));
        let user_prefix = "local pi = ...\n-- user code remains\n";
        std::fs::write(
            agent_dir.join("config.lua"),
            crate::config::update_managed_settings(user_prefix, &initial),
        )
        .unwrap();

        let mut manager = SettingsManager::create(
            &cwd,
            Some(agent_dir.clone()),
            SettingsManagerCreateOptions::default(),
        );
        manager.set_default_model("m1");
        manager.set_compaction_enabled(false);
        assert!(manager.drain_errors().is_empty());

        let source = std::fs::read_to_string(agent_dir.join("config.lua")).unwrap();
        assert!(source.starts_with(user_prefix));
        let written = crate::config::evaluate(&source, "config.lua")
            .unwrap()
            .settings;
        assert_eq!(written["theme"], json!("dark"));
        assert_eq!(written["unknownKey"], json!(42));
        assert_eq!(written["defaultModel"], json!("m1"));
        assert_eq!(
            written["compaction"],
            json!({ "reserveTokens": 99, "enabled": false })
        );
        assert!(!agent_dir.join("config.lua.lock").exists());
    }

    #[test]
    fn file_storage_creates_file_only_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        let agent_dir = dir.path().join("agent");
        let mut manager = SettingsManager::create(
            &cwd,
            Some(agent_dir.clone()),
            SettingsManagerCreateOptions::default(),
        );
        assert!(!agent_dir.join("config.lua").exists());
        manager.set_theme("light");
        assert!(agent_dir.join("config.lua").exists());
        assert!(!agent_dir.join("settings.json").exists());
    }

    #[test]
    fn project_scope_reads_and_writes_dot_pi() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(cwd.join(".pi")).unwrap();
        let project = obj(json!({ "theme": "project-theme" }));
        std::fs::write(
            cwd.join(".pi/config.lua"),
            crate::config::update_managed_settings("", &project),
        )
        .unwrap();

        let mut manager = SettingsManager::create(
            &cwd,
            Some(agent_dir),
            SettingsManagerCreateOptions::default(),
        );
        assert_eq!(manager.get_theme().as_deref(), Some("project-theme"));
        manager
            .set_project_skill_paths(&["skills/".to_owned()])
            .unwrap();
        let source = std::fs::read_to_string(cwd.join(".pi/config.lua")).unwrap();
        let written = crate::config::evaluate(&source, ".pi/config.lua")
            .unwrap()
            .settings;
        assert_eq!(written["theme"], json!("project-theme"));
        assert_eq!(written["skills"], json!(["skills/"]));
    }

    #[test]
    fn untrusted_project_is_ignored_and_write_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(cwd.join(".pi")).unwrap();
        let project = obj(json!({ "theme": "evil" }));
        std::fs::write(
            cwd.join(".pi/config.lua"),
            crate::config::update_managed_settings("", &project),
        )
        .unwrap();

        let mut manager = SettingsManager::create(
            &cwd,
            Some(agent_dir),
            SettingsManagerCreateOptions {
                project_trusted: Some(false),
            },
        );
        assert_eq!(manager.get_theme(), None);
        assert!(manager.set_project_skill_paths(&[]).is_err());
        manager.set_project_trusted(true);
        assert_eq!(manager.get_theme().as_deref(), Some("evil"));
        manager.set_project_trusted(false);
        assert_eq!(manager.get_theme(), None);
    }

    #[test]
    fn parse_error_is_recorded_and_blocks_saves() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("config.lua"), "this is not lua").unwrap();

        let mut manager = SettingsManager::create(
            &cwd,
            Some(agent_dir.clone()),
            SettingsManagerCreateOptions::default(),
        );
        let errors = manager.drain_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].scope, SettingsScope::Global);
        manager.set_theme("light");
        assert_eq!(
            std::fs::read_to_string(agent_dir.join("config.lua")).unwrap(),
            "this is not lua"
        );
        assert_eq!(manager.get_theme().as_deref(), Some("light"));
    }

    #[test]
    fn reload_picks_up_external_changes_and_clears_marks() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        let agent_dir = dir.path().join("agent");
        let mut manager = SettingsManager::create(
            &cwd,
            Some(agent_dir.clone()),
            SettingsManagerCreateOptions::default(),
        );
        manager.set_theme("light");
        let external = obj(json!({ "theme": "external" }));
        std::fs::write(
            agent_dir.join("config.lua"),
            crate::config::update_managed_settings("", &external),
        )
        .unwrap();
        manager.try_reload().unwrap();
        assert_eq!(manager.get_theme().as_deref(), Some("external"));
    }

    #[test]
    fn apply_overrides_wins_over_merged() {
        let mut manager = SettingsManager::in_memory(obj(json!({ "theme": "dark" })));
        manager.apply_overrides(&obj(json!({ "theme": "cli-override" })));
        assert_eq!(manager.get_theme().as_deref(), Some("cli-override"));
    }

    #[test]
    fn clamped_setters() {
        let mut manager = SettingsManager::in_memory(Settings::new());
        manager.set_editor_padding_x(9);
        assert_eq!(manager.get_editor_padding_x(), 3);
        manager.set_autocomplete_max_visible(1);
        assert_eq!(manager.get_autocomplete_max_visible(), 3);
        manager.set_image_width_cells(0);
        assert_eq!(manager.get_image_width_cells(), 1);
    }
}
