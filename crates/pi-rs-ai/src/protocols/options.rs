//! Runtime stream options — the non-serde half of the spec's `types.ts`
//! options families (the serde-able pieces live in `pi-rs-ai-types`).
//!
//! The spec's `onPayload`/`onResponse` hooks may be async; here they are
//! synchronous callbacks — the coroutine-awaitable form arrives with the
//! Lua provider binding (WS2.4) if a translation needs it.

use std::collections::BTreeMap;
use std::sync::Arc;

use pi_rs_ai_types::{
    CacheRetention, Model, ProviderResponse, ThinkingBudgets, ThinkingLevel, Transport,
};
use serde_json::Value;

use crate::transport::AbortSignal;

/// Spec: `StreamOptions["onPayload"]` — inspect or replace the provider
/// payload before sending; `None` keeps it unchanged.
pub type PayloadHook = Arc<dyn Fn(Value, &Model) -> Option<Value> + Send + Sync>;

/// Spec: `StreamOptions["onResponse"]` — invoked after HTTP response
/// headers arrive, before the body stream is consumed.
pub type ResponseHook = Arc<dyn Fn(&ProviderResponse, &Model) + Send + Sync>;

/// Spec: `StreamOptions`.
#[derive(Clone, Default)]
pub struct StreamOptions {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub signal: Option<AbortSignal>,
    pub api_key: Option<String>,
    /// Preferred transport for providers that support multiple.
    pub transport: Option<Transport>,
    /// Prompt-cache retention preference (default "short").
    pub cache_retention: Option<CacheRetention>,
    /// Session identifier for providers with session-based caching.
    pub session_id: Option<String>,
    pub on_payload: Option<PayloadHook>,
    pub on_response: Option<ResponseHook>,
    /// Custom headers, merged over provider defaults.
    pub headers: Option<BTreeMap<String, String>>,
    /// HTTP request timeout (SDK clients default to 10 minutes).
    pub timeout_ms: Option<u64>,
    pub websocket_connect_timeout_ms: Option<u64>,
    /// Client-side retry attempts (default 0).
    pub max_retries: Option<u32>,
    /// Cap on server-requested retry delays (default 60 000; 0 disables).
    pub max_retry_delay_ms: Option<f64>,
    /// Provider-specific request metadata (e.g. Anthropic `user_id`).
    pub metadata: Option<serde_json::Map<String, Value>>,
}

/// Spec: `SimpleStreamOptions` (`StreamOptions` + reasoning knobs).
#[derive(Clone, Default)]
pub struct SimpleStreamOptions {
    pub base: StreamOptions,
    pub reasoning: Option<ThinkingLevel>,
    /// Custom token budgets for thinking levels (token-based providers).
    pub thinking_budgets: Option<ThinkingBudgets>,
}
