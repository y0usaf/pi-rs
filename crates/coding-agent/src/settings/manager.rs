//! `core/settings-manager.ts`: settings read from `<agentDir>/settings.json` and, in a
//! trusted project, `<cwd>/.pi/settings.json`, merged with the project winning, and written
//! back to whichever layer a setter touches. A write re-reads the file and replaces only the
//! fields this session changed, so keys pi-rs does not know survive.

use std::collections::BTreeSet;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use pi_durable::ai::types::ModelThinkingLevel;
use serde_json::{Map, Value};

use crate::config::CONFIG_DIR_NAME;
use crate::json;
use crate::node_fs;
use crate::paths::{PathInput, dirname, join, normalize_path, resolve_path};

use super::schema::{
    self, BranchSummarySettings, CompactionSettings, ProviderRetrySettings, QuietStartup,
    RetrySettings, ThinkingBudgets, WarningSettings, WheelScrollLines, defaults, js_string,
    token_count,
};

/// Tools on when `defaultTools` does not change them.
pub const DEFAULT_TOOL_NAMES: [&str; 4] = ["read", "bash", "edit", "write"];

const LOCK_EX: i32 = 2;

fn is_tool_modifier(entry: &str) -> bool {
    entry.starts_with('+') || entry.starts_with('-')
}

/// `applyToolModifiers`: `+name` adds a tool, `-name` removes one, in list order.
pub fn apply_tool_modifiers(mut tools: Vec<String>, entries: &[String]) -> Vec<String> {
    for entry in entries.iter().filter(|entry| is_tool_modifier(entry)) {
        let name = &entry[1..];
        let index = tools.iter().position(|tool| tool == name);
        if entry.starts_with('+') && index.is_none() && !name.is_empty() {
            tools.push(name.to_owned());
        } else if let (true, Some(index)) = (entry.starts_with('-'), index) {
            tools.remove(index);
        }
    }
    tools
}

/// `resolveDefaultTools`: plain names replace the built-in selection, then modifiers apply.
fn resolve_default_tools(entries: &[String]) -> Vec<String> {
    let plain: Vec<String> = entries
        .iter()
        .filter(|entry| !is_tool_modifier(entry))
        .cloned()
        .collect();
    let base = if !plain.is_empty() || entries.is_empty() {
        plain
    } else {
        DEFAULT_TOOL_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    };
    apply_tool_modifiers(base, entries)
}

/// One settings layer: the JSON object a single file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    values: Map<String, Value>,
}

impl Settings {
    pub fn from_values(values: Map<String, Value>) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }

    /// A member read as upstream's `??` does: absent and `null` both mean "not set".
    fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key).filter(|value| !value.is_null())
    }

    fn raw(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    fn object(&self, key: &str) -> Option<&Map<String, Value>> {
        self.get(key)?.as_object()
    }

    fn object_mut(&mut self, key: &str) -> &mut Map<String, Value> {
        if !self.values.get(key).is_some_and(Value::is_object) {
            self.values
                .insert(key.to_owned(), Value::Object(Map::new()));
        }
        match self.values.get_mut(key).and_then(Value::as_object_mut) {
            Some(object) => object,
            None => unreachable!("the member was just made an object"),
        }
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    fn flag(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    fn number(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    fn texts(&self, key: &str) -> Option<Vec<String>> {
        Some(
            self.get(key)?
                .as_array()?
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
        )
    }

    fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_owned(), value);
    }

    fn set_text(&mut self, key: &str, value: &str) {
        self.set(key, Value::String(value.to_owned()));
    }

    fn set_flag(&mut self, key: &str, value: bool) {
        self.set(key, Value::Bool(value));
    }

    fn set_number(&mut self, key: &str, value: f64) {
        let number = serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number);
        self.set(key, number);
    }

    fn set_texts(&mut self, key: &str, values: &[String]) {
        self.set(
            key,
            Value::Array(values.iter().cloned().map(Value::String).collect()),
        );
    }

    fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }

    /// `JSON.parse(stripBom(content))`, migrated. A file that is not a JSON object reads as no
    /// settings, as before this port.
    fn parse(text: &str) -> Result<Settings, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        match json::parse(text)? {
            Value::Object(values) => Ok(Settings::from_values(migrate(values))),
            _ => Ok(Settings::default()),
        }
    }

    /// `JSON.stringify(settings, null, 2)`.
    fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Value::Object(self.values.clone())).unwrap_or_default()
    }
}

/// `migrateSettings`: the four old shapes upstream rewrites on read.
fn migrate(mut values: Map<String, Value>) -> Map<String, Value> {
    if values.contains_key("queueMode")
        && !values.contains_key("steeringMode")
        && let Some(mode) = values.remove("queueMode")
    {
        values.insert("steeringMode".to_owned(), mode);
    }
    if !values.contains_key("transport")
        && let Some(Value::Bool(sockets)) = values.get("websockets").cloned()
    {
        let transport = if sockets { "websocket" } else { "sse" };
        values.insert("transport".to_owned(), Value::String(transport.to_owned()));
        values.remove("websockets");
    }
    if let Some(Value::Object(skills)) = values.get("skills").cloned() {
        if let Some(enabled) = skills.get("enableSkillCommands")
            && !values.contains_key("enableSkillCommands")
        {
            values.insert("enableSkillCommands".to_owned(), enabled.clone());
        }
        match skills.get("customDirectories") {
            Some(Value::Array(directories)) if !directories.is_empty() => {
                values.insert("skills".to_owned(), Value::Array(directories.clone()));
            }
            _ => {
                values.remove("skills");
            }
        }
    }
    if let Some(Value::Object(mut retry)) = values.get("retry").cloned() {
        let provider = retry.get("provider").and_then(Value::as_object).cloned();
        let inherited = provider
            .as_ref()
            .and_then(|provider| provider.get("maxRetryDelayMs"));
        if let Some(delay) = retry.get("maxDelayMs").filter(|delay| delay.is_number())
            && matches!(inherited, None | Some(Value::Null))
        {
            let mut provider = provider.unwrap_or_default();
            provider.insert("maxRetryDelayMs".to_owned(), delay.clone());
            retry.insert("provider".to_owned(), Value::Object(provider));
        }
        retry.remove("maxDelayMs");
        values.insert("retry".to_owned(), Value::Object(retry));
    }
    values
}

/// `deepMergeObjects`: nested objects merge, anything else is replaced.
fn deep_merge_objects(
    base: &Map<String, Value>,
    overrides: &Map<String, Value>,
) -> Map<String, Value> {
    let mut merged = base.clone();
    for (key, value) in overrides {
        let next = match (base.get(key), value) {
            (Some(Value::Object(base)), Value::Object(overrides)) => {
                Value::Object(deep_merge_objects(base, overrides))
            }
            _ => value.clone(),
        };
        merged.insert(key.clone(), next);
    }
    merged
}

