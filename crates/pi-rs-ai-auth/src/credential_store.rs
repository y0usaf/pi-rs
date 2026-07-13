//! Explicit-path credential storage with XDG-only writes and read-only legacy fallback.
//!
//! The application path resolver supplies both locations. This crate never
//! discovers `$HOME`, XDG variables, or product roots itself. Canonical presence
//! wins even when unreadable or malformed; legacy data is considered only when
//! the canonical path is absent. Every mutation is lock-serialized and atomically
//! replaces the canonical file without touching the selected legacy file.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::engine::now_ms;
use crate::error::AuthError;
use crate::registry::get_oauth_provider;
use crate::types::OAuthCredentials;

const LOCK_ATTEMPTS: usize = 100;
const LOCK_DELAY: Duration = Duration::from_millis(20);
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Explicit locations supplied by the XDG startup boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialPaths {
    canonical: PathBuf,
    legacy: Option<PathBuf>,
}

impl CredentialPaths {
    pub fn new(canonical: PathBuf, legacy: Option<PathBuf>) -> Result<Self, AuthError> {
        if !canonical.is_absolute() {
            return Err(AuthError::Message(
                "canonical credential path must be absolute".into(),
            ));
        }
        if legacy.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err(AuthError::Message(
                "legacy credential path must be absolute".into(),
            ));
        }
        if legacy.as_ref() == Some(&canonical) {
            return Err(AuthError::Message(
                "canonical and legacy credential paths must differ".into(),
            ));
        }
        Ok(Self { canonical, legacy })
    }

    #[must_use]
    pub fn canonical(&self) -> &Path {
        &self.canonical
    }

    #[must_use]
    pub fn legacy(&self) -> Option<&Path> {
        self.legacy.as_deref()
    }
}

/// Stored `credentials.json` / legacy `auth.json` row.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthCredential {
    ApiKey {
        key: String,
    },
    #[serde(rename = "oauth")]
    OAuth {
        refresh: String,
        access: String,
        expires: i64,
        #[serde(flatten)]
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

impl std::fmt::Debug for AuthCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey { .. } => formatter
                .debug_struct("ApiKey")
                .field("key", &pi_rs_ai_types::REDACTED)
                .finish(),
            Self::OAuth { expires, extra, .. } => formatter
                .debug_struct("OAuth")
                .field("refresh", &pi_rs_ai_types::REDACTED)
                .field("access", &pi_rs_ai_types::REDACTED)
                .field("expires", expires)
                .field("extra_fields", &extra.keys().collect::<Vec<_>>())
                .finish(),
        }
    }
}

impl AuthCredential {
    fn oauth_credentials(&self) -> Option<OAuthCredentials> {
        let Self::OAuth {
            refresh,
            access,
            expires,
            extra,
        } = self
        else {
            return None;
        };
        Some(OAuthCredentials {
            refresh: refresh.clone(),
            access: access.clone(),
            expires: *expires,
            extra: extra.clone(),
        })
    }

    fn from_oauth(credentials: OAuthCredentials) -> Self {
        Self::OAuth {
            refresh: credentials.refresh,
            access: credentials.access,
            expires: credentials.expires,
            extra: credentials.extra,
        }
    }
}

/// Provenance of the currently selected credential file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    Canonical,
    Legacy,
    Absent,
}

/// Secret-free credential state suitable for snapshots and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CredentialSnapshot {
    pub source: CredentialSource,
    pub providers: Vec<String>,
}

/// A resolved stored key. Debug output intentionally excludes the key.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredApiKey {
    api_key: String,
    pub refreshed: bool,
}

impl StoredApiKey {
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.api_key
    }
}

impl std::fmt::Debug for StoredApiKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredApiKey")
            .field("api_key", &pi_rs_ai_types::REDACTED)
            .field("refreshed", &self.refreshed)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct CredentialStore {
    paths: CredentialPaths,
}

impl CredentialStore {
    #[must_use]
    pub fn new(paths: CredentialPaths) -> Self {
        Self { paths }
    }

    pub fn snapshot(&self) -> Result<CredentialSnapshot, AuthError> {
        let loaded = self.load()?;
        Ok(CredentialSnapshot {
            source: loaded.source,
            providers: loaded.data.keys().cloned().collect(),
        })
    }

    pub fn get(&self, provider: &str) -> Result<Option<AuthCredential>, AuthError> {
        Ok(self.load()?.data.get(provider).cloned())
    }

    pub async fn set(&self, provider: &str, credential: AuthCredential) -> Result<(), AuthError> {
        validate_provider(provider)?;
        let _lock = self.acquire_lock().await?;
        let mut loaded = self.load()?;
        loaded.data.insert(provider.to_owned(), credential);
        self.write_canonical(&loaded.data)
    }

    pub async fn remove(&self, provider: &str) -> Result<(), AuthError> {
        validate_provider(provider)?;
        let _lock = self.acquire_lock().await?;
        let mut loaded = self.load()?;
        loaded.data.remove(provider);
        self.write_canonical(&loaded.data)
    }

