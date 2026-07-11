//! `pi.auth` — credential storage and OAuth login exposed to Lua.
//!
//! Mechanism only (DESIGN divergence 2): the [`crate::auth_storage`]
//! port of `core/auth-storage.ts` plus a channel bridge that runs an
//! OAuth login flow (`utils/oauth/`) concurrently with the Lua frontend.
//! All policy — selectors, dialog presentation, message strings, when to
//! login or logout — lives in the interactive Lua pack.
//!
//! Login seam: `pi.auth.login_start(provider)` spawns the provider's
//! login future on the VM runtime and returns a handle. The flow's
//! `OAuthLoginCallbacks` become an event stream the Lua side drains
//! (`handle:next_event(timeout_ms)`); prompts suspend the flow until
//! `handle:respond(text)`; `handle:cancel()` rejects a pending prompt
//! with the spec's "Login cancelled" (the string
//! `interactive-mode.ts` compares against). On success the credential
//! persists through the same storage instance as the sync bindings —
//! the spec's `authStorage.login` = `provider.login(callbacks)` +
//! `set(providerId, { type: "oauth", ...credentials })`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mlua::{Lua, Table, UserData, UserDataMethods};
use pi_rs_ai_auth::{
    AuthFuture, OAuthAuthInfo, OAuthDeviceCodeInfo, OAuthLoginCallbacks, OAuthPrompt,
    OAuthSelectPrompt,
};
use tokio::sync::mpsc;

use crate::auth_storage::{AuthCredential, AuthStorage};
use crate::convert::{json_to_lua, lua_to_json};

/// The spec's rejection message for a cancelled login prompt
/// (`login-dialog.ts` `cancel()`); `interactive-mode.ts` string-compares
/// against it to suppress the error banner.
const LOGIN_CANCELLED: &str = "Login cancelled";

/// One [`AuthStorage`] instance per VM, shared between the `pi.auth`
/// bindings, the `pi.ai` model-registry bindings, and spawned login
/// flows. A tokio mutex so `get_api_key` (which may await an OAuth
/// refresh) can hold the guard across awaits without blocking the
/// current-thread runtime — the spec's single-threaded `AuthStorage`
/// with awaits interleaving between operations.
pub(crate) type SharedStorage = Arc<tokio::sync::Mutex<AuthStorage>>;

// ---------------------------------------------------------------------------
// Login channel bridge
// ---------------------------------------------------------------------------

/// Cancellation for a pending prompt: `cancel()` only rejects an input
/// await that is currently pending (the spec rejects `inputRejecter`
/// only when set).
struct CancelState {
    waiting: AtomicBool,
    notify: tokio::sync::Notify,
}

impl CancelState {
    fn new() -> Self {
        Self {
            waiting: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn cancel(&self) {
        if self.waiting.load(Ordering::SeqCst) {
            self.notify.notify_one();
        }
    }
}

struct ChannelCallbacks {
    events: mpsc::UnboundedSender<serde_json::Value>,
    input: tokio::sync::Mutex<mpsc::UnboundedReceiver<Option<String>>>,
    cancel: Arc<CancelState>,
}

impl ChannelCallbacks {
    fn send(&self, event: serde_json::Value) {
        let _ = self.events.send(event);
    }

    /// Await one `respond(...)` from Lua; `cancel()` while pending
    /// resolves `None` (the caller maps it per callback semantics).
    async fn await_input(&self) -> Option<Option<String>> {
        let mut rx = self.input.lock().await;
        self.cancel.waiting.store(true, Ordering::SeqCst);
        let received = tokio::select! {
            value = rx.recv() => value,
            () = self.cancel.notify.notified() => None,
        };
        self.cancel.waiting.store(false, Ordering::SeqCst);
        received
    }
}

impl OAuthLoginCallbacks for ChannelCallbacks {
    fn on_auth(&self, info: OAuthAuthInfo) {
        self.send(serde_json::json!({
            "type": "auth",
            "url": info.url,
            "instructions": info.instructions,
        }));
    }

    fn on_device_code(&self, info: OAuthDeviceCodeInfo) {
        self.send(serde_json::json!({
            "type": "deviceCode",
            "userCode": info.user_code,
            "verificationUri": info.verification_uri,
        }));
    }

    fn on_prompt(&self, prompt: OAuthPrompt) -> AuthFuture<'_, String> {
        Box::pin(async move {
            self.send(serde_json::json!({
                "type": "prompt",
                "message": prompt.message,
                "placeholder": prompt.placeholder,
            }));
            match self.await_input().await {
                Some(Some(text)) => Ok(text),
                // The spec rejects a pending prompt with "Login cancelled".
                _ => Err(pi_rs_ai_auth::AuthError::Message(LOGIN_CANCELLED.into())),
            }
        })
    }

    fn on_select(&self, prompt: OAuthSelectPrompt) -> AuthFuture<'_, Option<String>> {
        Box::pin(async move {
            self.send(serde_json::json!({
                "type": "select",
                "message": prompt.message,
                "options": prompt
                    .options
                    .iter()
                    .map(|option| serde_json::json!({ "id": option.id, "label": option.label }))
                    .collect::<Vec<_>>(),
            }));
            // The spec's selector resolves `undefined` on cancel.
            Ok(self.await_input().await.flatten())
        })
    }

