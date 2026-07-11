//! Port of `core/model-registry.ts` — manages models and resolves
//! request auth via [`AuthStorage`].
//!
//! Divergences (recorded):
//! - the spec's registry *owns* its `authStorage`; here the storage
//!   handle is passed explicitly so the substrate can share one
//!   [`AuthStorage`] instance between the `pi.auth` bindings, the
//!   `pi.ai` registry bindings, and the CLI — same instance semantics,
//!   different ownership;
//! - WS2.6 subset: the built-in catalog + auth resolution half. The
//!   models.json half (custom models, provider/model overrides, schema
//!   validation, `$ENV`/`!command` header interpolation) and the
//!   `registerProvider` glue land when their consumers do. Until then
//!   `get_error()` is always `None` and provider request configs are
//!   empty, so `hasConfiguredAuth` reduces to `authStorage.hasAuth` —
//!   behaviorally identical to the spec with no models.json on disk.

use pi_rs_ai::registry::{get_models, get_providers};
use pi_rs_ai_types::Model;
use std::collections::BTreeMap;

use crate::auth_storage::{AuthCredential, AuthStatus, AuthStorage};

/// Spec: `ResolvedRequestAuth`.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedRequestAuth {
    Ok {
        api_key: Option<String>,
        headers: Option<BTreeMap<String, String>>,
    },
    Err {
        error: String,
    },
}

/// Spec: `ModelRegistry` — loads and manages models, resolves API keys.
pub struct ModelRegistry {
    models: Vec<Model>,
    load_error: Option<String>,
}

impl ModelRegistry {
    /// Spec: `ModelRegistry.create` / `inMemory` (no models.json half
    /// landed yet, so the two constructors coincide).
    pub fn new(auth_storage: &AuthStorage) -> Self {
        let mut registry = Self {
            models: Vec::new(),
            load_error: None,
        };
        registry.load_models(auth_storage);
        registry
    }

    /// Spec: `refresh()`.
    pub fn refresh(&mut self, auth_storage: &AuthStorage) {
        self.load_error = None;
        self.load_models(auth_storage);
    }

    /// Spec: `getError()`.
    pub fn get_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Spec: `loadModels()` — built-ins, then OAuth providers may
    /// modify their models (e.g. update baseUrl).
    fn load_models(&mut self, auth_storage: &AuthStorage) {
        let mut combined: Vec<Model> = get_providers()
            .into_iter()
            .flat_map(|provider| get_models(provider).iter().cloned())
            .collect();

        for oauth_provider in pi_rs_ai_auth::get_oauth_providers() {
            if let Some(AuthCredential::OAuth(cred)) = auth_storage.get(oauth_provider.id()) {
                let cred = cred.clone();
                combined = oauth_provider.modify_models(combined, &cred);
            }
        }

        self.models = combined;
    }

    /// Spec: `getAll()`.
    pub fn get_all(&self) -> &[Model] {
        &self.models
    }

    /// Spec: `getAvailable()` — only models with auth configured; a
    /// fast check that doesn't refresh OAuth tokens.
    pub fn get_available(&self, auth_storage: &AuthStorage) -> Vec<&Model> {
        self.models
            .iter()
            .filter(|m| self.has_configured_auth(auth_storage, m))
            .collect()
    }

    /// Spec: `find(provider, modelId)`.
    pub fn find(&self, provider: &str, model_id: &str) -> Option<&Model> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.id == model_id)
    }

    /// Spec: `hasConfiguredAuth(model)`.
    pub fn has_configured_auth(&self, auth_storage: &AuthStorage, model: &Model) -> bool {
        auth_storage.has_auth(&model.provider)
    }

    /// Spec: `getApiKeyAndHeaders(model)`.
    pub async fn get_api_key_and_headers(
        &self,
        auth_storage: &mut AuthStorage,
        model: &Model,
    ) -> ResolvedRequestAuth {
        let api_key = auth_storage.get_api_key(&model.provider).await;
        ResolvedRequestAuth::Ok {
            api_key,
            headers: model.headers.clone().filter(|headers| !headers.is_empty()),
        }
    }

    /// Spec: `getProviderAuthStatus(provider)`.
    pub fn get_provider_auth_status(
        &self,
        auth_storage: &AuthStorage,
        provider: &str,
    ) -> AuthStatus {
        auth_storage.get_auth_status(provider)
    }

    /// Spec: `isUsingOAuth(model)`.
    pub fn is_using_oauth(&self, auth_storage: &AuthStorage, model: &Model) -> bool {
        matches!(
            auth_storage.get(&model.provider),
            Some(AuthCredential::OAuth(_))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_storage::AuthStorageData;

    fn storage_with(data: AuthStorageData) -> AuthStorage {
        AuthStorage::in_memory(data)
    }

    #[test]
    fn all_models_are_the_full_catalog() {
        let storage = storage_with(AuthStorageData::new());
        let registry = ModelRegistry::new(&storage);
        let catalog_len: usize = get_providers()
            .into_iter()
            .map(|p| get_models(p).len())
            .sum();
        assert_eq!(registry.get_all().len(), catalog_len);
    }

    #[test]
    fn available_filters_by_auth() {
        let mut data = AuthStorageData::new();
        data.insert(
            "anthropic".to_owned(),
            AuthCredential::ApiKey { key: "sk".into() },
        );
        let storage = storage_with(data);
        let registry = ModelRegistry::new(&storage);
        let available = registry.get_available(&storage);
        assert!(!available.is_empty());
        // The filter is exactly has_auth per model provider (env keys
        // may add providers on a developer machine).
        assert!(available.iter().all(|m| storage.has_auth(&m.provider)));
        assert!(available.iter().any(|m| m.id == "claude-opus-4-8"));
    }

    #[test]
    fn find_locates_catalog_rows() {
        let storage = storage_with(AuthStorageData::new());
        let registry = ModelRegistry::new(&storage);
        assert!(registry.find("anthropic", "claude-opus-4-8").is_some());
        assert!(registry.find("anthropic", "nope").is_none());
    }
}
