//! `core/auth-storage.ts`: the credentials pi keeps in `<agentDir>/auth.json`, one per provider:
//! `{"type": "api_key", "key": ..., "env": {...}}` or `{"type": "oauth", "access", "refresh",
//! "expires", ...}`.
//!
//! Print mode writes the file only as upstream does: it creates an empty `{}` when it is missing
//! (mode 0600), and it stores a refreshed OAuth token when the stored one is within five minutes
//! of expiring, under the same `auth.json.lock` directory lock that `proper-lockfile` takes, so a
//! running TypeScript pi and this one never refresh the same token twice.

use crate::config_value::{self, Env};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};

pub const OAUTH_MINIMUM_VALIDITY_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq)]
pub enum Credential {
    ApiKey {
        key: Option<String>,
        env: Option<Env>,
    },
    /// The stored object, holding at least `access`, `refresh` and `expires`.
    OAuth(Map<String, Value>),
}

fn parse_credential(value: &Value) -> Option<Credential> {
    let obj = value.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("api_key") => {
            let key = match obj.get("key") {
                None => None,
                Some(Value::String(k)) => Some(k.clone()),
                Some(_) => return None,
            };
            let env = match obj.get("env") {
                None => None,
                Some(Value::Object(map)) => {
                    let mut env = Env::new();
                    for (k, v) in map {
                        env.insert(k.clone(), v.as_str()?.to_owned());
                    }
                    Some(env)
                }
                Some(_) => return None,
            };
            Some(Credential::ApiKey { key, env })
        }
        Some("oauth")
            if obj.get("access").is_some_and(Value::is_string)
                && obj.get("refresh").is_some_and(Value::is_string)
                && obj
                    .get("expires")
                    .and_then(Value::as_f64)
                    .is_some_and(f64::is_finite) =>
        {
            Some(Credential::OAuth(obj.clone()))
        }
        _ => None,
    }
}

/// Exchanges a stored OAuth credential for a fresh one.
pub type RefreshFn<'a> = dyn Fn(&Map<String, Value>) -> Result<Map<String, Value>, String> + 'a;

pub struct AuthStore {
    pub path: String,
    data: Map<String, Value>,
}

fn now_ms() -> i64 {
    pi_durable::ai::oauth::now_ms()
}

/// Creates `{}` at `path` (mode 0600, parent 0700) when nothing is there, as upstream's file
/// backend does before every access.
pub fn ensure_file(path: &str) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = crate::paths::dirname(path);
    if !std::path::Path::new(&dir).exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| crate::node_fs::message(&e, "mkdir", &dir))?;
    }
    if !std::path::Path::new(path).exists() {
        write_private(path, "{}")?;
    }
    Ok(())
}

/// `proper-lockfile`'s lock: the directory `<path>.lock`, stale after 10 s without an update.
pub struct FileLock {
    dir: String,
}

impl FileLock {
    pub fn acquire(path: &str, wait: Duration) -> Result<Self, String> {
        let dir = format!("{path}.lock");
        let deadline = Instant::now() + wait;
        loop {
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self { dir }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&dir)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(10));
                    if stale {
                        let _ = std::fs::remove_dir(&dir);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err("Lock file is already being held".to_owned());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(crate::node_fs::message(&e, "mkdir", &dir)),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.dir);
    }
}

fn read_map(path: &str) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => {
            return Err(format!(
                "Failed to read auth.json: {}",
                crate::node_fs::message(&e, "open", path)
            ));
        }
    };
    if text.is_empty() {
        return Ok(Map::new());
    }
    match crate::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Ok(Map::new()),
        Err(e) => Err(format!("Failed to read auth.json: {e}")),
    }
}

fn write_private(path: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| crate::node_fs::message(&e, "open", path))?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

impl AuthStore {
    /// `AuthStorage.create(<agentDir>/auth.json)`: makes the file when missing, then reads it. A
    /// file that does not parse reads as empty, as upstream's `reload` swallows the error.
    pub fn open(path: &str) -> Self {
        let _ = ensure_file(path);
        let data = read_map(path).unwrap_or_default();
        Self {
            path: path.to_owned(),
            data,
        }
    }

    pub fn has(&self, provider: &str) -> bool {
        self.data.contains_key(provider)
    }

    /// The stored credential with an API key's value resolved (`$VAR`, `!command`).
    pub fn read(&self, provider: &str) -> Option<Credential> {
        match parse_credential(self.data.get(provider)?)? {
            Credential::ApiKey {
                key: Some(key),
                env,
            } => {
                let resolved = config_value::resolve(&key, env.as_ref());
                Some(Credential::ApiKey { key: resolved, env })
            }
            other => Some(other),
        }
    }

