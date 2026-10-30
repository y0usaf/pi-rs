//! `core/model-runtime.ts` for print mode: the built-in catalog composed with models.json, which
//! providers have auth configured, and the auth each request gets (`resolveProviderAuth`).
//!
//! Auth order, as upstream: a `--api-key` key, else the credential stored in auth.json (an OAuth
//! token, refreshed when it expires within five minutes, or an API key), else models.json's
//! `apiKey`, else the provider's environment variables. models.json `headers` and `authHeader`
//! apply on top, then the model's own headers.

use crate::auth::{AuthStore, Credential, OAUTH_MINIMUM_VALIDITY_MS};
use crate::config_value::{self, Env};
use crate::model_config::{self, ModelConfig};
use crate::remote_catalog::{self, Store};
use pi_durable::ai::registry::Auth;
use pi_durable::ai::types::Model;
use pi_durable::cancel::CancelToken;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

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
const OAUTH_REFRESH: [&str; 8] = [
    "anthropic",
    "github-copilot",
    "kimi-coding",
    "meta",
    "openai",
    "openai-codex",
    "radius",
    "xai",
];

pub struct ModelRegistry {
    pub config: ModelConfig,
    pub network: bool,
    catalog: Vec<(String, Vec<Model>)>,
    remote: BTreeSet<String>,
    store: Option<Store>,
    overlays: Mutex<BTreeMap<String, Vec<Model>>>,
    models: RwLock<Arc<Vec<Model>>>,
    listed: Mutex<BTreeSet<(String, String)>>,
    builtin: BTreeSet<String>,
    composition_errors: RwLock<BTreeMap<String, String>>,
    auth: Mutex<AuthStore>,
    runtime_keys: Mutex<BTreeMap<String, String>>,
    virtuals: Mutex<Vec<Model>>,
}

pub const VIRTUAL_API: &str = "pi-virtual";

pub fn is_virtual(model: &Model) -> bool {
    model.api == VIRTUAL_API
}