    fn on_progress(&self, message: &str) {
        self.send(serde_json::json!({ "type": "progress", "message": message }));
    }

    fn on_manual_code_input(&self) -> Option<AuthFuture<'_, String>> {
        // The spec's interactive login always supplies the racing manual
        // promise; the frontend decides whether to surface the input
        // (`usesCallbackServer`).
        Some(Box::pin(async move {
            match self.await_input().await {
                Some(Some(text)) => Ok(text),
                _ => Err(pi_rs_ai_auth::AuthError::Message(LOGIN_CANCELLED.into())),
            }
        }))
    }
}

/// Lua-side handle to a running login flow.
struct LoginHandle {
    events: tokio::sync::Mutex<mpsc::UnboundedReceiver<serde_json::Value>>,
    input: mpsc::UnboundedSender<Option<String>>,
    cancel: Arc<CancelState>,
}

impl UserData for LoginHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // next_event(timeout_ms?) -> event | nil. nil timeout awaits
        // indefinitely; 0 yields to the flow task and polls; >0 awaits
        // with a deadline.
        methods.add_async_method(
            "next_event",
            |lua, this, timeout_ms: Option<u64>| async move {
                let mut rx = this.events.lock().await;
                let event = match timeout_ms {
                    None => rx.recv().await,
                    Some(0) => {
                        // Give the spawned flow a chance to progress, then poll.
                        tokio::task::yield_now().await;
                        rx.try_recv().ok()
                    }
                    Some(ms) => {
                        tokio::time::timeout(std::time::Duration::from_millis(ms), rx.recv())
                            .await
                            .ok()
                            .flatten()
                    }
                };
                match event {
                    Some(event) => Ok(Some(json_to_lua(&lua, &event)?)),
                    None => Ok(None),
                }
            },
        );
        // respond(text?) — resolve the pending prompt/select/manual-input.
        methods.add_method("respond", |_, this, text: Option<String>| {
            let _ = this.input.send(text);
            Ok(())
        });
        // cancel() — reject a pending prompt (spec: dialog `cancel()`).
        methods.add_method("cancel", |_, this, ()| {
            this.cancel.cancel();
            Ok(())
        });
    }
}

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

fn credential_to_lua(lua: &Lua, credential: &AuthCredential) -> mlua::Result<mlua::Value> {
    let json = serde_json::to_value(credential)
        .map_err(|error| mlua::Error::runtime(error.to_string()))?;
    json_to_lua(lua, &json)
}

