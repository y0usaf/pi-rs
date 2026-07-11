//! Port of `core/auth-storage.ts` — credential storage for API keys and
//! OAuth tokens (`auth.json`), with file locking so concurrent pi-rs
//! instances don't race token refreshes.
//!
//! Divergences (recorded):
//! - the spec's `proper-lockfile` is a mkdir-based `<file>.lock` with
//!   staleness; ported directly (as in `pi-rs-host`'s trust store) with
//!   the spec's retry schedules, minus jitter (`randomize: true`);
//! - the spec's pluggable backend interface collapses to the two
//!   implementations it ships (file / in-memory) — an enum, revisited
//!   if a consumer needs a custom backend;
//! - `setFallbackResolver` (models.json custom-provider keys) lands
//!   with the models.json port (WS7); until then the fallback branches
//!   resolve to nothing;
//! - credential order in `auth.json` is not preserved on reload
//!   (`HashMap`); no landed consumer observes order.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pi_rs_ai::registry::{find_env_keys, get_env_api_key};
use pi_rs_ai_auth::{OAuthCredentials, OAuthLoginCallbacks, get_oauth_api_key, get_oauth_provider};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::resolve_config_value::resolve_config_value;

/// Spec: `getAuthPath()` — `<agent dir>/auth.json` (the agent dir is
/// [`crate::discover::agent_dir`], `PI_CODING_AGENT_DIR` overridable).
pub fn get_auth_path() -> PathBuf {
    Path::new(&crate::discover::agent_dir()).join("auth.json")
}

/// Errors surfaced by the storage layer (the spec records most of these
/// via `recordError` and keeps going; `Err` is reserved for operations
/// the spec lets throw, e.g. `login`).
#[derive(Debug, Error)]
pub enum AuthStorageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Auth(#[from] pi_rs_ai_auth::AuthError),
    #[error("{0}")]
    Message(String),
}

/// Spec: `AuthCredential` — `{type:"api_key"}` | `{type:"oauth"}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthCredential {
    #[serde(rename = "api_key")]
    ApiKey { key: String },
    #[serde(rename = "oauth")]
    OAuth(OAuthCredentials),
}

/// Spec: `AuthStorageData`.
pub type AuthStorageData = HashMap<String, AuthCredential>;

