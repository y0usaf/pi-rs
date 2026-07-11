//! Project trust — port of the spec's `core/trust-manager.ts` (store,
//! options, inputs check) and the decision core of
//! `core/project-trust.ts` (`resolveProjectTrusted`).
//!
//! The substrate never prompts: where the spec runs `ctx.ui.select`, the
//! resolver returns [`TrustResolution::Ask`]; a frontend prompts with
//! [`project_trust_options`] and persists the pick via
//! [`save_trust_option`]; a headless caller treats `Ask` as untrusted
//! (spec: `!hasUI → false`).
//!
//! The `project_trust` extension event (spec: `emitProjectTrustEvent`) is
//! emitted by the caller through [`Host::emit`](crate::Host::emit) and
//! folded with [`trust_event_result`]: the first handler answering
//! `trusted = "yes"` or `"no"` wins, `"undecided"` falls through, failing
//! handlers are collected — string keys at the seam, no closed enum.

use std::collections::BTreeMap;
use std::path::Path;

use crate::Outcome;
use crate::os;
use crate::paths::{canonicalize_path, process_cwd, resolve_path};

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error("failed to read trust store {path}: {message}")]
    Read { path: String, message: String },

    #[error("invalid trust store {path}: {message}")]
    Invalid { path: String, message: String },

    #[error("failed to write trust store {path}: {message}")]
    Write { path: String, message: String },

    #[error("failed to acquire trust store lock {path}: {message}")]
    Lock { path: String, message: String },
}

/// Spec `ProjectTrustStoreEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEntry {
    pub path: String,
    pub decision: bool,
}

/// Spec `ProjectTrustUpdate`: `decision = None` deletes the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustUpdate {
    pub path: String,
    pub decision: Option<bool>,
}

/// Spec `ProjectTrustOption`: one prompt choice with the store updates it
/// implies (session-only choices carry no updates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustOption {
    pub label: String,
    pub trusted: bool,
    pub updates: Vec<TrustUpdate>,
    pub saved_path: Option<String>,
}

fn normalize_cwd(cwd: &str) -> String {
    canonicalize_path(&resolve_path(cwd, &process_cwd(), false))
}

/// Spec `getProjectTrustPath`.
pub fn project_trust_path(cwd: &str) -> String {
    normalize_cwd(cwd)
}

/// Spec `getProjectTrustParentPath`.
pub fn project_trust_parent_path(cwd: &str) -> Option<String> {
    let trust_path = project_trust_path(cwd);
    let parent = os::dirname(&trust_path);
    if parent == trust_path {
        None
    } else {
        Some(parent)
    }
}

/// Spec `formatProjectTrustPrompt` (`project-trust.ts`).
pub fn format_project_trust_prompt(cwd: &str) -> String {
    format!(
        "Trust project folder?\n{cwd}\n\nThis allows pi to load .pi settings and resources, install missing project packages, and execute project extensions."
    )
}