/// `mergeDefaultTools`: a list of only modifiers extends the inherited selection.
fn merge_default_tools(base: Option<&Value>, overrides: Option<&Value>) -> Option<Value> {
    let Some(overrides) = overrides else {
        return base.cloned();
    };
    if let (Some(base), Some(overrides)) = (base.and_then(Value::as_array), overrides.as_array())
        && overrides
            .iter()
            .all(|entry| entry.as_str().is_some_and(is_tool_modifier))
    {
        let mut both = base.clone();
        both.extend(overrides.iter().cloned());
        return Some(Value::Array(both));
    }
    Some(overrides.clone())
}

/// `deepMergeSettings`: base first, overrides winning, `defaultTools` merged separately.
fn deep_merge_settings(base: &Settings, overrides: &Settings) -> Settings {
    let mut values = deep_merge_objects(&base.values, &overrides.values);
    match merge_default_tools(
        base.values.get("defaultTools"),
        overrides.values.get("defaultTools"),
    ) {
        Some(default_tools) => {
            values.insert("defaultTools".to_owned(), default_tools);
        }
        None => {
            values.remove("defaultTools");
        }
    }
    Settings::from_values(values)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsScope {
    Global,
    Project,
}

impl SettingsScope {
    pub fn name(self) -> &'static str {
        match self {
            SettingsScope::Global => "global",
            SettingsScope::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    pub scope: SettingsScope,
    pub path: Option<String>,
    pub error: String,
}

/// The write half of a settings layer: it gets the file's current content and returns the new
/// content, or None to leave the file alone.
pub type Update<'a> = dyn FnMut(Option<&str>) -> Result<Option<String>, String> + 'a;

/// Where a settings layer is read and written. `write` hands the current file content to
/// `update` under an exclusive lock and writes what comes back.
pub trait SettingsStorage {
    fn read(&self, scope: SettingsScope) -> Result<Option<String>, String>;
    fn write(&self, scope: SettingsScope, update: &mut Update<'_>) -> Result<(), String>;
}

/// The two settings files of a cwd and an agent dir.
pub struct FileSettingsStorage {
    global: String,
    project: String,
}

impl FileSettingsStorage {
    pub fn new(cwd: &str, agent_dir: &str) -> Self {
        let cwd = resolve_path(cwd, &process_cwd(), PathInput::default());
        let agent_dir = resolve_path(agent_dir, &process_cwd(), PathInput::default());
        Self {
            global: join(&agent_dir, "settings.json"),
            project: join(&cwd, &format!("{CONFIG_DIR_NAME}/settings.json")),
        }
    }

    pub fn global_path(&self) -> &str {
        &self.global
    }

    pub fn project_path(&self) -> &str {
        &self.project
    }

    fn path(&self, scope: SettingsScope) -> &str {
        match scope {
            SettingsScope::Global => &self.global,
            SettingsScope::Project => &self.project,
        }
    }
}

impl SettingsStorage for FileSettingsStorage {
    fn read(&self, scope: SettingsScope) -> Result<Option<String>, String> {
        let path = self.path(scope);
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(node_fs::message(&error, "open", path)),
        }
    }

    fn write(&self, scope: SettingsScope, update: &mut Update<'_>) -> Result<(), String> {
        let path = self.path(scope);
        if Path::new(path).exists() {
            let mut file = lock(path)?;
            let bytes =
                std::fs::read(path).map_err(|error| node_fs::message(&error, "open", path))?;
            let current = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(next) = update(Some(&current))? {
                replace(&mut file, path, &next)?;
            }
            return Ok(());
        }
        let Some(next) = update(None)? else {
            return Ok(());
        };
        std::fs::create_dir_all(dirname(path))
            .map_err(|error| node_fs::message(&error, "mkdir", path))?;
        let mut file = lock(path)?;
        replace(&mut file, path, &next)
    }
}

/// A settings file, opened read-write and held under `flock` until it is dropped.
fn lock(path: &str) -> Result<std::fs::File, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| node_fs::message(&error, "open", path))?;
    unsafe extern "C" {
        fn flock(descriptor: i32, operation: i32) -> i32;
    }
    // SAFETY: the descriptor belongs to `file`, which outlives the call.
    let locked = unsafe { flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), LOCK_EX) };
    if locked != 0 {
        return Err(node_fs::message(
            &std::io::Error::last_os_error(),
            "flock",
            path,
        ));
    }
    Ok(file)
}

fn replace(file: &mut std::fs::File, path: &str, text: &str) -> Result<(), String> {
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)))
        .and_then(|_| file.write_all(text.as_bytes()))
        .map_err(|error| node_fs::message(&error, "write", path))
}

/// A settings backend that keeps both layers in memory, for tests and in-memory sessions.
#[derive(Default)]
pub struct InMemorySettingsStorage {
    global: Mutex<Option<String>>,
    project: Mutex<Option<String>>,
}

impl InMemorySettingsStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a layer, as a file would hold it.
    pub fn set(&self, scope: SettingsScope, text: &str) {
        *content(self.slot(scope)) = Some(text.to_owned());
    }

    fn slot(&self, scope: SettingsScope) -> &Mutex<Option<String>> {
        match scope {
            SettingsScope::Global => &self.global,
            SettingsScope::Project => &self.project,
        }
    }
}

/// A layer's content; a poisoned lock still holds the last written settings.
fn content(slot: &Mutex<Option<String>>) -> std::sync::MutexGuard<'_, Option<String>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SettingsStorage for InMemorySettingsStorage {
    fn read(&self, scope: SettingsScope) -> Result<Option<String>, String> {
        Ok(content(self.slot(scope)).clone())
    }

    fn write(&self, scope: SettingsScope, update: &mut Update<'_>) -> Result<(), String> {
        let next = {
            let current = content(self.slot(scope));
            update(current.as_deref())?
        };
        if let Some(text) = next {
            *content(self.slot(scope)) = Some(text);
        }
        Ok(())
    }
}