    /// The stored OAuth credential, refreshed with `refresh` first when it expires within five
    /// minutes. The refresh runs under the file lock and re-reads the file there, so a token
    /// another process refreshed meanwhile is used as it is. None when the provider has no OAuth
    /// credential.
    pub fn fresh_oauth(
        &mut self,
        provider: &str,
        refresh: &RefreshFn<'_>,
    ) -> Result<Option<Map<String, Value>>, String> {
        let expires_soon = |c: &Map<String, Value>| {
            c.get("expires")
                .and_then(Value::as_f64)
                .is_none_or(|e| (now_ms() + OAUTH_MINIMUM_VALIDITY_MS) as f64 >= e)
        };
        let Some(Credential::OAuth(stored)) = self.read(provider) else {
            return Ok(None);
        };
        if !expires_soon(&stored) {
            return Ok(Some(stored));
        }
        ensure_file(&self.path)?;
        let lock = FileLock::acquire(&self.path, Duration::from_secs(30))
            .map_err(|e| format!("Credential store modify failed for {provider}: {e}"))?;
        let mut current = read_map(&self.path)?;
        let result = match current.get(provider).and_then(parse_credential) {
            Some(Credential::OAuth(c)) if !expires_soon(&c) => Some(c),
            Some(Credential::OAuth(c)) => {
                let next =
                    refresh(&c).map_err(|e| format!("OAuth refresh failed for {provider}: {e}"))?;
                let mut merged = Map::new();
                merged.insert("type".to_owned(), Value::String("oauth".to_owned()));
                for (k, v) in next {
                    merged.insert(k, v);
                }
                current.insert(provider.to_owned(), Value::Object(merged.clone()));
                let text = serde_json::to_string_pretty(&Value::Object(current.clone()))
                    .map_err(|e| e.to_string())?;
                write_private(&self.path, &text)?;
                Some(merged)
            }
            _ => None,
        };
        drop(lock);
        self.data = current;
        Ok(result)
    }
}

impl AuthStore {
    /// `AuthStorage.modify`: stores one provider's credential and keeps the rest as they are on
    /// disk, under the same lock a refresh takes.
    pub fn write(&mut self, provider: &str, credential: Value) -> Result<(), String> {
        self.update(|data| {
            data.insert(provider.to_owned(), credential);
        })
    }

    /// `AuthStorage.delete`.
    pub fn remove(&mut self, provider: &str) -> Result<(), String> {
        self.update(|data| {
            data.remove(provider);
        })
    }

    /// `AuthStorage.list`: the providers with a stored credential.
    pub fn providers(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    fn update(&mut self, change: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
        ensure_file(&self.path)?;
        let lock = FileLock::acquire(&self.path, Duration::from_secs(30))?;
        let mut current = read_map(&self.path)?;
        change(&mut current);
        let text = serde_json::to_string_pretty(&Value::Object(current.clone()))
            .map_err(|e| e.to_string())?;
        write_private(&self.path, &text)?;
        drop(lock);
        self.data = current;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_under_lock_writes_once() {
        let dir = std::env::temp_dir().join(format!("pi-rs-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json").to_string_lossy().into_owned();
        let initial = serde_json::json!({
            "p": {"type": "oauth", "access": "old", "refresh": "r", "expires": 0},
            "k": {"type": "api_key", "key": "lit"}
        });
        std::fs::write(&path, initial.to_string()).unwrap();
        let mut store = AuthStore::open(&path);
        assert_eq!(
            store.read("k"),
            Some(Credential::ApiKey {
                key: Some("lit".into()),
                env: None
            })
        );
        let calls = std::cell::Cell::new(0);
        let refresh = |c: &Map<String, Value>| {
            calls.set(calls.get() + 1);
            let mut next = Map::new();
            next.insert("access".into(), "new".into());
            next.insert("refresh".into(), c["refresh"].clone());
            next.insert("expires".into(), (now_ms() + 3_600_000).into());
            Ok(next)
        };
        let fresh = store.fresh_oauth("p", &refresh).unwrap().unwrap();
        assert_eq!(fresh["access"], "new");
        let again = store.fresh_oauth("p", &refresh).unwrap().unwrap();
        assert_eq!(again["access"], "new");
        assert_eq!(calls.get(), 1);
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk["p"]["access"], "new");
        assert_eq!(on_disk["k"]["key"], "lit");
        assert!(!std::path::Path::new(&format!("{path}.lock")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
