//! Protocol layer — wire mapping per API family (spec: `ref/pi` @
//! `c5582102`, pi v0.79.0), per the locked `pi-rs-ai` structure row:
//! protocols own request shaping + stream-event mapping and nothing
//! else; transport mechanics live in [`crate::transport`], catalog and
//! resolution land with the registry (WS2.4).
//!
//! Provenance:
//! - [`options`] — the runtime half of the spec's `types.ts` options
//!   families (`StreamOptions`, `SimpleStreamOptions`; signals, hooks).
//! - [`simple_options`] ← `providers/simple-options.ts`.
//! - [`transform_messages`] ← `providers/transform-messages.ts`.
//! - [`cloudflare`] ← `providers/cloudflare.ts`.
//! - [`copilot_headers`] ← `providers/github-copilot-headers.ts`.
//! - [`anthropic`] ← `providers/anthropic.ts`.
//! - [`azure_openai_responses`] ← `providers/azure-openai-responses.ts`.
//! - [`google`] + [`google_shared`] ← Google Generative AI wire protocol.
//! - [`google_vertex`] ← Google Vertex request/auth policy over the shared wire path.
//! - [`openai_prompt_cache`] ← `providers/openai-prompt-cache.ts`.
//! - [`openai_completions`] ← `providers/openai-completions.ts`.
//! - [`openai_responses`] ← `providers/openai-responses.ts` + shared mapping.
//! - [`openai_codex_responses`] ← Codex Responses SSE/WebSocket + continuation.
//!
//! Remaining protocol families land in PLAN item 8 slices.

pub mod anthropic;
pub mod azure_openai_responses;
pub mod bedrock;
pub mod cloudflare;
pub mod copilot_headers;
pub mod google;
pub(crate) mod google_shared;
pub mod google_vertex;
pub mod mistral;
pub mod openai_codex_responses;
pub mod openai_completions;
pub mod openai_prompt_cache;
pub mod openai_responses;
pub mod options;
pub mod simple_options;
pub mod transform_messages;

pub use options::{PayloadHook, ResponseHook, SimpleStreamOptions, StreamOptions};

use std::collections::BTreeMap;

use pi_rs_ai_types::CacheRetention;

/// Protocol-layer error. The spec throws `Error(message)` and its stream
/// functions fold every failure into `AssistantMessage.errorMessage`;
/// the message string is the parity surface, so this is a typed wrapper
/// around exactly that string.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProtocolError(pub String);

/// Spec: `resolveCacheRetention` — default "short",
/// `PI_CACHE_RETENTION=long` for backward compatibility. Each spec
/// provider file repeats this verbatim; written once here.
pub(crate) fn resolve_cache_retention(cache_retention: Option<CacheRetention>) -> CacheRetention {
    if let Some(retention) = cache_retention {
        return retention;
    }
    if std::env::var("PI_CACHE_RETENTION").as_deref() == Ok("long") {
        return CacheRetention::Long;
    }
    CacheRetention::Short
}

/// Case-insensitive header upsert (JS object spread over lowercased
/// header names).
pub(crate) fn merge_header(headers: &mut Vec<(String, String)>, key: &str, value: &str) {
    let lower = key.to_ascii_lowercase();
    if let Some(entry) = headers.iter_mut().find(|(k, _)| *k == lower) {
        entry.1 = value.to_string();
    } else {
        headers.push((lower, value.to_string()));
    }
}

/// Merge a header record over `headers` ([`merge_header`] per entry).
pub(crate) fn merge_header_map(
    headers: &mut Vec<(String, String)>,
    map: Option<&BTreeMap<String, String>>,
) {
    if let Some(map) = map {
        for (key, value) in map {
            merge_header(headers, key, value);
        }
    }
}