    /// Resolve a stored API key, refreshing expired OAuth credentials under the
    /// same inter-process lock used for the canonical write.
    pub async fn resolve_stored_api_key(
        &self,
        provider_id: &str,
    ) -> Result<Option<StoredApiKey>, AuthError> {
        validate_provider(provider_id)?;
        let _lock = self.acquire_lock().await?;
        let mut loaded = self.load()?;
        let Some(credential) = loaded.data.get(provider_id).cloned() else {
            return Ok(None);
        };
        match credential {
            AuthCredential::ApiKey { key } => Ok(crate::config_value::resolve_config_value(&key)
                .await
                .map(|api_key| StoredApiKey {
                    api_key,
                    refreshed: false,
                })),
            oauth @ AuthCredential::OAuth { expires, .. } => {
                let provider = get_oauth_provider(provider_id).ok_or_else(|| {
                    AuthError::Message(format!("Unknown OAuth provider: {provider_id}"))
                })?;
                let mut credentials = oauth
                    .oauth_credentials()
                    .ok_or_else(|| AuthError::Message("invalid OAuth credential row".into()))?;
                let refreshed = now_ms() >= expires;
                if refreshed {
                    credentials = provider.refresh_token(&credentials).await.map_err(|_| {
                        AuthError::Message(format!(
                            "Failed to refresh OAuth token for {provider_id}"
                        ))
                    })?;
                    loaded.data.insert(
                        provider_id.to_owned(),
                        AuthCredential::from_oauth(credentials.clone()),
                    );
                    self.write_canonical(&loaded.data)?;
                }
                Ok(Some(StoredApiKey {
                    api_key: provider.get_api_key(&credentials),
                    refreshed,
                }))
            }
        }
    }

    fn load(&self) -> Result<LoadedCredentials, AuthError> {
        match fs::symlink_metadata(&self.paths.canonical) {
            Ok(_) => read_credentials(&self.paths.canonical, CredentialSource::Canonical),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(legacy) = &self.paths.legacy {
                    match fs::symlink_metadata(legacy) {
                        Ok(_) => read_credentials(legacy, CredentialSource::Legacy),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            Ok(LoadedCredentials::absent())
                        }
                        Err(error) => Err(error.into()),
                    }
                } else {
                    Ok(LoadedCredentials::absent())
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn acquire_lock(&self) -> Result<CredentialLock, AuthError> {
        let parent =
            self.paths.canonical.parent().ok_or_else(|| {
                AuthError::Message("canonical credential path has no parent".into())
            })?;
        create_private_directory(parent)?;
        let lock_path = lock_path(&self.paths.canonical);
        for _ in 0..LOCK_ATTEMPTS {
            match create_private_file(&lock_path, true) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())?;
                    return Ok(CredentialLock { path: lock_path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    tokio::time::sleep(LOCK_DELAY).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(AuthError::Message(
            "timed out acquiring credential storage lock".into(),
        ))
    }

    fn write_canonical(
        &self,
        credentials: &BTreeMap<String, AuthCredential>,
    ) -> Result<(), AuthError> {
        let parent =
            self.paths.canonical.parent().ok_or_else(|| {
                AuthError::Message("canonical credential path has no parent".into())
            })?;
        create_private_directory(parent)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".credentials.json.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let bytes = serde_json::to_vec_pretty(credentials)?;
        let result = (|| -> Result<(), AuthError> {
            let mut file = create_private_file(&temp, true)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.paths.canonical)?;
            sync_directory(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

struct LoadedCredentials {
    source: CredentialSource,
    data: BTreeMap<String, AuthCredential>,
}

impl LoadedCredentials {
    fn absent() -> Self {
        Self {
            source: CredentialSource::Absent,
            data: BTreeMap::new(),
        }
    }
}

fn read_credentials(path: &Path, source: CredentialSource) -> Result<LoadedCredentials, AuthError> {
    let bytes = fs::read(path)?;
    let data = serde_json::from_slice(&bytes)?;
    Ok(LoadedCredentials { source, data })
}

fn validate_provider(provider: &str) -> Result<(), AuthError> {
    if provider.is_empty() || provider.chars().any(char::is_whitespace) {
        return Err(AuthError::Message("invalid credential provider id".into()));
    }
    Ok(())
}

fn lock_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".lock");
    PathBuf::from(value)
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn create_private_file(path: &Path, create_new: bool) -> Result<fs::File, std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(!create_new)
        .create_new(create_new)
        .truncate(!create_new)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path, create_new: bool) -> Result<fs::File, std::io::Error> {
    OpenOptions::new()
        .write(true)
        .create(!create_new)
        .create_new(create_new)
        .truncate(!create_new)
        .open(path)
}

fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    fs::File::open(path)?.sync_all()
}

struct CredentialLock {
    path: PathBuf,
}

impl Drop for CredentialLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
