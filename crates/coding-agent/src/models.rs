//! `core/model-runtime.ts` for print mode: the built-in catalog composed with models.json, which
//! providers have auth configured, and the auth each request gets (`resolveProviderAuth`).
//!
//! Auth order, as upstream: a `--api-key` key, else the credential stored in auth.json (an OAuth
//! token, refreshed when it expires within five minutes, or an API key), else models.json's
//! `apiKey`, else the provider's environment variables. models.json `headers` and `authHeader`
//! apply on top, then the model's own headers.

use crate::auth::{AuthStore, Credential};
use crate::config_value::{self, Env};
use crate::model_config::{self, ModelConfig};
use pi_durable::ai::registry::Auth;
use pi_durable::ai::types::Model;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// Providers whose built-in definition has an OAuth login upstream.
const OAUTH_PROVIDERS: [&str; 9] = [
    "anthropic",
    "openai-codex",
    "github-copilot",
    "kimi-coding",
    "meta",
    "openai",
    "openrouter",
    "xai",
    "radius",
];
/// Of those, the ones whose token refresh this port has (pi-ai's Rust OAuth flows).
const OAUTH_REFRESH: [&str; 6] = [
    "anthropic",
    "github-copilot",
    "kimi-coding",
    "meta",
    "openai-codex",
    "xai",
];

pub struct ModelRegistry {
    pub config: ModelConfig,
    models: Vec<Model>,
    builtin: BTreeSet<String>,
    composition_errors: BTreeMap<String, String>,
    auth: Mutex<AuthStore>,
    runtime_keys: Mutex<BTreeMap<String, String>>,
}

fn env_names(provider: &str) -> Vec<&'static str> {
    let mut names = Vec::new();
    if provider == "anthropic" {
        names.push("ANTHROPIC_AUTH_TOKEN");
    }
    names
}

impl ModelRegistry {
    /// `ModelRuntime.create()` without network: `<agentDir>/models.json` over the built-in catalog,
    /// credentials from `<agentDir>/auth.json`. Like upstream it creates an empty
    /// `models-store.json` next to models.json when there is none.
    pub fn load(agent_dir: &str) -> Self {
        let models_path = crate::paths::join(agent_dir, "models.json");
        let config = ModelConfig::load(&models_path);
        let _ = crate::auth::ensure_file(&crate::paths::join(agent_dir, "models-store.json"));
        let auth = AuthStore::open(&crate::paths::join(agent_dir, "auth.json"));
        Self::new(config, auth, pi_durable::ai::catalog::all())
    }

    pub fn new(config: ModelConfig, auth: AuthStore, catalog: Vec<Model>) -> Self {
        let mut by_provider: BTreeMap<String, Vec<Model>> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for model in catalog {
            if !by_provider.contains_key(&model.provider) {
                order.push(model.provider.clone());
            }
            by_provider
                .entry(model.provider.clone())
                .or_default()
                .push(model);
        }
        let builtin: BTreeSet<String> = order.iter().cloned().collect();
        for id in config.providers.keys() {
            if !by_provider.contains_key(id) {
                order.push(id.clone());
            }
        }
        let mut models = Vec::new();
        let mut composition_errors = BTreeMap::new();
        for provider in &order {
            let base = by_provider.get(provider).cloned().unwrap_or_default();
            match model_config::compose(provider, &base, config.providers.get(provider)) {
                Ok(composed) => models.extend(composed),
                Err(e) => {
                    composition_errors.insert(provider.clone(), e);
                    models.extend(base);
                }
            }
        }
        Self {
            config,
            models,
            builtin,
            composition_errors,
            auth: Mutex::new(auth),
            runtime_keys: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// Re-reads `auth.json` once a login saved a credential.
    pub fn reload_auth(&self) {
        let mut auth = self.auth.lock().unwrap();
        let path = auth.path.clone();
        *auth = AuthStore::open(&path);
    }

    /// The number of providers with auth, as the footer counts them.
    pub fn provider_count(&self) -> usize {
        let mut providers: Vec<&str> = self
            .available()
            .iter()
            .map(|model| model.provider.as_str())
            .collect();
        providers.sort();
        providers.dedup();
        providers.len()
    }

    pub fn get_model(&self, provider: &str, id: &str) -> Option<&Model> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.id == id)
    }