/// Spec: `AuthStatus`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthStatus {
    pub configured: bool,
    /// `"stored" | "runtime" | "environment" | "fallback" |
    /// "models_json_key" | "models_json_command"`.
    pub source: Option<&'static str>,
    pub label: Option<String>,
}

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// Held mkdir lock (`<auth.json>.lock`); removed on drop.
struct FileLock(PathBuf);

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Spec: `proper-lockfile`'s `stale: 30000` — a lock directory older
/// than 30s is abandoned and may be broken.
fn break_stale_lock(lock_path: &Path) {
    let Ok(meta) = std::fs::metadata(lock_path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    if modified
        .elapsed()
        .map(|age| age >= Duration::from_millis(30_000))
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

/// Spec: `FileAuthStorageBackend` + `InMemoryAuthStorageBackend`.
pub enum AuthStorageBackend {
    File {
        auth_path: PathBuf,
    },
    InMemory {
        value: std::sync::Mutex<Option<String>>,
    },
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

impl AuthStorageBackend {
    /// Spec: `ensureParentDir` + `ensureFileExists` (0700 dir, 0600 file).
    fn ensure_file(auth_path: &Path) -> Result<(), AuthStorageError> {
        if let Some(dir) = auth_path.parent()
            && !dir.exists()
        {
            std::fs::create_dir_all(dir)?;
            set_mode(dir, 0o700);
        }
        if !auth_path.exists() {
            std::fs::write(auth_path, "{}")?;
            set_mode(auth_path, 0o600);
        }
        Ok(())
    }

    fn write_auth_file(auth_path: &Path, next: &str) -> Result<(), AuthStorageError> {
        std::fs::write(auth_path, next)?;
        set_mode(auth_path, 0o600);
        Ok(())
    }

    /// Spec: `acquireLockSyncWithRetry` — 10 attempts, 20ms apart.
    fn acquire_lock_sync(auth_path: &Path) -> Result<FileLock, AuthStorageError> {
        let lock_path = auth_path.with_extension("json.lock");
        for attempt in 1..=10u32 {
            if let Some(lock) = try_acquire(&lock_path) {
                return Ok(lock);
            }
            if attempt < 10 {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Err(AuthStorageError::Message(format!(
            "Failed to acquire auth storage lock: {}",
            lock_path.display()
        )))
    }

    /// Spec: the async `lockfile.lock` retry schedule — 10 retries,
    /// factor 2, minTimeout 100ms, maxTimeout 10s (jitter not ported).
    async fn acquire_lock_async(auth_path: &Path) -> Result<FileLock, AuthStorageError> {
        let lock_path = auth_path.with_extension("json.lock");
        let mut delay_ms = 100u64;
        for attempt in 0..=10u32 {
            if let Some(lock) = try_acquire(&lock_path) {
                return Ok(lock);
            }
            if attempt < 10 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                delay_ms = (delay_ms * 2).min(10_000);
            }
        }
        Err(AuthStorageError::Message(format!(
            "Failed to acquire auth storage lock: {}",
            lock_path.display()
        )))
    }

    fn read_current(auth_path: &Path) -> Option<String> {
        std::fs::read_to_string(auth_path).ok()
    }

    /// Spec: `withLock(fn)` — the closure receives the current file
    /// content and returns `(result, next)`; `next` is written back
    /// under the lock.
    pub fn with_lock<T>(
        &self,
        f: impl FnOnce(Option<&str>) -> Result<(T, Option<String>), AuthStorageError>,
    ) -> Result<T, AuthStorageError> {
        match self {
            AuthStorageBackend::File { auth_path } => {
                Self::ensure_file(auth_path)?;
                let _lock = Self::acquire_lock_sync(auth_path)?;
                let current = Self::read_current(auth_path);
                let (result, next) = f(current.as_deref())?;
                if let Some(next) = next {
                    Self::write_auth_file(auth_path, &next)?;
                }
                Ok(result)
            }
            AuthStorageBackend::InMemory { value } => {
                let mut guard = value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let (result, next) = f(guard.as_deref())?;
                if let Some(next) = next {
                    *guard = Some(next);
                }
                Ok(result)
            }
        }
    }

    /// Spec: `withLockAsync(fn)`.
    pub async fn with_lock_async<T, F, Fut>(&self, f: F) -> Result<T, AuthStorageError>
    where
        F: FnOnce(Option<String>) -> Fut,
        Fut: Future<Output = Result<(T, Option<String>), AuthStorageError>>,
    {
        match self {
            AuthStorageBackend::File { auth_path } => {
                Self::ensure_file(auth_path)?;
                let _lock = Self::acquire_lock_async(auth_path).await?;
                let current = Self::read_current(auth_path);
                let (result, next) = f(current).await?;
                if let Some(next) = next {
                    Self::write_auth_file(auth_path, &next)?;
                }
                Ok(result)
            }
            AuthStorageBackend::InMemory { value } => {
                let current = value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let (result, next) = f(current).await?;
                if let Some(next) = next {
                    *value
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(next);
                }
                Ok(result)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AuthStorage
// ---------------------------------------------------------------------------

/// Spec: `AuthStorage` — credential storage backed by `auth.json`.
pub struct AuthStorage {
    data: AuthStorageData,
    runtime_overrides: HashMap<String, String>,
    load_error: Option<String>,
    errors: Vec<String>,
    storage: AuthStorageBackend,
}

fn parse_storage_data(content: Option<&str>) -> Result<AuthStorageData, serde_json::Error> {
    match content {
        None | Some("") => Ok(AuthStorageData::new()),
        Some(content) => serde_json::from_str(content),
    }
}

fn serialize_storage_data(data: &AuthStorageData) -> String {
    // Spec: `JSON.stringify(merged, null, 2)`.
    serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".to_owned())
}

impl AuthStorage {
    fn new(storage: AuthStorageBackend) -> Self {
        let mut this = Self {
            data: AuthStorageData::new(),
            runtime_overrides: HashMap::new(),
            load_error: None,
            errors: Vec::new(),
            storage,
        };
        this.reload();
        this
    }

    /// Spec: `AuthStorage.create(authPath?)`.
    pub fn create(auth_path: Option<PathBuf>) -> Self {
        Self::new(AuthStorageBackend::File {
            auth_path: auth_path.unwrap_or_else(get_auth_path),
        })
    }

    /// Spec: `AuthStorage.inMemory(data?)`.
    pub fn in_memory(data: AuthStorageData) -> Self {
        let backend = AuthStorageBackend::InMemory {
            value: std::sync::Mutex::new(Some(serialize_storage_data(&data))),
        };
        Self::new(backend)
    }

    /// Spec: `setRuntimeApiKey` — CLI `--api-key`, not persisted.
    pub fn set_runtime_api_key(&mut self, provider: &str, api_key: &str) {
        self.runtime_overrides
            .insert(provider.to_owned(), api_key.to_owned());
    }

    /// Spec: `removeRuntimeApiKey`.
    pub fn remove_runtime_api_key(&mut self, provider: &str) {
        self.runtime_overrides.remove(provider);
    }

    fn record_error(&mut self, error: impl std::fmt::Display) {
        self.errors.push(error.to_string());
    }

    /// Spec: `reload()`.
    pub fn reload(&mut self) {
        let read = self
            .storage
            .with_lock(|current| Ok((current.map(str::to_owned), None)));
        match read.and_then(|content| Ok(parse_storage_data(content.as_deref())?)) {
            Ok(data) => {
                self.data = data;
                self.load_error = None;
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
                self.record_error(error);
            }
        }
    }

    /// Spec: `persistProviderChange`.
    fn persist_provider_change(&mut self, provider: &str, credential: Option<&AuthCredential>) {
        if self.load_error.is_some() {
            return;
        }
        let result = self.storage.with_lock(|current| {
            let mut merged = parse_storage_data(current)?;
            match credential {
                Some(credential) => {
                    merged.insert(provider.to_owned(), credential.clone());
                }
                None => {
                    merged.remove(provider);
                }
            }
            Ok(((), Some(serialize_storage_data(&merged))))
        });
        if let Err(error) = result {
            self.record_error(error);
        }
    }

    /// Spec: `get(provider)`.
    pub fn get(&self, provider: &str) -> Option<&AuthCredential> {
        self.data.get(provider)
    }

    /// Spec: `set(provider, credential)`.
    pub fn set(&mut self, provider: &str, credential: AuthCredential) {
        self.data.insert(provider.to_owned(), credential.clone());
        self.persist_provider_change(provider, Some(&credential));
    }

    /// Spec: `remove(provider)`.
    pub fn remove(&mut self, provider: &str) {
        self.data.remove(provider);
        self.persist_provider_change(provider, None);
    }

    /// Spec: `list()`.
    pub fn list(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    /// Spec: `has(provider)` — credentials exist in auth.json.
    pub fn has(&self, provider: &str) -> bool {
        self.data.contains_key(provider)
    }

    /// Spec: `hasAuth(provider)` — any form of auth, without refreshing.
    pub fn has_auth(&self, provider: &str) -> bool {
        if self.runtime_overrides.contains_key(provider) {
            return true;
        }
        if self.data.contains_key(provider) {
            return true;
        }
        if get_env_api_key(provider).is_some() {
            return true;
        }
        // Fallback resolver (models.json custom providers): WS7.
        false
    }

    /// Spec: `getAuthStatus(provider)`.
    pub fn get_auth_status(&self, provider: &str) -> AuthStatus {
        if self.data.contains_key(provider) {
            return AuthStatus {
                configured: true,
                source: Some("stored"),
                label: None,
            };
        }

        if self.runtime_overrides.contains_key(provider) {
            return AuthStatus {
                configured: false,
                source: Some("runtime"),
                label: Some("--api-key".to_owned()),
            };
        }

        if let Some(env_keys) = find_env_keys(provider)
            && let Some(first) = env_keys.first()
        {
            return AuthStatus {
                configured: false,
                source: Some("environment"),
                label: Some((*first).to_owned()),
            };
        }

        AuthStatus::default()
    }

    /// Spec: `getAll()`.
    pub fn get_all(&self) -> AuthStorageData {
        self.data.clone()
    }

    /// Spec: `drainErrors()`.
    pub fn drain_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    /// Spec: `login(providerId, callbacks)`.
    pub async fn login(
        &mut self,
        provider_id: &str,
        callbacks: &dyn OAuthLoginCallbacks,
    ) -> Result<(), AuthStorageError> {
        let provider = get_oauth_provider(provider_id).ok_or_else(|| {
            AuthStorageError::Message(format!("Unknown OAuth provider: {provider_id}"))
        })?;
        let credentials = provider.login(callbacks).await?;
        self.set(provider_id, AuthCredential::OAuth(credentials));
        Ok(())
    }

    /// Spec: `logout(provider)`.
    pub fn logout(&mut self, provider: &str) {
        self.remove(provider);
    }

    /// Spec: `refreshOAuthTokenWithLock` — refresh under the backend
    /// lock so concurrent instances don't race.
    async fn refresh_oauth_token_with_lock(
        &mut self,
        provider_id: &str,
    ) -> Result<Option<String>, AuthStorageError> {
        let Some(provider) = get_oauth_provider(provider_id) else {
            return Ok(None);
        };

        let provider_id_owned = provider_id.to_owned();
        let (api_key, data) = self
            .storage
            .with_lock_async(move |current| async move {
                let current_data = parse_storage_data(current.as_deref())?;

                let Some(AuthCredential::OAuth(cred)) = current_data.get(&provider_id_owned) else {
                    return Ok(((None, current_data), None));
                };

                if pi_rs_ai_types::now_ms() < cred.expires {
                    let api_key = provider.get_api_key(cred);
                    return Ok(((Some(api_key), current_data), None));
                }

                let mut oauth_creds: HashMap<String, OAuthCredentials> = HashMap::new();
                for (key, value) in &current_data {
                    if let AuthCredential::OAuth(c) = value {
                        oauth_creds.insert(key.clone(), c.clone());
                    }
                }

                let Some(refreshed) = get_oauth_api_key(&provider_id_owned, &oauth_creds).await?
                else {
                    return Ok(((None, current_data), None));
                };

                let mut merged = current_data;
                merged.insert(
                    provider_id_owned.clone(),
                    AuthCredential::OAuth(refreshed.new_credentials.clone()),
                );
                let next = serialize_storage_data(&merged);
                Ok(((Some(refreshed.api_key), merged), Some(next)))
            })
            .await?;

        // Spec: the closure sets `this.data`/`this.loadError` as it goes.
        self.data = data;
        self.load_error = None;
        Ok(api_key)
    }

    /// Spec: `getApiKey(providerId, options?)` — priority: runtime
    /// override → api_key from auth.json → OAuth token (auto-refreshed
    /// with locking) → environment variable → fallback resolver (WS7).
    pub async fn get_api_key(&mut self, provider_id: &str) -> Option<String> {
        if let Some(runtime_key) = self.runtime_overrides.get(provider_id) {
            return Some(runtime_key.clone());
        }

        match self.data.get(provider_id).cloned() {
            Some(AuthCredential::ApiKey { key }) => return resolve_config_value(&key),
            Some(AuthCredential::OAuth(cred)) => {
                let provider = get_oauth_provider(provider_id)?;

                let needs_refresh = pi_rs_ai_types::now_ms() >= cred.expires;
                if needs_refresh {
                    match self.refresh_oauth_token_with_lock(provider_id).await {
                        Ok(Some(api_key)) => return Some(api_key),
                        Ok(None) => {}
                        Err(error) => {
                            self.record_error(error);
                            // Refresh failed — re-read in case another
                            // instance succeeded.
                            self.reload();
                            if let Some(AuthCredential::OAuth(updated)) = self.data.get(provider_id)
                                && pi_rs_ai_types::now_ms() < updated.expires
                            {
                                return Some(provider.get_api_key(updated));
                            }
                            // Truly failed — credentials preserved for
                            // retry via /login.
                            return None;
                        }
                    }
                    return None;
                }
                return Some(provider.get_api_key(&cred));
            }
            None => {}
        }

        get_env_api_key(provider_id)
        // Fallback resolver (models.json custom providers): WS7.
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn oauth_cred(expires: i64) -> AuthCredential {
        AuthCredential::OAuth(OAuthCredentials {
            refresh: "r".into(),
            access: "a".into(),
            expires,
            extra: serde_json::Map::new(),
        })
    }

    #[test]
    fn credential_serde_shapes_match_spec() {
        let api_key = AuthCredential::ApiKey { key: "sk-1".into() };
        assert_eq!(
            serde_json::to_value(&api_key).unwrap(),
            serde_json::json!({"type": "api_key", "key": "sk-1"})
        );
        let oauth = oauth_cred(5);
        assert_eq!(
            serde_json::to_value(&oauth).unwrap(),
            serde_json::json!({"type": "oauth", "refresh": "r", "access": "a", "expires": 5})
        );
    }

    #[test]
    fn in_memory_set_get_remove_persist() {
        let mut storage = AuthStorage::in_memory(AuthStorageData::new());
        assert!(!storage.has("anthropic"));
        storage.set("anthropic", AuthCredential::ApiKey { key: "sk-2".into() });
        assert!(storage.has("anthropic"));
        assert_eq!(storage.list(), vec!["anthropic".to_owned()]);
        assert_eq!(
            storage.get_auth_status("anthropic"),
            AuthStatus {
                configured: true,
                source: Some("stored"),
                label: None
            }
        );
        storage.remove("anthropic");
        assert!(!storage.has("anthropic"));
    }

    #[test]
    fn file_backend_round_trips_with_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut storage = AuthStorage::create(Some(path.clone()));
        storage.set("anthropic", AuthCredential::ApiKey { key: "sk-3".into() });

        // A fresh instance sees the persisted credential.
        let storage2 = AuthStorage::create(Some(path.clone()));
        assert_eq!(
            storage2.get("anthropic"),
            Some(&AuthCredential::ApiKey { key: "sk-3".into() })
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn corrupt_auth_json_records_load_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, "not json").unwrap();
        let mut storage = AuthStorage::create(Some(path));
        assert!(storage.load_error.is_some());
        assert_eq!(storage.drain_errors().len(), 1);
        // Spec: persist is a no-op while loadError is set.
        storage.set("x", AuthCredential::ApiKey { key: "k".into() });
    }

    #[tokio::test]
    async fn get_api_key_priority() {
        let mut storage = AuthStorage::in_memory(AuthStorageData::new());
        storage.set(
            "anthropic",
            AuthCredential::ApiKey {
                key: "stored".into(),
            },
        );
        storage.set_runtime_api_key("anthropic", "runtime");
        assert_eq!(
            storage.get_api_key("anthropic").await,
            Some("runtime".to_owned())
        );
        storage.remove_runtime_api_key("anthropic");
        assert_eq!(
            storage.get_api_key("anthropic").await,
            Some("stored".to_owned())
        );
    }

    #[tokio::test]
    async fn unexpired_oauth_returns_access_token() {
        let mut storage = AuthStorage::in_memory(AuthStorageData::new());
        storage.set("anthropic", oauth_cred(pi_rs_ai_types::now_ms() + 60_000));
        // The anthropic flow's getApiKey returns the access token.
        assert_eq!(storage.get_api_key("anthropic").await, Some("a".to_owned()));
    }
}
