use std::collections::BTreeMap;
use std::sync::Arc;

use super::event_stream::AssistantMessageEventStream;
use super::models::{Models, ModelsError};
use super::stream::error_stream;
use super::types::{
    AssistantMessage, DeferredHandle, Model, RequestHooks, SimpleStreamOptions, TranscriptContext,
};
use pi_rt::cancel::CancelToken;

pub trait Api: Send + Sync {
    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream;
}

#[derive(Debug, Clone, Default)]
pub struct Auth {
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub base_url: Option<String>,
    pub env: BTreeMap<String, String>,
}

pub type AuthResolver = Arc<dyn Fn(&Model) -> Result<Auth, String> + Send + Sync>;

/// The API a provider registered at runtime streams a model with, when one did.
pub type Router = Arc<dyn Fn(&Model, Origin) -> Option<Arc<dyn Api>> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    AgentLoop,
    Direct,
}

pub type Lookup = Arc<dyn Fn(&str, &str) -> Option<Model> + Send + Sync>;

#[derive(Default)]
pub struct Providers {
    models: Vec<Model>,
    apis: BTreeMap<String, Arc<dyn Api>>,
    auth: Option<AuthResolver>,
    router: Option<Router>,
    lookup: Option<Lookup>,
    hooks: RequestHooks,
}

impl Providers {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(feature = "anthropic", feature = "openai", feature = "vercel-gateway"))]
    pub fn builtin() -> Self {
        let mut providers = Self::new();
        #[cfg(feature = "anthropic")]
        providers.add_api("anthropic-messages", super::api::anthropic::api());
        #[cfg(feature = "openai")]
        {
            providers.add_api("openai-completions", super::api::openai_completions::api());
            providers.add_api("openai-responses", super::api::openai_responses::api());
            providers.add_api(
                "openai-codex-responses",
                super::api::openai_responses::api(),
            );
            providers.add_api("mistral-conversations", super::api::mistral::api());
            providers.add_api("azure-openai-responses", super::api::azure::api());
            providers.add_api("pi-messages", super::api::pi_messages::api());
        }
        #[cfg(feature = "http")]
        providers.add_api("bedrock-converse-stream", super::api::bedrock::api());
        #[cfg(feature = "http")]
        providers.add_api("google-generative-ai", super::api::google::api());
        #[cfg(feature = "http")]
        providers.add_api("google-vertex", super::api::google::vertex_api());
        providers.add_models(super::catalog::all());
        providers
    }

    pub fn add_api(&mut self, name: &str, api: Arc<dyn Api>) -> &mut Self {
        self.apis.insert(name.to_owned(), api);
        self
    }

    pub fn add_models(&mut self, models: impl IntoIterator<Item = Model>) -> &mut Self {
        for model in models {
            match self
                .models
                .iter_mut()
                .find(|known| known.provider == model.provider && known.id == model.id)
            {
                Some(slot) => *slot = model,
                None => self.models.push(model),
            }
        }
        self
    }

    pub fn set_auth(&mut self, resolver: AuthResolver) -> &mut Self {
        self.auth = Some(resolver);
        self
    }

    pub fn set_router(&mut self, router: Router) -> &mut Self {
        self.router = Some(router);
        self
    }

    pub fn set_lookup(&mut self, lookup: Lookup) -> &mut Self {
        self.lookup = Some(lookup);
        self
    }

    pub fn set_hooks(&mut self, hooks: RequestHooks) -> &mut Self {
        self.hooks = hooks;
        self
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// Streams with the API the model names, past any provider the router hands out.
    pub fn stream_builtin(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        self.dispatch(model, context, options, None)
    }

    pub fn stream(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
        origin: Origin,
    ) -> AssistantMessageEventStream {
        self.dispatch(model, context, options, Some(origin))
    }

    fn dispatch(
        &self,
        model: &Model,
        context: TranscriptContext,
        mut options: SimpleStreamOptions,
        origin: Option<Origin>,
    ) -> AssistantMessageEventStream {
        if origin == Some(Origin::AgentLoop) {
            options.hooks = options.hooks.or(&self.hooks);
        }
        let api = match origin
            .and_then(|origin| self.router.as_ref().and_then(|route| route(model, origin)))
        {
            Some(api) => api,
            None => match self.apis.get(&model.api) {
                Some(api) => Arc::clone(api),
                None => {
                    return error_stream(
                        model,
                        format!("No API provider registered for api: {}", model.api),
                    );
                }
            },
        };
        match self.resolve(model, options) {
            Ok(options) => api.stream_simple(model, context, options),
            Err(message) => error_stream(model, message),
        }
    }

    fn resolve(
        &self,
        model: &Model,
        mut options: SimpleStreamOptions,
    ) -> Result<SimpleStreamOptions, String> {
        let mut env = BTreeMap::new();
        if let Some(resolver) = &self.auth {
            let auth = resolver(model)?;
            env = auth.env;
            if options.api_key.is_none() {
                options.api_key = auth.api_key;
            }
            if options.base_url.is_none() {
                options.base_url = auth.base_url;
            }
            if !auth.headers.is_empty() {
                let mut headers = auth.headers;
                headers.extend(options.headers.take().unwrap_or_default());
                options.headers = Some(headers);
            }
        }
        if options.api_key.is_none() {
            options.api_key = super::auth::env_api_key(&model.provider, &env);
        }
        Ok(options)
    }
}

impl Models for Providers {
    fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        if let Some(lookup) = &self.lookup {
            return lookup(provider, id);
        }
        self.models
            .iter()
            .find(|model| model.provider == provider && model.id == id)
            .cloned()
    }

    fn stream_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        self.stream(model, context, options, Origin::AgentLoop)
    }

    fn complete_simple(
        &self,
        model: &Model,
        context: TranscriptContext,
        options: SimpleStreamOptions,
    ) -> Result<AssistantMessage, ModelsError> {
        let stream = self.stream(model, context, options, Origin::Direct);
        while stream.next_event().is_some() {}
        stream
            .result()
            .ok_or_else(|| ModelsError("stream ended without a final message".to_owned()))
    }

    fn fetch_deferred(
        &self,
        model: &Model,
        _handle: &DeferredHandle,
        _signal: &CancelToken,
    ) -> Result<AssistantMessage, ModelsError> {
        Err(ModelsError(format!(
            "{}/{} does not support deferred responses",
            model.provider, model.id
        )))
    }

    fn cancel_deferred(
        &self,
        model: &Model,
        _handle: &DeferredHandle,
        _signal: &CancelToken,
    ) -> Result<(), ModelsError> {
        Err(ModelsError(format!(
            "{}/{} does not support deferred responses",
            model.provider, model.id
        )))
    }
}
