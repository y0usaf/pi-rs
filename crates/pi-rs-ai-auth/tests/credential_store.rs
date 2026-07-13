//! XDG credential storage, fallback provenance, locking, and redaction invariants.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pi_rs_ai_auth::{
    AuthCredential, AuthError, AuthFuture, CredentialPaths, CredentialSource, CredentialStore,
    OAuthCredentials, OAuthLoginCallbacks, OAuthProviderInterface, register_oauth_provider,
    unregister_oauth_provider,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pi-rs-ai-auth-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store(root: &TempRoot) -> (CredentialStore, PathBuf, PathBuf) {
    let canonical = root.0.join("xdg-state/pi/credentials.json");
    let legacy = root.0.join("home/.pi/agent/auth.json");
    let paths = CredentialPaths::new(canonical.clone(), Some(legacy.clone())).unwrap();
    (CredentialStore::new(paths), canonical, legacy)
}

#[tokio::test]
async fn legacy_read_then_mutation_writes_only_canonical() {
    let root = TempRoot::new();
    let (store, canonical, legacy) = store(&root);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let legacy_bytes = br#"{
  "anthropic": {"type":"api_key","key":"legacy-secret-value"}
}"#;
    std::fs::write(&legacy, legacy_bytes).unwrap();
    let before = std::fs::metadata(&legacy).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.source, CredentialSource::Legacy);
    assert_eq!(snapshot.providers, ["anthropic"]);
    let resolved = store
        .resolve_stored_api_key("anthropic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.expose_secret(), "legacy-secret-value");

    store
        .set(
            "openai",
            AuthCredential::ApiKey {
                key: "canonical-secret-value".into(),
            },
        )
        .await
        .unwrap();

    assert!(canonical.exists());
    assert_eq!(std::fs::read(&legacy).unwrap(), legacy_bytes);
    let after = std::fs::metadata(&legacy).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(
        store.snapshot().unwrap().source,
        CredentialSource::Canonical
    );
    assert_eq!(store.snapshot().unwrap().providers, ["anthropic", "openai"]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(canonical).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn present_malformed_canonical_fails_closed_without_legacy_fallthrough() {
    let root = TempRoot::new();
    let (store, canonical, legacy) = store(&root);
    std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&canonical, "not json").unwrap();
    std::fs::write(
        &legacy,
        r#"{"anthropic":{"type":"api_key","key":"legacy-secret-value"}}"#,
    )
    .unwrap();

    let error = store.snapshot().unwrap_err();
    assert!(error.to_string().starts_with("json error:"));
    assert!(!error.to_string().contains("legacy-secret-value"));
}

#[tokio::test]
async fn concurrent_writers_merge_and_snapshots_and_debug_are_secret_free() {
    let root = TempRoot::new();
    let (store, _, _) = store(&root);
    let first = store.clone();
    let second = store.clone();
    let (left, right) = tokio::join!(
        first.set(
            "anthropic",
            AuthCredential::ApiKey {
                key: "first-secret-value".into(),
            }
        ),
        second.set(
            "openai",
            AuthCredential::ApiKey {
                key: "second-secret-value".into(),
            }
        )
    );
    left.unwrap();
    right.unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.providers, ["anthropic", "openai"]);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    let credential = store.get("anthropic").unwrap().unwrap();
    let debug = format!("{credential:?}");
    for secret in ["first-secret-value", "second-secret-value"] {
        assert!(!serialized.contains(secret));
        assert!(!debug.contains(secret));
    }
    assert!(debug.contains("[REDACTED]"));
}

struct RefreshProvider;

impl OAuthProviderInterface for RefreshProvider {
    fn id(&self) -> &str {
        "refresh-fixture"
    }

    fn name(&self) -> &str {
        "Refresh Fixture"
    }

    fn login<'a>(
        &'a self,
        _callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(async { Err(AuthError::Cancelled) })
    }

    fn refresh_token<'a>(
        &'a self,
        _credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(async {
            Ok(OAuthCredentials {
                refresh: "new-refresh-secret".into(),
                access: "new-access-secret".into(),
                expires: i64::MAX,
                extra: serde_json::Map::new(),
            })
        })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }
}

#[tokio::test]
async fn expired_oauth_refresh_is_locked_and_persisted_canonically() {
    let root = TempRoot::new();
    let (store, canonical, legacy) = store(&root);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(
        &legacy,
        r#"{"refresh-fixture":{"type":"oauth","refresh":"old-refresh-secret","access":"old-access-secret","expires":0}}"#,
    )
    .unwrap();
    let legacy_before = std::fs::read(&legacy).unwrap();
    register_oauth_provider(Arc::new(RefreshProvider));

    let resolved = store
        .resolve_stored_api_key("refresh-fixture")
        .await
        .unwrap()
        .unwrap();
    assert!(resolved.refreshed);
    assert_eq!(resolved.expose_secret(), "new-access-secret");
    assert!(canonical.exists());
    assert_eq!(std::fs::read(&legacy).unwrap(), legacy_before);
    let canonical_text = std::fs::read_to_string(canonical).unwrap();
    assert!(canonical_text.contains("new-refresh-secret"));
    assert!(!canonical_text.contains("old-refresh-secret"));
    assert!(!format!("{resolved:?}").contains("new-access-secret"));

    unregister_oauth_provider("refresh-fixture");
}

#[test]
fn paths_must_be_explicit_absolute_and_distinct() {
    assert!(CredentialPaths::new("relative".into(), None).is_err());
    let absolute = std::env::temp_dir().join("credentials.json");
    assert!(CredentialPaths::new(absolute.clone(), Some("relative".into())).is_err());
    assert!(CredentialPaths::new(absolute.clone(), Some(absolute)).is_err());
}