/// Spec `getProjectTrustOptions`: Trust / Trust parent / (session-only) /
/// Do not trust / (session-only), in that order.
pub fn project_trust_options(cwd: &str, include_session_only: bool) -> Vec<TrustOption> {
    let trust_path = project_trust_path(cwd);
    let mut options = vec![TrustOption {
        label: "Trust".to_owned(),
        trusted: true,
        updates: vec![TrustUpdate {
            path: trust_path.clone(),
            decision: Some(true),
        }],
        saved_path: Some(trust_path.clone()),
    }];
    if let Some(parent_path) = project_trust_parent_path(cwd) {
        options.push(TrustOption {
            label: format!("Trust parent folder ({parent_path})"),
            trusted: true,
            updates: vec![
                TrustUpdate {
                    path: parent_path.clone(),
                    decision: Some(true),
                },
                TrustUpdate {
                    path: trust_path.clone(),
                    decision: None,
                },
            ],
            saved_path: Some(parent_path),
        });
    }
    if include_session_only {
        options.push(TrustOption {
            label: "Trust (this session only)".to_owned(),
            trusted: true,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options.push(TrustOption {
        label: "Do not trust".to_owned(),
        trusted: false,
        updates: vec![TrustUpdate {
            path: trust_path.clone(),
            decision: Some(false),
        }],
        saved_path: Some(trust_path),
    });
    if include_session_only {
        options.push(TrustOption {
            label: "Do not trust (this session only)".to_owned(),
            trusted: false,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options
}

/// Spec `hasProjectConfigDir`: `cwd/.pi` exists (the canonicalized cwd
/// only — ancestors are not checked).
pub fn has_project_config_dir(cwd: &str) -> bool {
    Path::new(&normalize_cwd(cwd))
        .join(crate::discover::CONFIG_DIR_NAME)
        .exists()
}

/// Spec `hasProjectTrustInputs`: a project config dir at cwd, or an
/// `.agents/skills` directory in cwd or any ancestor.
pub fn has_project_trust_inputs(cwd: &str) -> bool {
    let mut current = normalize_cwd(cwd);
    if has_project_config_dir(&current) {
        return true;
    }
    loop {
        if Path::new(&current).join(".agents").join("skills").exists() {
            return true;
        }
        let parent = os::dirname(&current);
        if parent == current {
            return false;
        }
        current = parent;
    }
}

// ---------------------------------------------------------------------------
// Trust file (trust.json under the agent dir)
// ---------------------------------------------------------------------------

type TrustFile = BTreeMap<String, Option<bool>>;

/// Spec `readTrustFile`: missing → empty; must be a JSON object whose
/// values are `true`, `false`, or `null`.
fn read_trust_file(path: &str) -> Result<TrustFile, TrustError> {
    if !Path::new(path).exists() {
        return Ok(TrustFile::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| TrustError::Read {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| TrustError::Read {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
    let serde_json::Value::Object(map) = parsed else {
        return Err(TrustError::Invalid {
            path: path.to_owned(),
            message: "expected an object".to_owned(),
        });
    };
    let mut data = TrustFile::new();
    for (key, value) in map {
        let decision = match value {
            serde_json::Value::Bool(b) => Some(b),
            serde_json::Value::Null => None,
            _ => {
                return Err(TrustError::Invalid {
                    path: path.to_owned(),
                    message: format!("value for {key:?} must be true, false, or null"),
                });
            }
        };
        data.insert(key, decision);
    }
    Ok(data)
}

/// Spec `writeTrustFile`: keys sorted (`BTreeMap` iteration), two-space
/// indent, trailing newline.
fn write_trust_file(path: &str, data: &TrustFile) -> Result<(), TrustError> {
    let mut map = serde_json::Map::new();
    for (key, decision) in data {
        map.insert(
            key.clone(),
            match decision {
                Some(b) => serde_json::Value::Bool(*b),
                None => serde_json::Value::Null,
            },
        );
    }
    let body = serde_json::to_string_pretty(&serde_json::Value::Object(map)).map_err(|e| {
        TrustError::Write {
            path: path.to_owned(),
            message: e.to_string(),
        }
    })?;
    let dir = os::dirname(path);
    std::fs::create_dir_all(&dir).map_err(|e| TrustError::Write {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    std::fs::write(path, format!("{body}\n")).map_err(|e| TrustError::Write {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

/// Spec `findNearestTrustEntry`: walk cwd and its ancestors for the first
/// `true`/`false` entry (`null` entries fall through to the parent).
fn find_nearest_trust_entry(data: &TrustFile, cwd: &str) -> Option<TrustEntry> {
    let mut current = normalize_cwd(cwd);
    loop {
        if let Some(Some(decision)) = data.get(&current) {
            return Some(TrustEntry {
                path: current,
                decision: *decision,
            });
        }
        let parent = os::dirname(&current);
        if parent == current {
            return None;
        }
        current = parent;
    }
}

/// Held lock on the trust file (spec: `proper-lockfile`'s mkdir-based
/// `trust.json.lock`); removed on drop.
struct TrustLock(String);

impl Drop for TrustLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Spec `acquireTrustLockSync`: mkdir `trust.json.lock`, retrying a held
/// lock 10 × 20ms before giving up.
fn acquire_trust_lock(path: &str) -> Result<TrustLock, TrustError> {
    const MAX_ATTEMPTS: u32 = 10;
    const DELAY_MS: u64 = 20;
    let dir = os::dirname(path);
    std::fs::create_dir_all(&dir).map_err(|e| TrustError::Lock {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let lock_path = format!("{path}.lock");
    for attempt in 1..=MAX_ATTEMPTS {
        match std::fs::create_dir(&lock_path) {
            Ok(()) => return Ok(TrustLock(lock_path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < MAX_ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(DELAY_MS));
            }
            Err(e) => {
                return Err(TrustError::Lock {
                    path: path.to_owned(),
                    message: e.to_string(),
                });
            }
        }
    }
    Err(TrustError::Lock {
        path: path.to_owned(),
        message: "lock is held".to_owned(),
    })
}

/// Spec `ProjectTrustStore`: `trust.json` under the agent dir, guarded by
/// a lock file, decisions looked up nearest-ancestor-first.
pub struct ProjectTrustStore {
    trust_path: String,
}

impl ProjectTrustStore {
    pub fn new(agent_dir: &str) -> Self {
        let resolved = resolve_path(agent_dir, &process_cwd(), false);
        Self {
            trust_path: os::join(&[resolved, "trust.json".to_owned()]),
        }
    }

    /// The nearest-ancestor decision for `cwd`, or `None` when undecided.
    pub fn get(&self, cwd: &str) -> Result<Option<bool>, TrustError> {
        Ok(self.get_entry(cwd)?.map(|e| e.decision))
    }

    /// Spec `getEntry`: the nearest-ancestor entry with its path.
    pub fn get_entry(&self, cwd: &str) -> Result<Option<TrustEntry>, TrustError> {
        let _lock = acquire_trust_lock(&self.trust_path)?;
        let data = read_trust_file(&self.trust_path)?;
        Ok(find_nearest_trust_entry(&data, cwd))
    }

    /// Spec `set`: `None` deletes the entry for `cwd`.
    pub fn set(&self, cwd: &str, decision: Option<bool>) -> Result<(), TrustError> {
        self.set_many(&[TrustUpdate {
            path: cwd.to_owned(),
            decision,
        }])
    }

    /// Spec `setMany`: apply updates under one lock and rewrite the file.
    pub fn set_many(&self, updates: &[TrustUpdate]) -> Result<(), TrustError> {
        let _lock = acquire_trust_lock(&self.trust_path)?;
        let mut data = read_trust_file(&self.trust_path)?;
        for update in updates {
            let key = normalize_cwd(&update.path);
            match update.decision {
                None => {
                    data.remove(&key);
                }
                Some(decision) => {
                    data.insert(key, Some(decision));
                }
            }
        }
        write_trust_file(&self.trust_path, &data)
    }
}

// ---------------------------------------------------------------------------
// Trust resolution (project-trust.ts decision core)
// ---------------------------------------------------------------------------

/// A `project_trust` handler's answer, folded from emit outcomes (spec:
/// `ProjectTrustEventResult` with `trusted` already mapped from
/// `"yes"`/`"no"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustEventResult {
    pub trusted: bool,
    pub remember: bool,
}

/// Fold `project_trust` emit outcomes (spec: `emitProjectTrustEvent`):
/// the first handler returning a result whose `trusted` is not
/// `"undecided"` wins (`trusted = "yes"` maps to true, anything else to
/// false, per the spec's `result.trusted === "yes"`); failing handlers
/// are collected as `(source, error)` and fall through.
pub fn trust_event_result(
    outcomes: &[Outcome],
) -> (Option<TrustEventResult>, Vec<(String, String)>) {
    let mut errors = Vec::new();
    for outcome in outcomes {
        match &outcome.result {
            Err(e) => errors.push((outcome.source.clone(), e.clone())),
            Ok(None) => errors.push((
                outcome.source.clone(),
                "project_trust handler returned no result".to_owned(),
            )),
            Ok(Some(value)) => {
                let trusted = value.get("trusted").and_then(serde_json::Value::as_str);
                if trusted == Some("undecided") {
                    continue;
                }
                let remember =
                    value.get("remember").and_then(serde_json::Value::as_bool) == Some(true);
                return (
                    Some(TrustEventResult {
                        trusted: trusted == Some("yes"),
                        remember,
                    }),
                    errors,
                );
            }
        }
    }
    (None, errors)
}

/// The substrate's answer: a decision, or "the frontend must ask" (spec:
/// the `ctx.ui.select` branch of `resolveProjectTrusted`; headless
/// callers map `Ask` to false, per the spec's `!hasUI → false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustResolution {
    Decided(bool),
    Ask,
}

/// Inputs to [`resolve_project_trusted`] (spec:
/// `ResolveProjectTrustedOptions`). `default_project_trust` is the
/// settings value (`"always"` / `"never"` / `"ask"`; `None` = `"ask"`);
/// `extension_result` is the folded `project_trust` event answer.
pub struct ResolveProjectTrust<'a> {
    pub cwd: &'a str,
    pub store: &'a ProjectTrustStore,
    pub trust_override: Option<bool>,
    pub default_project_trust: Option<&'a str>,
    pub extension_result: Option<TrustEventResult>,
}

/// Spec `resolveProjectTrusted` decision order: explicit override → no
/// trust inputs (trivially trusted) → extension event answer (persisted
/// when `remember`) → stored decision → default setting → ask.
pub fn resolve_project_trusted(
    options: &ResolveProjectTrust<'_>,
) -> Result<TrustResolution, TrustError> {
    if let Some(trusted) = options.trust_override {
        return Ok(TrustResolution::Decided(trusted));
    }
    if !has_project_trust_inputs(options.cwd) {
        return Ok(TrustResolution::Decided(true));
    }
    if let Some(result) = options.extension_result {
        if result.remember {
            options.store.set(options.cwd, Some(result.trusted))?;
        }
        return Ok(TrustResolution::Decided(result.trusted));
    }
    if let Some(decision) = options.store.get(options.cwd)? {
        return Ok(TrustResolution::Decided(decision));
    }
    match options.default_project_trust.unwrap_or("ask") {
        "always" => Ok(TrustResolution::Decided(true)),
        "never" => Ok(TrustResolution::Decided(false)),
        _ => Ok(TrustResolution::Ask),
    }
}

/// Spec `saveProjectTrustPromptResult`: persist a prompted pick
/// (session-only options carry no updates and write nothing).
pub fn save_trust_option(
    store: &ProjectTrustStore,
    option: &TrustOption,
) -> Result<(), TrustError> {
    if option.updates.is_empty() {
        return Ok(());
    }
    store.set_many(&option.updates)
}

/// Install the project-trust persistence/discovery mechanism as `pi.trust`.
/// Selection, prompting, warnings, and command routing remain Lua policy.
pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let store = std::sync::Arc::new(ProjectTrustStore::new(&crate::discover::agent_dir()));
    let table = lua.create_table()?;

    let shared = std::sync::Arc::clone(&store);
    table.set(
        "get",
        lua.create_function(move |_, cwd: String| shared.get(&cwd).map_err(mlua::Error::external))?,
    )?;
    let shared = std::sync::Arc::clone(&store);
    table.set(
        "get_entry",
        lua.create_function(move |lua, cwd: String| {
            let Some(entry) = shared.get_entry(&cwd).map_err(mlua::Error::external)? else {
                return Ok(None);
            };
            let value = lua.create_table()?;
            value.set("path", entry.path)?;
            value.set("decision", entry.decision)?;
            Ok(Some(value))
        })?,
    )?;
    let shared = std::sync::Arc::clone(&store);
    table.set(
        "set",
        lua.create_function(move |_, (cwd, decision): (String, Option<bool>)| {
            shared.set(&cwd, decision).map_err(mlua::Error::external)
        })?,
    )?;
    let shared = std::sync::Arc::clone(&store);
    table.set(
        "set_many",
        lua.create_function(move |_, updates: Vec<mlua::Table>| {
            let updates = updates
                .into_iter()
                .map(|update| {
                    Ok(TrustUpdate {
                        path: update.get("path")?,
                        decision: update.get("decision")?,
                    })
                })
                .collect::<mlua::Result<Vec<_>>>()?;
            shared.set_many(&updates).map_err(mlua::Error::external)
        })?,
    )?;
    table.set(
        "options",
        lua.create_function(
            move |lua, (cwd, include_session_only): (String, Option<bool>)| {
                let result = lua.create_table()?;
                for option in project_trust_options(&cwd, include_session_only.unwrap_or(false)) {
                    let value = lua.create_table()?;
                    value.set("label", option.label)?;
                    value.set("trusted", option.trusted)?;
                    value.set("savedPath", option.saved_path)?;
                    let updates = lua.create_table()?;
                    for update in option.updates {
                        let item = lua.create_table()?;
                        item.set("path", update.path)?;
                        item.set("decision", update.decision)?;
                        updates.push(item)?;
                    }
                    value.set("updates", updates)?;
                    result.push(value)?;
                }
                Ok(result)
            },
        )?,
    )?;
    table.set(
        "has_inputs",
        lua.create_function(|_, cwd: String| Ok(has_project_trust_inputs(&cwd)))?,
    )?;
    table.set(
        "has_config_dir",
        lua.create_function(|_, cwd: String| Ok(has_project_config_dir(&cwd)))?,
    )?;
    table.set(
        "path",
        lua.create_function(|_, cwd: String| Ok(project_trust_path(&cwd)))?,
    )?;
    table.set(
        "prompt",
        lua.create_function(|_, cwd: String| Ok(format_project_trust_prompt(&cwd)))?,
    )?;
    pi.set("trust", table)
}
