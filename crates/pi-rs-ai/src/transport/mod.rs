//! Transport — HTTP + SSE decode + retry + cancellation, written once.
//!
//! pi delegates most transport to provider SDKs; its only first-party
//! transport is the hand-rolled fetch/SSE/retry in
//! `providers/openai-codex-responses.ts`. That file is the spec for this
//! module; the SDK-hidden behavior is pinned later by recorded-fixture
//! replay (WS2.3). Provenance:
//!
//! - [`event_stream`] ← `utils/event-stream.ts` (`EventStream`,
//!   `AssistantMessageEventStream`)
//! - [`abort`] ← `utils/abort-signals.ts` — `combineAbortSignals` becomes
//!   `tokio::select!` at await sites (no listener registration in Rust);
//!   the one internal combination (user signal + header timeout) lives in
//!   [`http`].
//! - [`sse`] ← `openai-codex-responses.ts` `parseSSE`, generalized with
//!   the SSE-standard `event:` field (needed by anthropic-messages;
//!   still zero protocol knowledge). `[DONE]` / empty-data filtering is
//!   the protocol layer's, as in the spec.
//! - [`retry`] ← `openai-codex-responses.ts` retry helpers
//!   (`isRetryableError`, `getRetryAfterDelayMs`, `capRetryDelayMs`,
//!   backoff constants, abortable `sleep`).
//! - [`http`] ← `openai-codex-responses.ts` fetch-with-retry loop +
//!   SSE header timeout.
//!
//! Compression notes (locked `pi-rs-ai` row):
//! - `utils/node-http-proxy.ts` → reqwest's built-in env-proxy support
//!   (`http_proxy`/`https_proxy`/`no_proxy`); Node needs the hand-rolled
//!   agent, Rust does not. SOCKS URLs error (feature not enabled),
//!   matching the spec's rejection in spirit.
//! - The spec's `onResponse`/`onPayload` stream-option hooks are
//!   protocol-layer surface; they land with WS2.3.
//! - Codex's `parseErrorResponse` friendly-message shaping is
//!   protocol-specific; transport returns [`TransportError::Status`]
//!   with the raw body and lets protocols format.

pub mod abort;
pub mod event_stream;
pub mod http;
pub mod retry;
pub mod sse;

pub use abort::AbortSignal;
pub use event_stream::{
    AssistantMessageEventStream, EventStream, create_assistant_message_event_stream,
};
pub use http::{RetryOptions, RetryPolicy, post_with_retry, response_sse_reader};
pub use sse::{SseDecoder, SseEvent, SseReader};

/// Typed transport errors. `Display` strings are part of the parity
/// surface where the spec matches on them: [`TransportError::Aborted`]
/// renders exactly `"Request was aborted"`, and the retry loop's
/// usage-limit check scans `Display` output (spec:
/// `lastError.message.includes("usage limit")`).
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Spec: `new Error("Request was aborted")`.
    #[error("Request was aborted")]
    Aborted,
    /// Abort raced against an in-flight body read. In pi the pending
    /// `reader.read()` rejects with undici's `DOMException` when the
    /// fetch is cancelled, so the surfaced message differs from the
    /// loop-top signal check (pinned by the anthropic-parity oracle's
    /// `abort-mid-stream` case).
    #[error("This operation was aborted")]
    BodyAborted,
    /// Spec: SSE response-header timeout (`createSSEHeaderTimeout`).
    #[error("SSE response headers timed out after {0}ms")]
    HeaderTimeout(u64),
    /// Non-2xx HTTP response that was not retried (or exhausted retries).
    /// Protocols shape the friendly message from the raw body.
    #[error("HTTP {status} {status_text}: {body}")]
    Status {
        status: u16,
        status_text: String,
        body: String,
    },
    /// Network / decode failure below HTTP semantics.
    #[error("{0}")]
    Network(String),
}