fn process_cwd() -> String {
    std::env::current_dir()
        .map(|cwd| cwd.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".to_owned())
}

/// The fields a session changed in one layer, in the order they were first set.
#[derive(Debug, Clone, Default)]
struct ModifiedFields {
    fields: Vec<String>,
    nested: Vec<(String, BTreeSet<String>)>,
}

impl ModifiedFields {
    fn mark(&mut self, field: &str, nested_key: Option<&str>) {
        if !self.fields.iter().any(|known| known == field) {
            self.fields.push(field.to_owned());
        }
        if let Some(key) = nested_key {
            match self.nested.iter_mut().find(|(known, _)| known == field) {
                Some((_, keys)) => {
                    keys.insert(key.to_owned());
                }
                None => self
                    .nested
                    .push((field.to_owned(), BTreeSet::from([key.to_owned()]))),
            }
        }
    }

    fn nested_keys(&self, field: &str) -> Option<&BTreeSet<String>> {
        self.nested
            .iter()
            .find(|(known, _)| known == field)
            .map(|(_, keys)| keys)
    }

    fn clear(&mut self) {
        self.fields.clear();
        self.nested.clear();
    }
}

fn load_scope(
    storage: &dyn SettingsStorage,
    scope: SettingsScope,
    project_trusted: bool,
) -> (Settings, Option<String>) {
    if scope == SettingsScope::Project && !project_trusted {
        return (Settings::default(), None);
    }
    match storage.read(scope) {
        Err(error) => (Settings::default(), Some(error)),
        Ok(None) => (Settings::default(), None),
        Ok(Some(text)) => {
            if text.is_empty() {
                return (Settings::default(), None);
            }
            match Settings::parse(&text) {
                Ok(settings) => (settings, None),
                Err(error) => (Settings::default(), Some(error)),
            }
        }
    }
}

/// `persistScopedSettings`: the file's own content, with this session's fields replaced.
fn persist(
    storage: &dyn SettingsStorage,
    scope: SettingsScope,
    snapshot: &Settings,
    modified: &ModifiedFields,
) -> Result<(), String> {
    storage.write(scope, &mut |current| {
        let mut merged = match current {
            None => Map::new(),
            Some("") => Map::new(),
            Some(text) => Settings::parse(text)?.values,
        };
        for field in &modified.fields {
            let value = snapshot.values.get(field);
            match (modified.nested_keys(field), value) {
                (Some(keys), Some(Value::Object(in_memory))) => {
                    let mut nested = merged
                        .get(field)
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    for key in keys {
                        match in_memory.get(key) {
                            Some(in_memory) => {
                                nested.insert(key.clone(), in_memory.clone());
                            }
                            None => {
                                nested.remove(key);
                            }
                        }
                    }
                    merged.insert(field.clone(), Value::Object(nested));
                }
                (_, Some(value)) => {
                    merged.insert(field.clone(), value.clone());
                }
                (_, None) => {
                    merged.remove(field);
                }
            }
        }
        Ok(Some(Settings::from_values(merged).to_json()))
    })
}

fn choice<'a>(value: Option<&Value>, allowed: &[&'a str], fallback: &'a str) -> &'a str {
    match value.and_then(Value::as_str) {
        Some(text) => allowed
            .iter()
            .copied()
            .find(|allowed| *allowed == text)
            .unwrap_or(fallback),
        None => fallback,
    }
}

fn text_or<'a>(value: Option<&'a Value>, fallback: &'a str) -> &'a str {
    value.and_then(Value::as_str).unwrap_or(fallback)
}

/// `parseHttpIdleTimeoutMs`: a non-negative number of milliseconds, `disabled` as zero.
fn parse_http_idle_timeout(value: &Value) -> Option<f64> {
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.to_lowercase() == "disabled" {
            return Some(0.0);
        }
        if trimmed.is_empty() {
            return None;
        }
        return trimmed.parse::<f64>().ok().and_then(finite_timeout);
    }
    value.as_f64().and_then(finite_timeout)
}

fn finite_timeout(millis: f64) -> Option<f64> {
    (millis.is_finite() && millis >= 0.0).then(|| millis.floor())
}

fn level_value(level: ModelThinkingLevel) -> Value {
    serde_json::to_value(level).unwrap_or(Value::Null)
}

/// `SettingsManager`: both layers, their merge, and the writes back to each.
pub struct SettingsManager {
    storage: Arc<dyn SettingsStorage>,
    global: Settings,
    project: Settings,
    merged: Settings,
    project_trusted: bool,
    modified: ModifiedFields,
    modified_project: ModifiedFields,
    global_load_error: Option<String>,
    project_load_error: Option<String>,
    errors: Vec<SettingsError>,
    global_path: Option<String>,
    project_path: Option<String>,
}

impl SettingsManager {
    /// `SettingsManager.create(cwd, agentDir, { projectTrusted })`.
    pub fn create(cwd: &str, agent_dir: &str, project_trusted: bool) -> SettingsManager {
        let storage = FileSettingsStorage::new(cwd, agent_dir);
        let global_path = Some(storage.global_path().to_owned());
        let project_path = Some(storage.project_path().to_owned());
        Self::from_storage_with_paths(
            Arc::new(storage),
            project_trusted,
            global_path,
            project_path,
        )
    }

    /// `SettingsManager.fromStorage`.
    pub fn from_storage(
        storage: Arc<dyn SettingsStorage>,
        project_trusted: bool,
    ) -> SettingsManager {
        Self::from_storage_with_paths(storage, project_trusted, None, None)
    }

    /// `SettingsManager.inMemory`: one global layer, no files.
    pub fn in_memory(values: Map<String, Value>, project_trusted: bool) -> SettingsManager {
        let storage = InMemorySettingsStorage::new();
        let migrated = Settings::from_values(migrate(values));
        storage.set(SettingsScope::Global, &migrated.to_json());
        Self::from_storage(Arc::new(storage), project_trusted)
    }

