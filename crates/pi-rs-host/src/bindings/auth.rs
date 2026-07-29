//! Versioned credential storage and subscription-provider inventory.
//!
//! Rust owns only the storage mechanics the provider/auth subsystem already
//! implements: canonical-first selection with a read-only legacy fallback,
//! inter-process locking, atomic replacement, private file modes, stored-value
//! expansion, and OAuth refresh. Both file locations are supplied by Lua, so no
//! resource path, precedence rule, or product credential name exists here.
//!
//! Secrets leave Rust through exactly one member (`resolve`). `snapshot` and
//! `describe` report provenance, presence, kind, and expiry only, so a package
//! can render credential state without ever holding a secret.
//!
//! The store handle owns no operating-system resource between calls: each
//! operation takes the canonical lock, completes, and releases it, so there is
//! nothing to register as a scope resource and nothing to leak at disposal.

use std::future::Future;
use std::path::PathBuf;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};
use pi_rs_ai_auth::{
    AuthCredential, AuthError, CredentialPaths, CredentialSource, CredentialStore, OAuthCredentials,
};

/// Auth API version. Independent of the stored credential file's own shape.
const AUTH_API_VERSION: u32 = 1;
/// Largest secret this module will store or hand back.
const MAX_SECRET_BYTES: usize = 64 * 1024;
/// Largest provider list a snapshot will materialise.
const MAX_PROVIDERS: usize = 256;

fn auth_error(error: AuthError) -> mlua::Error {
    mlua::Error::runtime(error.to_string())
}

fn source_name(source: CredentialSource) -> &'static str {
    match source {
        CredentialSource::Canonical => "canonical",
        CredentialSource::Legacy => "legacy",
        CredentialSource::Absent => "absent",
    }
}

