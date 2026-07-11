//! Registry layer — catalog as data + resolution (spec: `ref/pi` @
//! `c5582102`, pi v0.79.0), per the locked `pi-rs-ai` structure row: the
//! top of the `types → transport → protocols → registry` stack.
//!
//! Provenance:
//! - [`catalog`] ← the registry half of `models.ts`, with the catalog
//!   itself as generated data (`data/models.json` ←
//!   `models.generated.ts` via `scripts/gen-models-json.ts`) — the pure
//!   half (`calculateCost`, thinking levels) lives in `pi_rs_ai_types`.
//! - [`api_registry`] ← `api-registry.ts` — API stream providers keyed
//!   by open api strings (no closed enums at the seam).
//! - [`env_api_keys`] ← `env-api-keys.ts`.
//! - [`stream`] ← `stream.ts` + the landed half of
//!   `providers/register-builtins.ts` (anthropic-messages,
//!   openai-completions; the remaining api families register in WS5).

pub mod api_registry;
pub mod catalog;
pub mod env_api_keys;
pub mod stream;

pub use api_registry::{
    ApiProvider, ApiStreamFn, ApiStreamSimpleFn, clear_api_providers, get_api_provider,
    get_api_providers, register_api_provider, unregister_api_providers,
};
pub use catalog::{get_model, get_models, get_providers};
pub use env_api_keys::{find_env_keys, get_env_api_key};
pub use stream::{
    complete, complete_simple, register_builtin_api_providers, reset_api_providers, stream,
    stream_simple,
};