    fn from_storage_with_paths(
        storage: Arc<dyn SettingsStorage>,
        project_trusted: bool,
        global_path: Option<String>,
        project_path: Option<String>,
    ) -> SettingsManager {
        let mut errors = Vec::new();
        let (global, global_error) = load_scope(&*storage, SettingsScope::Global, project_trusted);
        if let Some(error) = &global_error {
            errors.push(SettingsError {
                scope: SettingsScope::Global,
                path: global_path.clone(),
                error: error.clone(),
            });
        }
        let (project, project_error) =
            load_scope(&*storage, SettingsScope::Project, project_trusted);
        if let Some(error) = &project_error {
            errors.push(SettingsError {
                scope: SettingsScope::Project,
                path: project_path.clone(),
                error: error.clone(),
            });
        }
        let merged = deep_merge_settings(&global, &project);
        SettingsManager {
            storage,
            global,
            project,
            merged,
            project_trusted,
            modified: ModifiedFields::default(),
            modified_project: ModifiedFields::default(),
            global_load_error: global_error,
            project_load_error: project_error,
            errors,
            global_path,
            project_path,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.merged
    }

    pub fn global_settings(&self) -> &Settings {
        &self.global
    }

    pub fn project_settings(&self) -> &Settings {
        &self.project
    }

    pub fn is_project_trusted(&self) -> bool {
        self.project_trusted
    }

    /// `drainErrors`: the recorded failures, emptied.
    pub fn drain_errors(&mut self) -> Vec<SettingsError> {
        std::mem::take(&mut self.errors)
    }

    fn record_error(&mut self, scope: SettingsScope, error: impl Into<String>) {
        let path = match scope {
            SettingsScope::Global => self.global_path.clone(),
            SettingsScope::Project => self.project_path.clone(),
        };
        self.errors.push(SettingsError {
            scope,
            path,
            error: error.into(),
        });
    }

    pub fn set_project_trusted(&mut self, trusted: bool) {
        if self.project_trusted == trusted {
            return;
        }
        self.project_trusted = trusted;
        self.modified_project.clear();
        if !trusted {
            self.project = Settings::default();
            self.project_load_error = None;
            self.merged = deep_merge_settings(&self.global, &self.project);
            return;
        }
        let (project, error) = load_scope(&*self.storage, SettingsScope::Project, true);
        self.project = project;
        self.project_load_error = error.clone();
        if let Some(error) = error {
            self.record_error(SettingsScope::Project, error);
        }
        self.merged = deep_merge_settings(&self.global, &self.project);
    }

    pub fn reload(&mut self) {
        let (global, global_error) = load_scope(&*self.storage, SettingsScope::Global, true);
        match global_error {
            None => {
                self.global = global;
                self.global_load_error = None;
            }
            Some(error) => {
                self.global_load_error = Some(error.clone());
                self.record_error(SettingsScope::Global, error);
            }
        }
        self.modified.clear();
        self.modified_project.clear();
        let (project, project_error) =
            load_scope(&*self.storage, SettingsScope::Project, self.project_trusted);
        match project_error {
            None => {
                self.project = project;
                self.project_load_error = None;
            }
            Some(error) => {
                self.project_load_error = Some(error.clone());
                self.record_error(SettingsScope::Project, error);
            }
        }
        self.merged = deep_merge_settings(&self.global, &self.project);
    }

    pub fn apply_overrides(&mut self, overrides: Map<String, Value>) {
        self.merged = deep_merge_settings(&self.merged, &Settings::from_values(overrides));
    }

    fn mark_modified(&mut self, field: &str, nested_key: Option<&str>) {
        self.modified.mark(field, nested_key);
    }

    fn assert_project_trusted(&self) -> Result<(), String> {
        if self.project_trusted {
            Ok(())
        } else {
            Err("Project is not trusted; refusing to write project settings".to_owned())
        }
    }

    fn save(&mut self) {
        self.merged = deep_merge_settings(&self.global, &self.project);
        if self.global_load_error.is_some() {
            return;
        }
        let storage = Arc::clone(&self.storage);
        let snapshot = self.global.clone();
        let modified = self.modified.clone();
        match persist(&*storage, SettingsScope::Global, &snapshot, &modified) {
            Ok(()) => self.modified.clear(),
            Err(error) => self.record_error(SettingsScope::Global, error),
        }
    }

    fn save_project(&mut self, settings: Settings) {
        self.project = settings;
        self.merged = deep_merge_settings(&self.global, &self.project);
        if self.project_load_error.is_some() {
            return;
        }
        let storage = Arc::clone(&self.storage);
        let snapshot = self.project.clone();
        let modified = self.modified_project.clone();
        match persist(&*storage, SettingsScope::Project, &snapshot, &modified) {
            Ok(()) => self.modified_project.clear(),
            Err(error) => self.record_error(SettingsScope::Project, error),
        }
    }

    fn update_project(
        &mut self,
        field: &str,
        update: impl FnOnce(&mut Settings),
    ) -> Result<(), String> {
        self.assert_project_trusted()?;
        let mut project = self.project.clone();
        update(&mut project);
        self.modified_project.mark(field, None);
        self.save_project(project);
        Ok(())
    }

    pub fn last_changelog_version(&self) -> Option<&str> {
        self.merged.text("lastChangelogVersion")
    }

    pub fn set_last_changelog_version(&mut self, version: &str) {
        self.global.set_text("lastChangelogVersion", version);
        self.mark_modified("lastChangelogVersion", None);
        self.save();
    }

    /// `getSessionDir`: `~` expanded, otherwise as written.
    pub fn session_dir(&self) -> Option<String> {
        self.merged
            .text("sessionDir")
            .map(|dir| normalize_path(dir, PathInput::default()))
    }

    pub fn default_provider(&self) -> Option<&str> {
        self.merged.text("defaultProvider")
    }

    pub fn default_model(&self) -> Option<&str> {
        self.merged.text("defaultModel")
    }

    pub fn set_default_provider(&mut self, provider: &str) {
        self.global.set_text("defaultProvider", provider);
        self.mark_modified("defaultProvider", None);
        self.save();
    }

    pub fn set_default_model(&mut self, model: &str) {
        self.global.set_text("defaultModel", model);
        self.mark_modified("defaultModel", None);
        self.save();
    }

    pub fn set_default_model_and_provider(&mut self, provider: &str, model: &str) {
        self.global.set_text("defaultProvider", provider);
        self.global.set_text("defaultModel", model);
        self.mark_modified("defaultProvider", None);
        self.mark_modified("defaultModel", None);
        self.save();
    }

    pub fn steering_mode(&self) -> &str {
        text_or(self.merged.get("steeringMode"), defaults::STEERING_MODE)
    }

    pub fn set_steering_mode(&mut self, mode: &str) {
        self.global.set_text("steeringMode", mode);
        self.mark_modified("steeringMode", None);
        self.save();
    }

    pub fn follow_up_mode(&self) -> &str {
        text_or(self.merged.get("followUpMode"), defaults::FOLLOW_UP_MODE)
    }

    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.global.set_text("followUpMode", mode);
        self.mark_modified("followUpMode", None);
        self.save();
    }

    pub fn theme_setting(&self) -> Option<&str> {
        self.merged.text("theme")
    }

    /// `getTheme`: a theme name, not the `light/dark` automatic pair.
    pub fn theme(&self) -> Option<&str> {
        self.theme_setting().filter(|theme| !theme.contains('/'))
    }

    pub fn set_theme(&mut self, theme: &str) {
        self.global.set_text("theme", theme);
        self.mark_modified("theme", None);
        self.save();
    }

    pub fn default_thinking_level(&self) -> Option<&str> {
        self.merged.text("defaultThinkingLevel")
    }

    pub fn set_default_thinking_level(&mut self, level: ModelThinkingLevel) {
        self.global.set("defaultThinkingLevel", level_value(level));
        self.mark_modified("defaultThinkingLevel", None);
        self.save();
    }

    /// `modelThinkingLevels["provider/modelId"]`.
    pub fn model_thinking_level(&self, provider: &str, id: &str) -> Option<&str> {
        self.merged
            .object("modelThinkingLevels")?
            .get(&format!("{provider}/{id}"))?
            .as_str()
    }