pub(crate) fn install(lua: &Lua, pi: &Table, storage: SharedStorage) -> mlua::Result<()> {
    let auth = lua.create_table()?;

    // Spec `getOAuthProviders()` — id/name/usesCallbackServer rows.
    auth.set(
        "oauth_providers",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for provider in pi_rs_ai_auth::get_oauth_providers() {
                let row = lua.create_table()?;
                row.set("id", provider.id())?;
                row.set("name", provider.name())?;
                row.set("usesCallbackServer", provider.uses_callback_server())?;
                result.push(row)?;
            }
            Ok(result)
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "get",
        lua.create_async_function(move |lua, provider: String| {
            let st = Arc::clone(&st);
            async move {
                let guard = st.lock().await;
                match guard.get(&provider) {
                    Some(credential) => credential_to_lua(&lua, credential),
                    None => Ok(mlua::Value::Nil),
                }
            }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "get_auth_status",
        lua.create_async_function(move |lua, provider: String| {
            let st = Arc::clone(&st);
            async move {
                let status = st.lock().await.get_auth_status(&provider);
                let result = lua.create_table()?;
                result.set("configured", status.configured)?;
                result.set("source", status.source)?;
                result.set("label", status.label)?;
                Ok(result)
            }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "list",
        lua.create_async_function(move |lua, ()| {
            let st = Arc::clone(&st);
            async move {
                let result = lua.create_table()?;
                for provider in st.lock().await.list() {
                    result.push(provider)?;
                }
                Ok(result)
            }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "set",
        lua.create_async_function(move |_, (provider, credential): (String, mlua::Value)| {
            let st = Arc::clone(&st);
            async move {
                let json = lua_to_json(credential)
                    .map_err(|error| mlua::Error::runtime(error.to_string()))?;
                let credential: AuthCredential = serde_json::from_value(json).map_err(|error| {
                    mlua::Error::runtime(format!("invalid credential: {error}"))
                })?;
                st.lock().await.set(&provider, credential);
                Ok(())
            }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "remove",
        lua.create_async_function(move |_, provider: String| {
            let st = Arc::clone(&st);
            async move {
                st.lock().await.remove(&provider);
                Ok(())
            }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "has",
        lua.create_async_function(move |_, provider: String| {
            let st = Arc::clone(&st);
            async move { Ok(st.lock().await.has(&provider)) }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "has_auth",
        lua.create_async_function(move |_, provider: String| {
            let st = Arc::clone(&st);
            async move { Ok(st.lock().await.has_auth(&provider)) }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "set_runtime_api_key",
        lua.create_async_function(move |_, (provider, key): (String, String)| {
            let st = Arc::clone(&st);
            async move {
                st.lock().await.set_runtime_api_key(&provider, &key);
                Ok(())
            }
        })?,
    )?;

    let st = Arc::clone(&storage);
    auth.set(
        "remove_runtime_api_key",
        lua.create_async_function(move |_, provider: String| {
            let st = Arc::clone(&st);
            async move {
                st.lock().await.remove_runtime_api_key(&provider);
                Ok(())
            }
        })?,
    )?;

    // Spec `getApiKey(providerId)` — runtime override → stored api_key →
    // OAuth token (auto-refreshed with locking) → environment variable.
    // Used by the Lua agent seam so each request resolves fresh auth for
    // the *current* model's provider (`modelRegistry.getApiKeyForProvider`).
    let st = Arc::clone(&storage);
    auth.set(
        "get_api_key",
        lua.create_async_function(move |_, provider: String| {
            let st = Arc::clone(&st);
            async move { Ok(st.lock().await.get_api_key(&provider).await) }
        })?,
    )?;

    // Spec `getEnvApiKey(provider)` — sync, no refresh.
    auth.set(
        "env_api_key",
        lua.create_function(|_, provider: String| {
            Ok(pi_rs_ai::registry::get_env_api_key(&provider))
        })?,
    )?;

    // Spec `resolveConfigValue(value)` — `$ENV` / `!command` / literal.
    auth.set(
        "resolve_config_value",
        lua.create_function(|_, value: String| {
            Ok(crate::resolve_config_value::resolve_config_value(&value))
        })?,
    )?;

    // Spec `getAuthPath()` — for the "Credentials saved to ..." status.
    auth.set(
        "auth_path",
        lua.create_function(|_, ()| {
            Ok(crate::auth_storage::get_auth_path()
                .to_string_lossy()
                .into_owned())
        })?,
    )?;

    // Spec `authStorage.login(providerId, callbacks)` as a spawned flow
    // plus an event-stream handle.
    let st = Arc::clone(&storage);
    auth.set(
        "login_start",
        lua.create_function(move |lua, provider_id: String| {
            let (event_tx, event_rx) = mpsc::unbounded_channel::<serde_json::Value>();
            let (input_tx, input_rx) = mpsc::unbounded_channel::<Option<String>>();
            let cancel = Arc::new(CancelState::new());

            let storage = Arc::clone(&st);
            let callbacks = ChannelCallbacks {
                events: event_tx.clone(),
                input: tokio::sync::Mutex::new(input_rx),
                cancel: Arc::clone(&cancel),
            };
            tokio::spawn(async move {
                let Some(provider) = pi_rs_ai_auth::get_oauth_provider(&provider_id) else {
                    let _ = event_tx.send(serde_json::json!({
                        "type": "error",
                        "message": format!("Unknown OAuth provider: {provider_id}"),
                    }));
                    return;
                };
                match provider.login(&callbacks).await {
                    Ok(credentials) => {
                        storage
                            .lock()
                            .await
                            .set(&provider_id, AuthCredential::OAuth(credentials));
                        let _ = event_tx.send(serde_json::json!({ "type": "done" }));
                    }
                    Err(error) => {
                        let message = match error {
                            pi_rs_ai_auth::AuthError::Cancelled => LOGIN_CANCELLED.to_owned(),
                            other => other.to_string(),
                        };
                        let _ = event_tx.send(serde_json::json!({
                            "type": "error",
                            "message": message,
                        }));
                    }
                }
            });

            lua.create_userdata(LoginHandle {
                events: tokio::sync::Mutex::new(event_rx),
                input: input_tx,
                cancel,
            })
        })?,
    )?;

    pi.set("auth", auth)?;
    Ok(())
}