    /// `getError()`: models.json and composition problems.
    pub fn error(&self) -> Option<String> {
        let mut errors: Vec<String> = self.config.error.iter().cloned().collect();
        errors.extend(
            self.composition_errors
                .iter()
                .map(|(p, e)| format!("Provider \"{p}\": {e}")),
        );
        (!errors.is_empty()).then(|| errors.join("\n\n"))
    }

    /// `setRuntimeApiKey` (`--api-key`).
    pub fn set_runtime_api_key(&self, provider: &str, key: &str) {
        self.runtime_keys
            .lock()
            .unwrap()
            .insert(provider.to_owned(), key.to_owned());
    }

    fn provider_config(&self, provider: &str) -> Option<&model_config::ProviderConfig> {
        self.config
            .providers
            .get(provider)
            .filter(|_| !self.composition_errors.contains_key(provider))
    }

    fn has_oauth(&self, provider: &str) -> bool {
        self.builtin.contains(provider) && OAUTH_PROVIDERS.contains(&provider)
    }

    fn has_api_key_auth(&self, provider: &str) -> bool {
        // Every built-in provider but openai-codex takes a key; a models.json provider gets key
        // auth unless it only inherits OAuth.
        if self.builtin.contains(provider) {
            return provider != "openai-codex"
                || self
                    .provider_config(provider)
                    .is_some_and(|c| c.contains_key("apiKey"));
        }
        self.config.providers.contains_key(provider)
    }