#[derive(Default)]
pub struct Refreshed {
    pub aborted: bool,
    pub errors: BTreeMap<String, String>,
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
        let store_path = crate::paths::join(agent_dir, "models-store.json");
        let _ = crate::auth::ensure_file(&store_path);
        let auth = AuthStore::open(&crate::paths::join(agent_dir, "auth.json"));
        let mut catalog = pi_durable::ai::catalog::all();
        let remote = catalog
            .iter()
            .map(|model| model.provider.clone())
            .filter(|provider| remote_catalog::extends(provider))
            .collect();
        catalog.extend(crate::mount::catalog().iter().cloned());
        let mut registry = Self::new(config, auth, catalog);
        registry.remote = remote;
        registry.builtin.extend(
            pi_durable::ai::catalog::typed()
                .iter()
                .filter_map(|model| model.get("provider").and_then(Value::as_str))
                .map(str::to_owned),
        );
        registry.store = Some(Store::at(&store_path));
        registry.refresh(false, false, None, &CancelToken::new());
        registry
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
        let catalog = order
            .into_iter()
            .map(|provider| {
                let models = by_provider.remove(&provider).unwrap_or_default();
                (provider, models)
            })
            .collect();
        let registry = Self {
            config,
            network: std::env::var_os("PI_OFFLINE").is_none_or(|value| value.is_empty()),
            catalog,
            remote: BTreeSet::new(),
            store: None,
            overlays: Mutex::new(BTreeMap::new()),
            models: RwLock::new(Arc::new(Vec::new())),
            listed: Mutex::new(BTreeSet::new()),
            builtin,
            composition_errors: RwLock::new(BTreeMap::new()),
            auth: Mutex::new(auth),
            runtime_keys: Mutex::new(BTreeMap::new()),
            virtuals: Mutex::new(Vec::new()),
        };
        registry.compose();
        registry
    }

    fn compose(&self) {
        let overlays = self.overlays.lock().unwrap();
        let mut models = Vec::new();
        let mut errors = BTreeMap::new();
        for (provider, builtin) in &self.catalog {
            let base = match overlays.get(provider) {
                Some(overlay) if !overlay.is_empty() => remote_catalog::merge(builtin, overlay),
                _ => builtin.clone(),
            };
            match model_config::compose(provider, &base, self.config.providers.get(provider)) {
                Ok(composed) => models.extend(composed),
                Err(e) => {
                    errors.insert(provider.clone(), e);
                    models.extend(base);
                }
            }
        }
        for virtual_model in self.virtuals.lock().unwrap().iter() {
            models.retain(|model| {
                model.provider != virtual_model.provider || model.id != virtual_model.id
            });
            let at = models
                .iter()
                .rposition(|model| model.provider == virtual_model.provider)
                .map_or(models.len(), |last| last + 1);
            models.insert(at, virtual_model.clone());
        }
        self.listed.lock().unwrap().extend(
            models
                .iter()
                .map(|model| (model.provider.clone(), model.id.clone())),
        );
        *self.models.write().unwrap() = Arc::new(models);
        *self.composition_errors.write().unwrap() = errors;
    }

    /// Whether the catalog has held this model at any point since pi-rs started.
    pub fn has_listed(&self, provider: &str, id: &str) -> bool {
        self.listed
            .lock()
            .unwrap()
            .contains(&(provider.to_owned(), id.to_owned()))
    }

    pub fn refresh(
        &self,
        allow_network: bool,
        force: bool,
        only: Option<&[String]>,
        cancel: &CancelToken,
    ) -> Refreshed {
        let mut refreshed = Refreshed::default();
        let Some(store) = &self.store else {
            return refreshed;
        };
        let providers: Vec<&String> = self
            .remote
            .iter()
            .filter(|provider| only.is_none_or(|only| only.contains(provider)))
            .collect();
        let entries = store.entries();
        let stored = |provider: &str| entries.get(provider).and_then(Value::as_object);
        {
            let mut overlays = self.overlays.lock().unwrap();
            for provider in &providers {
                let overlay = stored(provider)
                    .map(|entry| remote_catalog::overlay(provider, entry))
                    .unwrap_or_default();
                overlays.insert((*provider).clone(), overlay);
            }
        }
        if allow_network && !cancel.is_cancelled() {
            let online: Vec<&String> = providers
                .into_iter()
                .filter(|provider| self.has_configured_auth(provider))
                .collect();
            let results: Vec<(String, Result<Option<remote_catalog::Entry>, String>)> =
                std::thread::scope(|scope| {
                    let running: Vec<_> = online
                        .iter()
                        .map(|provider| {
                            let entry = stored(provider);
                            scope.spawn(move || {
                                let result =
                                    remote_catalog::fetch(provider, entry, force, store, cancel);
                                ((*provider).clone(), result)
                            })
                        })
                        .collect();
                    running
                        .into_iter()
                        .filter_map(|thread| thread.join().ok())
                        .collect()
                });
            let mut overlays = self.overlays.lock().unwrap();
            for (provider, result) in results {
                match result {
                    Ok(Some(entry)) => {
                        overlays
                            .insert(provider.clone(), remote_catalog::overlay(&provider, &entry));
                    }
                    Ok(None) => {}
                    Err(error) if !cancel.is_cancelled() => {
                        refreshed.errors.insert(provider, error);
                    }
                    Err(_) => {}
                }
            }
        }
        self.compose();
        refreshed.aborted = cancel.is_cancelled();
        refreshed
    }

    pub fn set_virtual(&self, models: Vec<Model>) {
        *self.virtuals.lock().unwrap() = models;
        self.compose();
    }

    fn only_virtual(&self, provider: &str) -> bool {
        !self.builtin.contains(provider)
            && !self.config.providers.contains_key(provider)
            && self
                .virtuals
                .lock()
                .unwrap()
                .iter()
                .any(|model| model.provider == provider)
    }

    pub fn models(&self) -> Arc<Vec<Model>> {
        Arc::clone(&self.models.read().unwrap())
    }

    /// `AuthStorage.read`: the provider's stored credential.
    pub fn credential(&self, provider: &str) -> Option<Credential> {
        self.auth.lock().unwrap().read(provider)
    }

    /// `AuthStorage.list`: the providers with a stored credential, and its type.
    pub fn stored_credentials(&self) -> Vec<(String, &'static str)> {
        let auth = self.auth.lock().unwrap();
        auth.providers()
            .into_iter()
            .map(|provider| {
                let kind = match auth.read(&provider) {
                    Some(Credential::OAuth(_)) => "oauth",
                    _ => "api_key",
                };
                (provider, kind)
            })
            .collect()
    }

    /// `getProvider`: whether the registry knows `provider`.
    pub fn provider_exists(&self, provider: &str) -> bool {
        self.builtin.contains(provider)
            || self.config.providers.contains_key(provider)
            || self.models().iter().any(|m| m.provider == provider)
    }

    /// `checkAuth`: the credential type requests to `provider` would use, if any.
    pub fn check_auth(&self, provider: &str) -> Option<&'static str> {
        match self.credential(provider) {
            Some(Credential::OAuth(_)) if self.has_oauth(provider) => Some("oauth"),
            _ if self.has_api_key_auth(provider) => self
                .provider_auth(provider, OAUTH_MINIMUM_VALIDITY_MS)
                .ok()
                .and_then(|auth| credential_value(&auth))
                .map(|_| "api_key"),
            _ => None,
        }
    }

    /// Re-reads `auth.json` once a login saved a credential.
    pub fn reload_auth(&self) {
        let mut auth = self.auth.lock().unwrap();
        let path = auth.path.clone();
        *auth = AuthStore::open(&path);
    }

    /// The number of providers with auth, as the footer counts them.
    pub fn provider_count(&self) -> usize {
        let mut providers: Vec<String> = self
            .available()
            .into_iter()
            .map(|model| model.provider)
            .collect();
        providers.sort();
        providers.dedup();
        providers.len()
    }

    pub fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        self.models()
            .iter()
            .find(|m| m.provider == provider && m.id == id)
            .cloned()
    }

    pub fn model_keys(&self) -> Value {
        let mut out = Map::new();
        for model in self.models().iter() {
            if is_virtual(model) {
                continue;
            }
            let Some(config) = self.provider_config(&model.provider) else {
                continue;
            };
            let keys = model_config::own_keys(model, config);
            if let Value::Object(ids) = out
                .entry(model.provider.clone())
                .or_insert_with(|| Value::Object(Map::new()))
            {
                ids.insert(model.id.clone(), Value::from(keys));
            }
        }
        Value::Object(out)
    }

    pub fn typed_models(&self) -> Vec<Value> {
        pi_durable::ai::catalog::typed()
            .iter()
            .map(|model| {
                let mut model = model.clone();
                let provider = model.get("provider").and_then(Value::as_str);
                if let Some(config) = provider.and_then(|provider| self.provider_config(provider)) {
                    if config.get("oauth").and_then(Value::as_str) != Some("radius")
                        && let Some(url) = config.get("baseUrl").filter(|url| !url.is_null())
                    {
                        model.insert("baseUrl".to_owned(), url.clone());
                    }
                    if model.get("type").and_then(Value::as_str) == Some("classifier")
                        && let Some(Value::Object(compat)) = config.get("compat")
                    {
                        let mut merged = model
                            .get("compat")
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        merged.extend(compat.clone());
                        model.insert("compat".to_owned(), Value::Object(merged));
                    }
                }
                Value::Object(model)
            })
            .collect()
    }

    /// `getError()`: models.json and composition problems.
    pub fn error(&self) -> Option<String> {
        let mut errors: Vec<String> = self.config.error.iter().cloned().collect();
        errors.extend(
            self.composition_errors
                .read()
                .unwrap()
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
        self.config.providers.get(provider).filter(|_| {
            !self
                .composition_errors
                .read()
                .unwrap()
                .contains_key(provider)
        })
    }

    fn has_oauth(&self, provider: &str) -> bool {
        self.oauth_flow(provider).is_some()
    }

    /// The built-in provider whose OAuth login and refresh `provider` uses: itself, or the one a
    /// provider an extension registered names.
    fn oauth_flow(&self, provider: &str) -> Option<String> {
        let flow = match crate::extensions::live().and_then(|live| live.oauth_flow(provider)) {
            Some(flow) => flow,
            None => provider.to_owned(),
        };
        (self.builtin.contains(&flow) && OAUTH_PROVIDERS.contains(&flow.as_str())).then_some(flow)
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
        if crate::mount::mounts(provider) || self.only_virtual(provider) {
            return true;
        }
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
    pub fn available(&self) -> Vec<Model> {
        let mut cache: BTreeMap<String, bool> = BTreeMap::new();
        self.models()
            .iter()
            .filter(|m| {
                *cache
                    .entry(m.provider.clone())
                    .or_insert_with(|| self.has_configured_auth(&m.provider))
            })
            .cloned()
            .collect()
    }

    /// The auth one request to `model` gets.
    pub fn resolve_auth(&self, model: &Model) -> Result<Auth, String> {
        self.auth_for(&model.provider, Some(model), OAUTH_MINIMUM_VALIDITY_MS)
    }

    /// `getAuth(providerId)`: the auth a request to `provider` gets, an OAuth token that expires
    /// within `min_validity_ms` refreshed first.
    pub fn provider_auth(&self, provider: &str, min_validity_ms: i64) -> Result<Auth, String> {
        self.auth_for(provider, None, min_validity_ms)
    }

    fn auth_for(
        &self,
        provider: &str,
        model: Option<&Model>,
        min_validity_ms: i64,
    ) -> Result<Auth, String> {
        let mut auth = Auth::default();
        let mut env: Env = Env::new();
        let runtime = self.runtime_keys.lock().unwrap().get(provider).cloned();
        if let Some(key) = runtime.filter(|_| self.has_api_key_auth(provider)) {
            auth.api_key = Some(key);
        } else {
            let stored = self.auth.lock().unwrap().read(provider);
            match stored {
                Some(Credential::OAuth(_)) if self.has_oauth(provider) => {
                    let access = self.oauth_access(provider, min_validity_ms)?;
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
        if let Some(model) = model
            && let Some(headers) = config_value::resolve_headers(
                model_config::model_headers(model, config).as_ref(),
                &format!("model \"{}/{}\"", model.provider, model.id),
                env_ref,
            )?
        {
            auth.headers.extend(headers);
        }
        auth.env = env;
        Ok(auth)
    }

    /// The access token of the stored OAuth credential, refreshed first when it expires within
    /// `min_validity_ms`.
    fn oauth_access(&self, provider: &str, min_validity_ms: i64) -> Result<String, String> {
        let flow = self
            .oauth_flow(provider)
            .unwrap_or_else(|| provider.to_owned());
        let refresh = |current: &Map<String, Value>| -> Result<Map<String, Value>, String> {
            if !OAUTH_REFRESH.contains(&flow.as_str()) {
                return Err(format!(
                    "refreshing {provider} OAuth tokens is not supported by pi-rs; log in again with pi"
                ));
            }
            refresh_oauth(&flow, current)
        };
        let fresh =
            self.auth
                .lock()
                .unwrap()
                .fresh_oauth_with(provider, min_validity_ms, &refresh)?;
        fresh
            .and_then(|c| c.get("access").and_then(Value::as_str).map(str::to_owned))
            .ok_or_else(|| format!("No OAuth credential for {provider}"))
    }
}

/// `getAuthCredential`: the API key a request carries, else the bearer token of its
/// Authorization header.
pub fn credential_value(auth: &Auth) -> Option<String> {
    if let Some(key) = auth.api_key.as_ref() {
        return Some(key.clone());
    }
    let (_, value) = auth
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))?;
    let (scheme, rest) = value.split_once(char::is_whitespace)?;
    let token = rest.trim_start_matches(char::is_whitespace);
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| token.to_owned())
}

fn refresh_oauth(
    provider: &str,
    stored: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    use pi_durable::ai::oauth::{
        anthropic, github_copilot, kimi_coding, meta, openai_chatgpt, openai_codex, radius, xai,
    };
    let credentials = match provider {
        "anthropic" => anthropic::refresh(&anthropic::Endpoints::from_env(), stored, None)?,
        "github-copilot" => {
            github_copilot::refresh(&github_copilot::Endpoints::from_env(), stored, None)?
        }
        "kimi-coding" => kimi_coding::refresh(&kimi_coding::Endpoints::from_env(), stored, None)?,
        "meta" => meta::refresh(&meta::Endpoints::from_env(), stored, None)?,
        "openai" => openai_chatgpt::refresh(&openai_chatgpt::Endpoints::from_env(), stored, None)?,
        "openai-codex" => {
            openai_codex::refresh(&openai_codex::Endpoints::from_env(), stored, None)?
        }
        "radius" => radius::refresh(&radius::Endpoints::from_env(), stored, None)?,
        "xai" => xai::refresh(&xai::Endpoints::from_env(), stored, None)?,
        _ => {
            return Err(format!(
                "refreshing {provider} OAuth tokens is not supported"
            ));
        }
    };
    pi_durable::ai::oauth::stored(provider, &credentials)
        .as_object()
        .cloned()
        .ok_or_else(|| format!("stored {provider} credential is not an object"))
}
