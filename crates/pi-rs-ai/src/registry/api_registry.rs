//! Port of `api-registry.ts` — the API-provider registry mapping open
//! api strings to stream functions (no closed enums at the seam: hosts
//! dispatch on registered string keys, never match on variants).
//!
//! The spec's registry is a module-level `Map`; registrations are
//! process-wide. `Map.set` semantics preserved: re-registering an api
//! replaces the entry but keeps its insertion position.
//!
//! Signature note (divergence-in-time, recorded in PLAN.md): the spec's
//! registry-level `stream` accepts any provider-specific options object
//! (`ProviderStreamOptions` union) via TS structural typing; here it
//! takes the shared [`StreamOptions`] until the Lua stream binding lands
//! and per-protocol knobs cross the bridge as tables.

use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use pi_rs_ai_types::{Context, Model};

use crate::protocols::{ProtocolError, SimpleStreamOptions, StreamOptions};
use crate::transport::event_stream::AssistantMessageEventStream;

/// Spec: `ApiStreamFunction`.
pub type ApiStreamFn = Arc<
    dyn Fn(
            &Model,
            &Context,
            Option<StreamOptions>,
        ) -> Result<AssistantMessageEventStream, ProtocolError>
        + Send
        + Sync,
>;

/// Spec: `ApiStreamSimpleFunction`.
pub type ApiStreamSimpleFn = Arc<
    dyn Fn(
            &Model,
            &Context,
            Option<SimpleStreamOptions>,
        ) -> Result<AssistantMessageEventStream, ProtocolError>
        + Send
        + Sync,
>;

/// Spec: `ApiProvider` — one api family's stream functions.
#[derive(Clone)]
pub struct ApiProvider {
    pub api: String,
    pub stream: ApiStreamFn,
    pub stream_simple: ApiStreamSimpleFn,
}

/// Spec: `RegisteredApiProvider` (provider + optional source id for
/// bulk unregistration).
struct Registered {
    provider: ApiProvider,
    source_id: Option<String>,
}

static REGISTRY: LazyLock<Mutex<Vec<Registered>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn registry() -> MutexGuard<'static, Vec<Registered>> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Spec: `wrapStream`/`wrapStreamSimple` — guard against calling a
/// provider with a model of a different api family.
fn mismatch_guard(model: &Model, api: &str) -> Result<(), ProtocolError> {
    if model.api == api {
        Ok(())
    } else {
        Err(ProtocolError(format!(
            "Mismatched api: {} expected {}",
            model.api, api
        )))
    }
}

/// Spec: `registerApiProvider(provider, sourceId?)`.
pub fn register_api_provider(provider: ApiProvider, source_id: Option<&str>) {
    let api = provider.api.clone();
    let inner_stream = Arc::clone(&provider.stream);
    let inner_simple = Arc::clone(&provider.stream_simple);
    let (guard_api, guard_api_simple) = (api.clone(), api.clone());
    let wrapped = ApiProvider {
        api: api.clone(),
        stream: Arc::new(move |model, context, options| {
            mismatch_guard(model, &guard_api)?;
            inner_stream(model, context, options)
        }),
        stream_simple: Arc::new(move |model, context, options| {
            mismatch_guard(model, &guard_api_simple)?;
            inner_simple(model, context, options)
        }),
    };
    let entry = Registered {
        provider: wrapped,
        source_id: source_id.map(str::to_owned),
    };
    let mut reg = registry();
    if let Some(existing) = reg.iter_mut().find(|r| r.provider.api == api) {
        *existing = entry;
    } else {
        reg.push(entry);
    }
}

/// Spec: `getApiProvider(api)`.
pub fn get_api_provider(api: &str) -> Option<ApiProvider> {
    registry()
        .iter()
        .find(|r| r.provider.api == api)
        .map(|r| r.provider.clone())
}

/// Spec: `getApiProviders()` — registration (insertion) order.
pub fn get_api_providers() -> Vec<ApiProvider> {
    registry().iter().map(|r| r.provider.clone()).collect()
}

/// Spec: `unregisterApiProviders(sourceId)`.
pub fn unregister_api_providers(source_id: &str) {
    registry().retain(|r| r.source_id.as_deref() != Some(source_id));
}

/// Spec: `clearApiProviders()`.
pub fn clear_api_providers() {
    registry().clear();
}
