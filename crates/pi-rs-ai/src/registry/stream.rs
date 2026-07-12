//! Port of `stream.ts` — resolution through the API-provider registry
//! with env API-key injection — plus `providers/register-builtins.ts`.
//!
//! Compression notes:
//! - The spec registers all nine api families at module import, each
//!   behind a lazy dynamic `import()` (a JS bundling concern with no
//!   Rust analogue — protocols are compiled in). Landed families register
//!   here; unresolved families retain Pi's exact registry error.
//! - Import side effects don't exist in Rust: the entry points ensure
//!   the one-time builtin registration themselves ([`ensure_builtins`]).
//! - `complete`/`completeSimple` await the stream result; where the
//!   spec's promise would pend forever on a result-less stream (see the
//!   `event_stream` divergence note), these return a [`ProtocolError`].

use std::sync::Once;

use pi_rs_ai_types::{AssistantMessage, Context, Model};

use crate::protocols::anthropic::{AnthropicOptions, stream_anthropic, stream_simple_anthropic};
use crate::protocols::openai_completions::{
    OpenAICompletionsOptions, stream_openai_completions, stream_simple_openai_completions,
};
use crate::protocols::openai_responses::{
    OpenAIResponsesOptions, stream_openai_responses, stream_simple_openai_responses,
};
use crate::protocols::{ProtocolError, SimpleStreamOptions, StreamOptions};
use crate::registry::api_registry::{
    ApiProvider, clear_api_providers, get_api_provider, register_api_provider,
};
use crate::registry::env_api_keys::get_env_api_key;
use crate::transport::event_stream::AssistantMessageEventStream;

use std::sync::Arc;

/// Spec: `registerBuiltInApiProviders()` — the landed api families
/// (`register-builtins.ts`; remaining families register in WS5).
pub fn register_builtin_api_providers() {
    register_api_provider(
        ApiProvider {
            api: "anthropic-messages".to_owned(),
            stream: Arc::new(|model, context, options| {
                let options = options.map(|base| AnthropicOptions {
                    base,
                    ..AnthropicOptions::default()
                });
                Ok(stream_anthropic(model, context, options))
            }),
            stream_simple: Arc::new(stream_simple_anthropic),
        },
        None,
    );
    register_api_provider(
        ApiProvider {
            api: "openai-completions".to_owned(),
            stream: Arc::new(|model, context, options| {
                let options = options.map(|base| OpenAICompletionsOptions {
                    base,
                    ..OpenAICompletionsOptions::default()
                });
                Ok(stream_openai_completions(model, context, options))
            }),
            stream_simple: Arc::new(stream_simple_openai_completions),
        },
        None,
    );
    register_api_provider(
        ApiProvider {
            api: "openai-responses".to_owned(),
            stream: Arc::new(|model, context, options| {
                let options = options.map(|base| OpenAIResponsesOptions {
                    base,
                    ..OpenAIResponsesOptions::default()
                });
                Ok(stream_openai_responses(model, context, options))
            }),
            stream_simple: Arc::new(stream_simple_openai_responses),
        },
        None,
    );
}

/// Spec: `resetApiProviders()` (`register-builtins.ts`) — clear dynamic
/// registrations and restore the builtins.
pub fn reset_api_providers() {
    clear_api_providers();
    register_builtin_api_providers();
}

/// The spec's module-import side effect (`import "./register-builtins"`),
/// made explicit: runs once, ever — a later `clear_api_providers` is
/// respected, exactly as in the spec.
fn ensure_builtins() {
    static INIT: Once = Once::new();
    INIT.call_once(register_builtin_api_providers);
}

/// Spec: `hasExplicitApiKey`.
fn has_explicit_api_key(api_key: Option<&str>) -> bool {
    api_key.is_some_and(|key| !key.trim().is_empty())
}

/// Spec: `withEnvApiKey` — inject the provider's env API key unless the
/// caller passed an explicit one.
fn with_env_api_key(model: &Model, options: Option<StreamOptions>) -> Option<StreamOptions> {
    if has_explicit_api_key(options.as_ref().and_then(|o| o.api_key.as_deref())) {
        return options;
    }
    let Some(api_key) = get_env_api_key(&model.provider) else {
        return options;
    };
    let mut options = options.unwrap_or_default();
    options.api_key = Some(api_key);
    Some(options)
}

/// [`with_env_api_key`] for the simple-options family (the spec's one
/// generic function, split by Rust's two concrete types).
fn with_env_api_key_simple(
    model: &Model,
    options: Option<SimpleStreamOptions>,
) -> Option<SimpleStreamOptions> {
    if has_explicit_api_key(options.as_ref().and_then(|o| o.base.api_key.as_deref())) {
        return options;
    }
    let Some(api_key) = get_env_api_key(&model.provider) else {
        return options;
    };
    let mut options = options.unwrap_or_default();
    options.base.api_key = Some(api_key);
    Some(options)
}

/// Spec: `resolveApiProvider`.
fn resolve_api_provider(api: &str) -> Result<ApiProvider, ProtocolError> {
    get_api_provider(api)
        .ok_or_else(|| ProtocolError(format!("No API provider registered for api: {api}")))
}

/// Spec: `stream(model, context, options?)`.
pub fn stream(
    model: &Model,
    context: &Context,
    options: Option<StreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    ensure_builtins();
    let provider = resolve_api_provider(&model.api)?;
    (provider.stream)(model, context, with_env_api_key(model, options))
}

/// Spec: `complete(model, context, options?)`.
pub async fn complete(
    model: &Model,
    context: &Context,
    options: Option<StreamOptions>,
) -> Result<AssistantMessage, ProtocolError> {
    let s = stream(model, context, options)?;
    s.result()
        .await
        .ok_or_else(|| ProtocolError("event stream completed without a result".to_owned()))
}

/// Spec: `streamSimple(model, context, options?)`.
pub fn stream_simple(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessageEventStream, ProtocolError> {
    ensure_builtins();
    let provider = resolve_api_provider(&model.api)?;
    (provider.stream_simple)(model, context, with_env_api_key_simple(model, options))
}

/// Spec: `completeSimple(model, context, options?)`.
pub async fn complete_simple(
    model: &Model,
    context: &Context,
    options: Option<SimpleStreamOptions>,
) -> Result<AssistantMessage, ProtocolError> {
    let s = stream_simple(model, context, options)?;
    s.result()
        .await
        .ok_or_else(|| ProtocolError("event stream completed without a result".to_owned()))
}
