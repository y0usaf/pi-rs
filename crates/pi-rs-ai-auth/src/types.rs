//! Port of `utils/oauth/types.ts` — credential shape, login callbacks,
//! and the OAuth provider interface.
//!
//! Divergences from the spec file (recorded):
//! - the deprecated aliases (`OAuthProvider`, `OAuthProviderInfo`) are
//!   not ported — no spec consumer uses them;
//! - cancellation and provider model IDs are represented as callback methods
//!   rather than JavaScript's `AbortSignal` and `getModels()` import. This keeps
//!   auth below the catalog crate while preserving observable flow behavior.

use std::future::Future;
use std::pin::Pin;

use pi_rs_ai_types::Model;
use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// Boxed future returned by async callback / provider methods (the
/// spec's `Promise<T>`; rejection = `Err`).
pub type AuthFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AuthError>> + Send + 'a>>;

/// Spec: `OAuthCredentials` — `refresh`/`access`/`expires` plus an open
/// index signature (`extra`, flattened through serde).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OAuthCredentials {
    pub refresh: String,
    pub access: String,
    /// Expiry as milliseconds since the epoch (spec: `Date.now()` scale).
    pub expires: i64,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Spec: `OAuthProviderId` — an open string, never a closed enum.
pub type OAuthProviderId = String;

/// Spec: `OAuthPrompt`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthPrompt {
    pub message: String,
    pub placeholder: Option<String>,
    pub allow_empty: bool,
}

/// Spec: `OAuthAuthInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthAuthInfo {
    pub url: String,
    pub instructions: Option<String>,
}

/// Spec: `OAuthDeviceCodeInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthDeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub interval_seconds: Option<f64>,
    pub expires_in_seconds: Option<f64>,
}

/// Spec: `OAuthSelectOption`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthSelectOption {
    pub id: String,
    pub label: String,
}

/// Spec: `OAuthSelectPrompt`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthSelectPrompt {
    pub message: String,
    pub options: Vec<OAuthSelectOption>,
}

/// Spec: `OAuthLoginCallbacks` — UI hooks a frontend supplies to a login
/// flow. Methods the spec marks optional (`onProgress?`,
/// `onManualCodeInput?`) have defaults; the rest are required.
pub trait OAuthLoginCallbacks: Send + Sync {
    /// Spec: `onAuth` — show the authorization URL.
    fn on_auth(&self, info: OAuthAuthInfo);

    /// Spec: `onDeviceCode` — show a device-code prompt.
    fn on_device_code(&self, info: OAuthDeviceCodeInfo);

    /// Spec: `onPrompt` — ask the user for a line of input.
    fn on_prompt(&self, prompt: OAuthPrompt) -> AuthFuture<'_, String>;

    /// Spec: `onSelect` — interactive selector; `Ok(None)` on cancel.
    fn on_select(&self, prompt: OAuthSelectPrompt) -> AuthFuture<'_, Option<String>>;

    /// Spec: `onProgress?`.
    fn on_progress(&self, _message: &str) {}

    /// Whether the spec's shared `AbortSignal` has fired.
    fn is_cancelled(&self) -> bool {
        false
    }

    /// Await cancellation. The default signal never fires.
    fn on_cancelled(&self) -> AuthFuture<'_, ()> {
        Box::pin(std::future::pending())
    }

    /// Catalog IDs used by Copilot's post-login policy-enablement calls.
    /// Supplied by the host to avoid a dependency from auth back to the
    /// transport/catalog crate.
    fn provider_model_ids(&self, _provider: &str) -> Vec<String> {
        Vec::new()
    }

    /// Spec: `onManualCodeInput?` — `None` means the callback is absent
    /// (the flow then relies on the callback server alone).
    fn on_manual_code_input(&self) -> Option<AuthFuture<'_, String>> {
        None
    }
}

/// Spec: `OAuthProviderInterface` — one OAuth provider: login flow,
/// token refresh, credential→API-key mapping.
pub trait OAuthProviderInterface: Send + Sync {
    fn id(&self) -> &str;

    fn name(&self) -> &str;

    /// Spec: `usesCallbackServer?` — login uses a local callback server
    /// and supports manual code input.
    fn uses_callback_server(&self) -> bool {
        false
    }

    /// Spec: `login(callbacks)` — run the flow, return credentials to
    /// persist.
    fn login<'a>(
        &'a self,
        callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials>;

    /// Spec: `refreshToken(credentials)`.
    fn refresh_token<'a>(
        &'a self,
        credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials>;

    /// Spec: `getApiKey(credentials)`.
    fn get_api_key(&self, credentials: &OAuthCredentials) -> String;

    /// Spec: `modifyModels?` — the default is the identity (behaviorally
    /// the same as the hook being absent).
    fn modify_models(&self, models: Vec<Model>, _credentials: &OAuthCredentials) -> Vec<Model> {
        models
    }
}