    fn configured_api_key(&self, provider: &str) -> Option<String> {
        self.provider_config(provider)
            .and_then(|c| c.get("apiKey"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    fn ambient_key(&self, provider: &str) -> Option<String> {
        if !self.builtin.contains(provider) {
            return None;
        }
        for name in env_names(provider) {
            if let Some(v) = std::env::var(name).ok().filter(|v| !v.is_empty()) {
                return Some(v);
            }
        }
        pi_durable::ai::auth::env_api_key(provider, &BTreeMap::new())
    }

    /// `hasConfiguredAuth`: whether requests to this provider would get credentials.
    pub fn has_configured_auth(&self, provider: &str) -> bool {
        if self.runtime_keys.lock().unwrap().contains_key(provider) {
            return self.has_api_key_auth(provider);
        }
        let stored = self.auth.lock().unwrap().read(provider);
        match stored {
            Some(Credential::OAuth(_)) => return self.has_oauth(provider),
            Some(Credential::ApiKey { key: Some(k), .. }) if !k.is_empty() => {
                return self.has_api_key_auth(provider);
            }
            Some(Credential::ApiKey { .. }) => return false,
            None => {}
        }
        if !self.has_api_key_auth(provider) {
            return false;
        }
        if let Some(raw) = self.configured_api_key(provider) {
            return config_value::is_command(&raw)
                || config_value::env_var_names(&raw)
                    .iter()
                    .all(|n| std::env::var(n).ok().is_some_and(|v| !v.is_empty()));
        }
        self.ambient_key(provider).is_some()
    }

    /// `getAvailableSnapshot()`: models whose provider has auth, in catalog order.
    pub fn available(&self) -> Vec<&Model> {
        let mut cache: BTreeMap<&str, bool> = BTreeMap::new();
        self.models
            .iter()
            .filter(|m| {
                *cache
                    .entry(m.provider.as_str())
                    .or_insert_with(|| self.has_configured_auth(&m.provider))
            })
            .collect()
    }

    /// The auth one request to `model` gets.
    pub fn resolve_auth(&self, model: &Model) -> Result<Auth, String> {
        let provider = model.provider.as_str();
        let mut auth = Auth::default();
        let mut env: Env = Env::new();
        let runtime = self.runtime_keys.lock().unwrap().get(provider).cloned();
        if let Some(key) = runtime.filter(|_| self.has_api_key_auth(provider)) {
            auth.api_key = Some(key);
        } else {
            let stored = self.auth.lock().unwrap().read(provider);
            match stored {
                Some(Credential::OAuth(_)) if self.has_oauth(provider) => {
                    let access = self.oauth_access(provider)?;
                    auth.api_key = Some(access);
                }
                Some(Credential::ApiKey {
                    key,
                    env: stored_env,
                }) if self.has_api_key_auth(provider) => {
                    auth.api_key = key.filter(|k| !k.is_empty());
                    env = stored_env.unwrap_or_default();
                }
                Some(_) => {}
                None if self.has_api_key_auth(provider) => {
                    match self.configured_api_key(provider) {
                        Some(raw) => {
                            auth.api_key = Some(
                                config_value::resolve_or_err(
                                    &raw,
                                    &format!("API key for provider \"{provider}\""),
                                    None,
                                )
                                .map_err(|e| {
                                    format!("API key auth failed for provider {provider}: {e}")
                                })?,
                            );
                        }
                        None => {
                            if provider == "anthropic"
                                && let Some(token) = std::env::var("ANTHROPIC_AUTH_TOKEN")
                                    .ok()
                                    .filter(|v| !v.is_empty())
                            {
                                auth.headers
                                    .insert("Authorization".to_owned(), format!("Bearer {token}"));
                            }
                            if auth.headers.is_empty() {
                                auth.api_key = self.ambient_key(provider);
                            }
                        }
                    }
                }
                None => {}
            }
        }
        let config = self.provider_config(provider);
        let env_ref = (!env.is_empty()).then_some(&env);
        if let Some(headers) = config_value::resolve_headers(
            model_config::provider_headers(config).as_ref(),
            &format!("provider \"{provider}\""),
            env_ref,
        )? {
            auth.headers.extend(headers);
        }
        if config
            .and_then(|c| c.get("authHeader"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            let key = auth
                .api_key
                .clone()
                .ok_or("authHeader requires a resolved API key")?;
            auth.headers
                .insert("Authorization".to_owned(), format!("Bearer {key}"));
        }
        if let Some(headers) = config_value::resolve_headers(
            model_config::model_headers(model, config).as_ref(),
            &format!("model \"{}/{}\"", model.provider, model.id),
            env_ref,
        )? {
            auth.headers.extend(headers);
        }
        auth.env = env;
        Ok(auth)
    }

    /// The access token of the stored OAuth credential, refreshed first when it expires soon.
    fn oauth_access(&self, provider: &str) -> Result<String, String> {
        let refresh = |current: &Map<String, Value>| -> Result<Map<String, Value>, String> {
            if !OAUTH_REFRESH.contains(&provider) {
                return Err(format!(
                    "refreshing {provider} OAuth tokens is not supported by pi-rs; log in again with pi"
                ));
            }
            refresh_oauth(provider, current)
        };
        let fresh = self.auth.lock().unwrap().fresh_oauth(provider, &refresh)?;
        fresh
            .and_then(|c| c.get("access").and_then(Value::as_str).map(str::to_owned))
            .ok_or_else(|| format!("No OAuth credential for {provider}"))
    }
}

fn refresh_oauth(
    provider: &str,
    stored: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    use pi_durable::ai::oauth::{anthropic, github_copilot, kimi_coding, meta, openai_codex, xai};
    let credentials = match provider {
        "anthropic" => anthropic::refresh(&anthropic::Endpoints::from_env(), stored, None)?,
        "github-copilot" => {
            github_copilot::refresh(&github_copilot::Endpoints::from_env(), stored, None)?
        }
        "kimi-coding" => kimi_coding::refresh(&kimi_coding::Endpoints::from_env(), stored, None)?,
        "meta" => meta::refresh(&meta::Endpoints::from_env(), stored, None)?,
        "xai" => xai::refresh(&xai::Endpoints::from_env(), stored, None)?,
        _ => openai_codex::refresh(&openai_codex::Endpoints::from_env(), stored, None)?,
    };
    pi_durable::ai::oauth::stored(provider, &credentials)
        .as_object()
        .cloned()
        .ok_or_else(|| format!("stored {provider} credential is not an object"))
}
