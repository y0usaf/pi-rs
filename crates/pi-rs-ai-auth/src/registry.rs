//! Port of `utils/oauth/index.ts` — the OAuth provider registry and the
//! high-level credential API.
//!
//! The spec's registry is a module-level `Map`; `Map.set` semantics are
//! preserved (re-registration replaces in place, keeping insertion
//! order; registering after deletion appends). All three built-in subscription
//! providers are registered in spec order. The deprecated surface
//! (`refreshOAuthToken`, `getOAuthProviderInfoList`) has no spec consumer and
//! is not ported.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use crate::anthropic::anthropic_flow;
use crate::engine::now_ms;
use crate::error::AuthError;
use crate::github_copilot::github_copilot_flow;
use crate::openai_codex::openai_codex_flow;
use crate::types::{OAuthCredentials, OAuthProviderInterface};

/// Spec: `BUILT_IN_OAUTH_PROVIDERS`.
fn built_in_oauth_providers() -> Vec<Arc<dyn OAuthProviderInterface>> {
    vec![
        Arc::new(anthropic_flow()),
        Arc::new(github_copilot_flow()),
        Arc::new(openai_codex_flow()),
    ]
}

static REGISTRY: LazyLock<Mutex<Vec<Arc<dyn OAuthProviderInterface>>>> =
    LazyLock::new(|| Mutex::new(built_in_oauth_providers()));

fn registry() -> MutexGuard<'static, Vec<Arc<dyn OAuthProviderInterface>>> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Spec: `getOAuthProvider(id)`.
pub fn get_oauth_provider(id: &str) -> Option<Arc<dyn OAuthProviderInterface>> {
    registry().iter().find(|p| p.id() == id).cloned()
}

/// Spec: `registerOAuthProvider(provider)`.
pub fn register_oauth_provider(provider: Arc<dyn OAuthProviderInterface>) {
    let mut reg = registry();
    if let Some(existing) = reg.iter_mut().find(|p| p.id() == provider.id()) {
        *existing = provider;
    } else {
        reg.push(provider);
    }
}

/// Spec: `unregisterOAuthProvider(id)` — a built-in is restored to its
/// built-in implementation; custom providers are removed completely.
pub fn unregister_oauth_provider(id: &str) {
    if let Some(built_in) = built_in_oauth_providers()
        .into_iter()
        .find(|p| p.id() == id)
    {
        register_oauth_provider(built_in);
        return;
    }
    registry().retain(|p| p.id() != id);
}

/// Spec: `resetOAuthProviders()`.
pub fn reset_oauth_providers() {
    *registry() = built_in_oauth_providers();
}

/// Spec: `getOAuthProviders()` — insertion order.
pub fn get_oauth_providers() -> Vec<Arc<dyn OAuthProviderInterface>> {
    registry().clone()
}

/// Spec: the `{ newCredentials, apiKey }` result of `getOAuthApiKey`.
#[derive(Clone, Debug)]
pub struct OAuthApiKeyResult {
    pub new_credentials: OAuthCredentials,
    pub api_key: String,
}

/// Spec: `getOAuthApiKey(providerId, credentials)` — resolve an API key
/// from stored credentials, refreshing expired tokens.
pub async fn get_oauth_api_key(
    provider_id: &str,
    credentials: &HashMap<String, OAuthCredentials>,
) -> Result<Option<OAuthApiKeyResult>, AuthError> {
    let Some(provider) = get_oauth_provider(provider_id) else {
        return Err(AuthError::Message(format!(
            "Unknown OAuth provider: {provider_id}"
        )));
    };

    let Some(creds) = credentials.get(provider_id) else {
        return Ok(None);
    };

    // Spec: refresh if expired (`Date.now() >= creds.expires`).
    let creds = if now_ms() >= creds.expires {
        provider.refresh_token(creds).await.map_err(|_| {
            AuthError::Message(format!("Failed to refresh OAuth token for {provider_id}"))
        })?
    } else {
        creds.clone()
    };

    let api_key = provider.get_api_key(&creds);
    Ok(Some(OAuthApiKeyResult {
        new_credentials: creds,
        api_key,
    }))
}