    pub fn all_model_thinking_levels(&self) -> Map<String, Value> {
        self.merged
            .object("modelThinkingLevels")
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_model_thinking_level(
        &mut self,
        provider: &str,
        id: &str,
        level: ModelThinkingLevel,
    ) {
        self.global
            .object_mut("modelThinkingLevels")
            .insert(format!("{provider}/{id}"), level_value(level));
        self.mark_modified("modelThinkingLevels", None);
        self.save();
    }

    pub fn remove_model_thinking_level(&mut self, provider: &str, id: &str) {
        if self.global.object("modelThinkingLevels").is_none() {
            return;
        }
        let key = format!("{provider}/{id}");
        let empty = {
            let levels = self.global.object_mut("modelThinkingLevels");
            levels.remove(&key);
            levels.is_empty()
        };
        if empty {
            self.global.remove("modelThinkingLevels");
        }
        self.mark_modified("modelThinkingLevels", None);
        self.save();
    }

    pub fn transport(&self) -> &str {
        text_or(self.merged.get("transport"), defaults::TRANSPORT)
    }

    pub fn set_transport(&mut self, transport: &str) {
        self.global.set_text("transport", transport);
        self.mark_modified("transport", None);
        self.save();
    }

    pub fn session_share_destination(&self) -> &str {
        choice(
            self.merged.get("sessionShareDestination"),
            &schema::SESSION_SHARE_DESTINATIONS,
            defaults::SESSION_SHARE_DESTINATION,
        )
    }

    pub fn set_session_share_destination(&mut self, destination: &str) {
        self.global.set_text("sessionShareDestination", destination);
        self.mark_modified("sessionShareDestination", None);
        self.save();
    }

    pub fn compaction_enabled(&self) -> bool {
        self.merged
            .object("compaction")
            .and_then(|compaction| compaction.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(defaults::COMPACTION_ENABLED)
    }

    pub fn set_compaction_enabled(&mut self, enabled: bool) {
        self.global
            .object_mut("compaction")
            .insert("enabled".to_owned(), Value::Bool(enabled));
        self.mark_modified("compaction", Some("enabled"));
        self.save();
    }

    /// `getCompactionTokenSetting`: the model's override, else the ordinary setting, else the
    /// built-in default. A malformed value is an error, as upstream throws.
    fn compaction_token(
        &self,
        field: &str,
        model: Option<(&str, &str)>,
        default: u64,
    ) -> Result<u64, String> {
        let compaction = self.merged.object("compaction");
        let ordinary = compaction.and_then(|compaction| compaction.get(field));
        let ordinary = match ordinary {
            None => None,
            Some(value) => Some(token_count(value, &format!("compaction.{field}"))?),
        };

        let key = model.map(|(provider, id)| format!("{provider}/{id}"));
        let entry = key
            .as_ref()
            .and_then(|key| compaction?.get("modelOverrides")?.as_object()?.get(key));
        let Some(entry) = entry else {
            return Ok(ordinary.unwrap_or(default));
        };
        let key = key.unwrap_or_default();
        let Some(entry) = entry.as_object() else {
            return Err(format!(
                "Invalid compaction.modelOverrides[\"{key}\"] setting: {}. Expected an object.",
                js_string(entry)
            ));
        };
        match entry.get(field) {
            Some(value) => token_count(
                value,
                &format!("compaction.modelOverrides[\"{key}\"].{field}"),
            ),
            None => Ok(ordinary.unwrap_or(default)),
        }
    }

    pub fn compaction_reserve_tokens(&self, model: Option<(&str, &str)>) -> Result<u64, String> {
        self.compaction_token("reserveTokens", model, defaults::COMPACTION_RESERVE_TOKENS)
    }

    pub fn compaction_keep_recent_tokens(
        &self,
        model: Option<(&str, &str)>,
    ) -> Result<u64, String> {
        self.compaction_token(
            "keepRecentTokens",
            model,
            defaults::COMPACTION_KEEP_RECENT_TOKENS,
        )
    }

    pub fn compaction_settings(
        &self,
        model: Option<(&str, &str)>,
    ) -> Result<CompactionSettings, String> {
        Ok(CompactionSettings {
            enabled: self.compaction_enabled(),
            reserve_tokens: self.compaction_reserve_tokens(model)?,
            keep_recent_tokens: self.compaction_keep_recent_tokens(model)?,
        })
    }

    pub fn branch_summary_settings(&self) -> BranchSummarySettings {
        let branch_summary = self.merged.object("branchSummary");
        BranchSummarySettings {
            reserve_tokens: branch_summary
                .and_then(|settings| settings.get("reserveTokens"))
                .filter(|value| !value.is_null())
                .and_then(Value::as_f64)
                .unwrap_or(defaults::BRANCH_SUMMARY_RESERVE_TOKENS),
            skip_prompt: branch_summary
                .and_then(|settings| settings.get("skipPrompt"))
                .filter(|value| !value.is_null())
                .and_then(Value::as_bool)
                .unwrap_or(defaults::BRANCH_SUMMARY_SKIP_PROMPT),
        }
    }

    pub fn branch_summary_skip_prompt(&self) -> bool {
        self.branch_summary_settings().skip_prompt
    }

    pub fn retry_enabled(&self) -> bool {
        self.merged
            .object("retry")
            .and_then(|retry| retry.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(defaults::RETRY_ENABLED)
    }

    pub fn set_retry_enabled(&mut self, enabled: bool) {
        self.global
            .object_mut("retry")
            .insert("enabled".to_owned(), Value::Bool(enabled));
        self.mark_modified("retry", Some("enabled"));
        self.save();
    }

    pub fn retry_settings(&self) -> RetrySettings {
        let retry = self.merged.object("retry");
        let number = |key: &str, fallback: f64| {
            retry
                .and_then(|retry| retry.get(key))
                .filter(|value| !value.is_null())
                .and_then(Value::as_f64)
                .unwrap_or(fallback)
        };
        RetrySettings {
            enabled: self.retry_enabled(),
            max_retries: number("maxRetries", defaults::RETRY_MAX_RETRIES),
            base_delay_ms: number("baseDelayMs", defaults::RETRY_BASE_DELAY_MS),
            max_agent_delay_ms: number("maxAgentDelayMs", defaults::RETRY_MAX_AGENT_DELAY_MS),
        }
    }

    pub fn provider_retry_settings(&self) -> ProviderRetrySettings {
        let provider = self
            .merged
            .object("retry")
            .and_then(|retry| retry.get("provider"))
            .and_then(Value::as_object);
        let number = |key: &str| {
            provider
                .and_then(|provider| provider.get(key))
                .filter(|value| !value.is_null())
                .and_then(Value::as_f64)
        };
        ProviderRetrySettings {
            timeout_ms: number("timeoutMs"),
            max_retries: number("maxRetries"),
            max_retry_delay_ms: number("maxRetryDelayMs")
                .unwrap_or(defaults::RETRY_PROVIDER_MAX_RETRY_DELAY_MS),
        }
    }

    fn parse_timeout(&self, key: &str, setting: &str) -> Result<Option<f64>, String> {
        let Some(value) = self.merged.raw(key) else {
            return Ok(None);
        };
        match parse_http_idle_timeout(value) {
            Some(millis) => Ok(Some(millis)),
            None => Err(format!("Invalid {setting} setting: {}", js_string(value))),
        }
    }

    pub fn http_idle_timeout_ms(&self) -> Result<f64, String> {
        Ok(self
            .parse_timeout("httpIdleTimeoutMs", "httpIdleTimeoutMs")?
            .unwrap_or(defaults::HTTP_IDLE_TIMEOUT_MS))
    }

    pub fn set_http_idle_timeout_ms(&mut self, millis: f64) -> Result<(), String> {
        let valid = millis.is_finite() && millis >= 0.0;
        if !valid {
            return Err(format!(
                "Invalid httpIdleTimeoutMs setting: {}",
                js_string(&serde_json::Number::from_f64(millis).map_or(Value::Null, Value::Number))
            ));
        }
        self.global.set_number("httpIdleTimeoutMs", millis.floor());
        self.mark_modified("httpIdleTimeoutMs", None);
        self.save();
        Ok(())
    }

    pub fn websocket_connect_timeout_ms(&self) -> Result<Option<f64>, String> {
        self.parse_timeout("websocketConnectTimeoutMs", "websocketConnectTimeoutMs")
    }

    /// Global only: each cache refresh costs money, so a project cannot turn it on.
    pub fn cache_warming_mode(&self) -> &str {
        choice(
            self.global.get("cacheWarming"),
            &schema::CACHE_WARMING_MODES,
            defaults::CACHE_WARMING,
        )
    }

    pub fn set_cache_warming_mode(&mut self, mode: &str) {
        self.global.set_text("cacheWarming", mode);
        self.mark_modified("cacheWarming", None);
        self.save();
    }

    pub fn hide_thinking_block(&self) -> bool {
        self.merged
            .flag("hideThinkingBlock")
            .unwrap_or(defaults::HIDE_THINKING_BLOCK)
    }

    pub fn set_hide_thinking_block(&mut self, hide: bool) {
        self.global.set_flag("hideThinkingBlock", hide);
        self.mark_modified("hideThinkingBlock", None);
        self.save();
    }

    pub fn show_cache_miss_notices(&self) -> bool {
        self.merged
            .flag("showCacheMissNotices")
            .unwrap_or(defaults::SHOW_CACHE_MISS_NOTICES)
    }

    pub fn set_show_cache_miss_notices(&mut self, show: bool) {
        self.global.set_flag("showCacheMissNotices", show);
        self.mark_modified("showCacheMissNotices", None);
        self.save();
    }

    /// `getExternalEditorCommand`: the setting, else `VISUAL`, else `EDITOR`, else the platform
    /// default.
    pub fn external_editor_command(&self) -> String {
        if let Some(editor) = self
            .merged
            .text("externalEditor")
            .filter(|editor| !editor.trim().is_empty())
        {
            return editor.to_owned();
        }
        for name in ["VISUAL", "EDITOR"] {
            if let Ok(editor) = std::env::var(name)
                && !editor.is_empty()
            {
                return editor;
            }
        }
        if cfg!(windows) { "notepad" } else { "nano" }.to_owned()
    }

    pub fn shell_path(&self) -> Option<String> {
        self.merged
            .text("shellPath")
            .filter(|path| !path.is_empty())
            .map(|path| normalize_path(path, PathInput::default()))
    }

    pub fn set_shell_path(&mut self, path: Option<&str>) {
        match path {
            Some(path) => self.global.set_text("shellPath", path),
            None => self.global.remove("shellPath"),
        }
        self.mark_modified("shellPath", None);
        self.save();
    }

    pub fn quiet_startup(&self) -> QuietStartup {
        match self.merged.get("quietStartup") {
            Some(Value::Bool(true)) => QuietStartup::On,
            Some(Value::String(mode)) if mode == "header" => QuietStartup::Header,
            _ if defaults::QUIET_STARTUP => QuietStartup::On,
            _ => QuietStartup::Off,
        }
    }

    pub fn set_quiet_startup(&mut self, quiet: QuietStartup) {
        let value = match quiet {
            QuietStartup::On => Value::Bool(true),
            QuietStartup::Header => Value::String("header".to_owned()),
            QuietStartup::Off => Value::Bool(false),
        };
        self.global.set("quietStartup", value);
        self.mark_modified("quietStartup", None);
        self.save();
    }

    /// Global only.
    pub fn default_project_trust(&self) -> &str {
        choice(
            self.global.get("defaultProjectTrust"),
            &schema::PROJECT_TRUSTS,
            defaults::DEFAULT_PROJECT_TRUST,
        )
    }

    pub fn set_default_project_trust(&mut self, trust: &str) {
        self.global.set_text("defaultProjectTrust", trust);
        self.mark_modified("defaultProjectTrust", None);
        self.save();
    }

    pub fn shell_command_prefix(&self) -> Option<&str> {
        self.merged.text("shellCommandPrefix")
    }

    pub fn set_shell_command_prefix(&mut self, prefix: Option<&str>) {
        match prefix {
            Some(prefix) => self.global.set_text("shellCommandPrefix", prefix),
            None => self.global.remove("shellCommandPrefix"),
        }
        self.mark_modified("shellCommandPrefix", None);
        self.save();
    }

    pub fn npm_command(&self) -> Option<Vec<String>> {
        self.merged.texts("npmCommand")
    }

    pub fn set_npm_command(&mut self, command: Option<&[String]>) {
        match command {
            Some(command) => self.global.set_texts("npmCommand", command),
            None => self.global.remove("npmCommand"),
        }
        self.mark_modified("npmCommand", None);
        self.save();
    }

    pub fn collapse_changelog(&self) -> bool {
        self.merged
            .flag("collapseChangelog")
            .unwrap_or(defaults::COLLAPSE_CHANGELOG)
    }

    pub fn set_collapse_changelog(&mut self, collapse: bool) {
        self.global.set_flag("collapseChangelog", collapse);
        self.mark_modified("collapseChangelog", None);
        self.save();
    }

    pub fn enable_install_telemetry(&self) -> bool {
        self.merged
            .flag("enableInstallTelemetry")
            .unwrap_or(defaults::ENABLE_INSTALL_TELEMETRY)
    }

    pub fn set_enable_install_telemetry(&mut self, enabled: bool) {
        self.global.set_flag("enableInstallTelemetry", enabled);
        self.mark_modified("enableInstallTelemetry", None);
        self.save();
    }

    pub fn enable_analytics(&self) -> bool {
        self.merged
            .flag("enableAnalytics")
            .unwrap_or(defaults::ENABLE_ANALYTICS)
    }

    pub fn tracking_id(&self) -> Option<&str> {
        self.merged.text("trackingId")
    }

    /// Opting in generates a tracking identifier on first use.
    pub fn set_enable_analytics(&mut self, enabled: bool) {
        self.global.set_flag("enableAnalytics", enabled);
        self.mark_modified("enableAnalytics", None);
        if enabled && self.global.text("trackingId").is_none_or(str::is_empty) {
            self.global.set_text("trackingId", &new_uuid());
            self.mark_modified("trackingId", None);
        }
        self.save();
    }

    /// Global only, so a committed project file cannot give every clone the same ID.
    pub fn get_or_create_device_id(&mut self) -> String {
        if let Some(id) = self.global.text("deviceId").filter(|id| !id.is_empty()) {
            return id.to_owned();
        }
        let id = new_uuid();
        self.global.set_text("deviceId", &id);
        self.mark_modified("deviceId", None);
        self.save();
        id.clone()
    }

    pub fn packages(&self) -> Vec<Value> {
        self.merged
            .get("packages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_packages(&mut self, packages: &[Value]) {
        self.global.set("packages", Value::Array(packages.to_vec()));
        self.mark_modified("packages", None);
        self.save();
    }

    pub fn set_project_packages(&mut self, packages: &[Value]) -> Result<(), String> {
        self.update_project("packages", |settings| {
            settings.set("packages", Value::Array(packages.to_vec()));
        })
    }

    pub fn extension_paths(&self) -> Vec<String> {
        self.merged.texts("extensions").unwrap_or_default()
    }

    pub fn set_extension_paths(&mut self, paths: &[String]) {
        self.global.set_texts("extensions", paths);
        self.mark_modified("extensions", None);
        self.save();
    }

    pub fn set_project_extension_paths(&mut self, paths: &[String]) -> Result<(), String> {
        self.update_project("extensions", |settings| {
            settings.set_texts("extensions", paths)
        })
    }

    pub fn skill_paths(&self) -> Vec<String> {
        self.merged.texts("skills").unwrap_or_default()
    }

    pub fn set_skill_paths(&mut self, paths: &[String]) {
        self.global.set_texts("skills", paths);
        self.mark_modified("skills", None);
        self.save();
    }

    pub fn set_project_skill_paths(&mut self, paths: &[String]) -> Result<(), String> {
        self.update_project("skills", |settings| settings.set_texts("skills", paths))
    }

    pub fn prompt_template_paths(&self) -> Vec<String> {
        self.merged.texts("prompts").unwrap_or_default()
    }

    pub fn set_prompt_template_paths(&mut self, paths: &[String]) {
        self.global.set_texts("prompts", paths);
        self.mark_modified("prompts", None);
        self.save();
    }

    pub fn set_project_prompt_template_paths(&mut self, paths: &[String]) -> Result<(), String> {
        self.update_project("prompts", |settings| settings.set_texts("prompts", paths))
    }

    pub fn theme_paths(&self) -> Vec<String> {
        self.merged.texts("themes").unwrap_or_default()
    }

    pub fn set_theme_paths(&mut self, paths: &[String]) {
        self.global.set_texts("themes", paths);
        self.mark_modified("themes", None);
        self.save();
    }

    pub fn set_project_theme_paths(&mut self, paths: &[String]) -> Result<(), String> {
        self.update_project("themes", |settings| settings.set_texts("themes", paths))
    }

    pub fn enable_skill_commands(&self) -> bool {
        self.merged
            .flag("enableSkillCommands")
            .unwrap_or(defaults::ENABLE_SKILL_COMMANDS)
    }

    pub fn set_enable_skill_commands(&mut self, enabled: bool) {
        self.global.set_flag("enableSkillCommands", enabled);
        self.mark_modified("enableSkillCommands", None);
        self.save();
    }

    pub fn thinking_budgets(&self) -> Option<ThinkingBudgets> {
        let budgets = self.merged.object("thinkingBudgets")?;
        let number = |key: &str| {
            budgets
                .get(key)
                .filter(|value| !value.is_null())
                .and_then(Value::as_f64)
        };
        Some(ThinkingBudgets {
            minimal: number("minimal"),
            low: number("low"),
            medium: number("medium"),
            high: number("high"),
        })
    }

    fn terminal(&self, flag: &str) -> Option<bool> {
        self.merged
            .object("terminal")?
            .get(flag)
            .filter(|value| !value.is_null())?
            .as_bool()
    }

    fn set_terminal(&mut self, flag: &str, value: Value) {
        self.global
            .object_mut("terminal")
            .insert(flag.to_owned(), value);
        self.mark_modified("terminal", Some(flag));
        self.save();
    }

    pub fn show_images(&self) -> bool {
        self.terminal("showImages")
            .unwrap_or(defaults::TERMINAL_SHOW_IMAGES)
    }

    pub fn set_show_images(&mut self, show: bool) {
        self.set_terminal("showImages", Value::Bool(show));
    }

    pub fn image_width_cells(&self) -> f64 {
        match self
            .merged
            .object("terminal")
            .and_then(|terminal| terminal.get("imageWidthCells"))
        {
            Some(value) => match value.as_f64().filter(|width| width.is_finite()) {
                Some(width) => width.floor().max(1.0),
                None => defaults::TERMINAL_IMAGE_WIDTH_CELLS,
            },
            None => defaults::TERMINAL_IMAGE_WIDTH_CELLS,
        }
    }

    pub fn set_image_width_cells(&mut self, width: f64) {
        self.set_terminal("imageWidthCells", Value::from(width.floor().max(1.0)));
    }

    /// The setting, else `PI_CLEAR_ON_SHRINK=1`, else off.
    pub fn clear_on_shrink(&self) -> bool {
        if let Some(clear) = self.terminal("clearOnShrink") {
            return clear;
        }
        if std::env::var("PI_CLEAR_ON_SHRINK").is_ok_and(|value| value == "1") {
            return true;
        }
        defaults::TERMINAL_CLEAR_ON_SHRINK
    }

    pub fn set_clear_on_shrink(&mut self, clear: bool) {
        self.set_terminal("clearOnShrink", Value::Bool(clear));
    }

    pub fn show_terminal_progress(&self) -> bool {
        self.terminal("showTerminalProgress")
            .unwrap_or(defaults::TERMINAL_SHOW_PROGRESS)
    }

    pub fn set_show_terminal_progress(&mut self, show: bool) {
        self.set_terminal("showTerminalProgress", Value::Bool(show));
    }

    pub fn tui_mode(&self) -> &str {
        choice(
            self.merged.get("tuiMode"),
            &schema::TUI_MODES,
            defaults::TUI_MODE,
        )
    }

    pub fn set_tui_mode(&mut self, mode: &str) {
        self.global.set_text("tuiMode", mode);
        self.mark_modified("tuiMode", None);
        self.save();
    }

    pub fn fullscreen_exit_output(&self) -> &str {
        choice(
            self.merged.get("fullscreenExitOutput"),
            &schema::FULLSCREEN_EXIT_OUTPUTS,
            defaults::FULLSCREEN_EXIT_OUTPUT,
        )
    }

    pub fn set_fullscreen_exit_output(&mut self, output: &str) {
        self.global.set_text("fullscreenExitOutput", output);
        self.mark_modified("fullscreenExitOutput", None);
        self.save();
    }

    pub fn fullscreen_scrollbar(&self) -> &str {
        choice(
            self.merged.get("fullscreenScrollbar"),
            &schema::FULLSCREEN_SCROLLBARS,
            defaults::FULLSCREEN_SCROLLBAR,
        )
    }

    pub fn set_fullscreen_scrollbar(&mut self, mode: &str) {
        self.global.set_text("fullscreenScrollbar", mode);
        self.mark_modified("fullscreenScrollbar", None);
        self.save();
    }

    pub fn fullscreen_copy_on_select(&self) -> bool {
        self.merged
            .flag("fullscreenCopyOnSelect")
            .unwrap_or(defaults::FULLSCREEN_COPY_ON_SELECT)
    }

    pub fn set_fullscreen_copy_on_select(&mut self, enabled: bool) {
        self.global.set_flag("fullscreenCopyOnSelect", enabled);
        self.mark_modified("fullscreenCopyOnSelect", None);
        self.save();
    }

    pub fn fullscreen_wheel_scroll_lines(&self) -> WheelScrollLines {
        match self
            .merged
            .get("fullscreenWheelScrollLines")
            .and_then(Value::as_f64)
            .filter(|lines| lines.is_finite())
        {
            Some(lines) => WheelScrollLines::Lines(lines.floor().clamp(1.0, 100.0) as u8),
            None => WheelScrollLines::Auto,
        }
    }

    pub fn set_fullscreen_wheel_scroll_lines(&mut self, lines: WheelScrollLines) {
        let value = match lines {
            WheelScrollLines::Auto => Value::String("auto".to_owned()),
            WheelScrollLines::Lines(lines) => Value::from(lines.clamp(1, 100)),
        };
        self.global.set("fullscreenWheelScrollLines", value);
        self.mark_modified("fullscreenWheelScrollLines", None);
        self.save();
    }

    fn images(&self, flag: &str) -> Option<bool> {
        self.merged
            .object("images")?
            .get(flag)
            .filter(|value| !value.is_null())?
            .as_bool()
    }

    fn set_images(&mut self, flag: &str, value: bool) {
        self.global
            .object_mut("images")
            .insert(flag.to_owned(), Value::Bool(value));
        self.mark_modified("images", Some(flag));
        self.save();
    }

    pub fn image_auto_resize(&self) -> bool {
        self.images("autoResize")
            .unwrap_or(defaults::IMAGES_AUTO_RESIZE)
    }

    pub fn set_image_auto_resize(&mut self, enabled: bool) {
        self.set_images("autoResize", enabled);
    }

    pub fn block_images(&self) -> bool {
        self.images("blockImages")
            .unwrap_or(defaults::IMAGES_BLOCK_IMAGES)
    }

    pub fn set_block_images(&mut self, blocked: bool) {
        self.set_images("blockImages", blocked);
    }

    pub fn enabled_models(&self) -> Option<Vec<String>> {
        self.merged.texts("enabledModels")
    }

    pub fn set_enabled_models(&mut self, patterns: Option<&[String]>) {
        match patterns {
            Some(patterns) => self.global.set_texts("enabledModels", patterns),
            None => self.global.remove("enabledModels"),
        }
        self.mark_modified("enabledModels", None);
        self.save();
    }

    /// `getDefaultTools`: the resolved selection, or None when no layer sets `defaultTools`.
    pub fn default_tools(&self) -> Option<Vec<String>> {
        let raw = self.merged.raw("defaultTools")?;
        let entries: Vec<String> = match raw {
            Value::Array(items) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        Some(resolve_default_tools(&entries))
    }

    pub fn double_escape_action(&self) -> &str {
        text_or(
            self.merged.get("doubleEscapeAction"),
            defaults::DOUBLE_ESCAPE_ACTION,
        )
    }

    pub fn set_double_escape_action(&mut self, action: &str) {
        self.global.set_text("doubleEscapeAction", action);
        self.mark_modified("doubleEscapeAction", None);
        self.save();
    }

    pub fn tree_filter_mode(&self) -> &str {
        choice(
            self.merged.get("treeFilterMode"),
            &schema::TREE_FILTER_MODES,
            defaults::TREE_FILTER_MODE,
        )
    }

    pub fn set_tree_filter_mode(&mut self, mode: &str) {
        self.global.set_text("treeFilterMode", mode);
        self.mark_modified("treeFilterMode", None);
        self.save();
    }

    pub fn show_hardware_cursor(&self) -> bool {
        self.merged
            .flag("showHardwareCursor")
            .unwrap_or_else(|| std::env::var("PI_HARDWARE_CURSOR").is_ok_and(|value| value == "1"))
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.global.set_flag("showHardwareCursor", enabled);
        self.mark_modified("showHardwareCursor", None);
        self.save();
    }

    pub fn editor_padding_x(&self) -> f64 {
        self.merged
            .number("editorPaddingX")
            .unwrap_or(defaults::EDITOR_PADDING_X)
    }

    pub fn set_editor_padding_x(&mut self, padding: f64) {
        self.global
            .set_number("editorPaddingX", padding.floor().clamp(0.0, 3.0));
        self.mark_modified("editorPaddingX", None);
        self.save();
    }

    pub fn output_pad(&self) -> u8 {
        let none = self.merged.get("outputPad").and_then(Value::as_f64) == Some(0.0);
        if none { 0 } else { defaults::OUTPUT_PAD }
    }

    pub fn set_output_pad(&mut self, padding: u8) {
        self.global.set("outputPad", Value::from(padding.min(1)));
        self.mark_modified("outputPad", None);
        self.save();
    }

    pub fn autocomplete_max_visible(&self) -> f64 {
        self.merged
            .number("autocompleteMaxVisible")
            .unwrap_or(defaults::AUTOCOMPLETE_MAX_VISIBLE)
    }

    pub fn set_autocomplete_max_visible(&mut self, max_visible: f64) {
        self.global.set_number(
            "autocompleteMaxVisible",
            max_visible.floor().clamp(3.0, 20.0),
        );
        self.mark_modified("autocompleteMaxVisible", None);
        self.save();
    }

    pub fn code_block_indent(&self) -> &str {
        self.merged
            .object("markdown")
            .and_then(|markdown| markdown.get("codeBlockIndent"))
            .filter(|value| !value.is_null())
            .and_then(Value::as_str)
            .unwrap_or(defaults::MARKDOWN_CODE_BLOCK_INDENT)
    }

    pub fn mermaid_rendering_mode(&self) -> &str {
        choice(
            self.merged
                .object("markdown")
                .and_then(|markdown| markdown.get("mermaid")),
            &schema::MERMAID_MODES,
            defaults::MARKDOWN_MERMAID,
        )
    }

    pub fn set_mermaid_rendering_mode(&mut self, mode: &str) {
        self.global
            .object_mut("markdown")
            .insert("mermaid".to_owned(), Value::String(mode.to_owned()));
        self.mark_modified("markdown", Some("mermaid"));
        self.save();
    }

    pub fn warnings(&self) -> WarningSettings {
        WarningSettings {
            anthropic_extra_usage: self
                .merged
                .object("warnings")
                .and_then(|warnings| warnings.get("anthropicExtraUsage"))
                .filter(|value| !value.is_null())
                .and_then(Value::as_bool)
                .unwrap_or(defaults::WARNINGS_ANTHROPIC_EXTRA_USAGE),
        }
    }

    pub fn set_warnings(&mut self, warnings: WarningSettings) {
        let mut values = Map::new();
        values.insert(
            "anthropicExtraUsage".to_owned(),
            Value::Bool(warnings.anthropic_extra_usage),
        );
        self.global.set("warnings", Value::Object(values));
        self.mark_modified("warnings", None);
        self.save();
    }
}

/// `randomUUID()`: a version 7 UUID, which pi-rs already has, in Node's grouped form.
fn new_uuid() -> String {
    pi_durable::ai::utils::uuid::uuidv7(None).unwrap_or_default()
}