fn check_secret(label: &str, value: &str) -> mlua::Result<()> {
    if value.len() > MAX_SECRET_BYTES {
        return Err(mlua::Error::runtime(format!(
            "auth.v1 {label} exceeds {MAX_SECRET_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Read an OAuth row authored in Lua. `refresh`, `access`, and `expires` are
/// required; every other field is preserved verbatim as provider-defined extra
/// data, so a provider that carries more state needs no Rust change.
fn oauth_credential(value: Table) -> mlua::Result<AuthCredential> {
    let json = crate::convert::lua_to_json(Value::Table(value))?;
    let credentials: OAuthCredentials = serde_json::from_value(json)
        .map_err(|error| mlua::Error::runtime(format!("invalid oauth credential: {error}")))?;
    check_secret("oauth refresh token", &credentials.refresh)?;
    check_secret("oauth access token", &credentials.access)?;
    Ok(AuthCredential::OAuth {
        refresh: credentials.refresh,
        access: credentials.access,
        expires: credentials.expires,
        extra: credentials.extra,
    })
}

/// Run one credential operation under the innermost dispatch cancellation.
///
/// Every await inside the store is a lock retry, a stored-value command, or a
/// token refresh, so dropping the future at any of them releases the lock file
/// descriptor without leaving a half-written canonical file: the canonical
/// write itself is synchronous and atomic.
async fn under_cancellation<T>(
    cancellation: Option<crate::kernel::CancellationToken>,
    operation: impl Future<Output = Result<T, AuthError>>,
) -> mlua::Result<T> {
    let Some(token) = cancellation else {
        return operation.await.map_err(auth_error);
    };
    tokio::select! {
        biased;
        () = token.cancelled() => Err(mlua::Error::runtime("auth operation cancelled")),
        result = operation => result.map_err(auth_error),
    }
}

struct LuaCredentialStore {
    store: CredentialStore,
}

impl UserData for LuaCredentialStore {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Provenance and presence, never secrets: which file the selection
        // rule chose and which providers it holds.
        methods.add_method("snapshot", |lua, this, ()| {
            let snapshot = this.store.snapshot().map_err(auth_error)?;
            if snapshot.providers.len() > MAX_PROVIDERS {
                return Err(mlua::Error::runtime(format!(
                    "credential store holds more than {MAX_PROVIDERS} providers"
                )));
            }
            let providers = lua.create_table_with_capacity(snapshot.providers.len(), 0)?;
            for provider in &snapshot.providers {
                providers.push(provider.as_str())?;
            }
            let result = lua.create_table()?;
            result.set("source", source_name(snapshot.source))?;
            result.set("providers", providers)?;
            Ok(result)
        });

        // Kind and expiry of one row, so a package can decide whether to log in
        // again without resolving — and therefore without touching a secret.
        methods.add_method("describe", |lua, this, provider: String| {
            let Some(credential) = this.store.get(&provider).map_err(auth_error)? else {
                return Ok(Value::Nil);
            };
            let row = lua.create_table()?;
            match credential {
                AuthCredential::ApiKey { .. } => row.set("kind", "api_key")?,
                AuthCredential::OAuth { expires, extra, .. } => {
                    row.set("kind", "oauth")?;
                    row.set("expires", expires)?;
                    row.set("expired", pi_rs_ai_types::now_ms() >= expires)?;
                    let fields = lua.create_table_with_capacity(extra.len(), 0)?;
                    for key in extra.keys() {
                        fields.push(key.as_str())?;
                    }
                    row.set("extra_fields", fields)?;
                }
            }
            Ok(Value::Table(row))
        });

        methods.add_async_method(
            "set_api_key",
            |lua, this, (provider, value): (String, String)| async move {
                check_secret("api key", &value)?;
                let cancellation = crate::kernel_api::current_cancellation(&lua)?;
                under_cancellation(
                    cancellation,
                    this.store
                        .set(&provider, AuthCredential::ApiKey { key: value }),
                )
                .await
            },
        );

        methods.add_async_method(
            "set_oauth",
            |lua, this, (provider, credentials): (String, Table)| async move {
                let credential = oauth_credential(credentials)?;
                let cancellation = crate::kernel_api::current_cancellation(&lua)?;
                under_cancellation(cancellation, this.store.set(&provider, credential)).await
            },
        );

        methods.add_async_method("remove", |lua, this, provider: String| async move {
            let cancellation = crate::kernel_api::current_cancellation(&lua)?;
            under_cancellation(cancellation, this.store.remove(&provider)).await
        });

        // The one member that yields a secret. A stored api-key row is expanded
        // (`$NAME` from the process environment, `!command` through the shell
        // with its own hard timeout); an expired OAuth row is refreshed and
        // written back under the same lock, and `refreshed` reports which
        // happened.
        methods.add_async_method("resolve", |lua, this, provider: String| async move {
            let cancellation = crate::kernel_api::current_cancellation(&lua)?;
            let resolved =
                under_cancellation(cancellation, this.store.resolve_stored_api_key(&provider))
                    .await?;
            let Some(resolved) = resolved else {
                return Ok(Value::Nil);
            };
            check_secret("resolved api key", resolved.expose_secret())?;
            let row = lua.create_table()?;
            row.set("api_key", resolved.expose_secret())?;
            row.set("refreshed", resolved.refreshed)?;
            Ok(Value::Table(row))
        });
    }
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let v1 = lua.create_table()?;
    v1.set("api_version", AUTH_API_VERSION)?;
    v1.set("max_secret_bytes", MAX_SECRET_BYTES)?;
    v1.set("max_providers", MAX_PROVIDERS)?;

    // Subscription inventory: which provider identities can refresh a stored
    // OAuth row. Which of them a product offers, and in what order, is policy
    // that rides `pi.kernel.v1.declare` like every other declaration.
    v1.set(
        "providers",
        lua.create_function(|lua, ()| {
            let providers = pi_rs_ai_auth::get_oauth_providers();
            let result = lua.create_table_with_capacity(providers.len(), 0)?;
            for provider in providers {
                let row = lua.create_table()?;
                row.set("id", provider.id())?;
                row.set("name", provider.name())?;
                row.set("uses_callback_server", provider.uses_callback_server())?;
                result.push(row)?;
            }
            Ok(result)
        })?,
    )?;

    // Both locations are explicit. Rust refuses only what it cannot implement:
    // a relative path, or a legacy fallback equal to the canonical file.
    v1.set(
        "store",
        lua.create_function(|_, options: Table| {
            let canonical: String = options.get("canonical")?;
            let legacy: Option<String> = options.get("legacy")?;
            let paths = CredentialPaths::new(PathBuf::from(canonical), legacy.map(PathBuf::from))
                .map_err(auth_error)?;
            Ok(LuaCredentialStore {
                store: CredentialStore::new(paths),
            })
        })?,
    )?;

    let auth = lua.create_table()?;
    auth.set("v1", v1)?;
    pi.set("auth", auth)
}
