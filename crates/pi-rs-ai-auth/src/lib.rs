//! pi-rs-ai-auth — the `packages/ai` OAuth port (spec: `ref/pi` @
//! `c5582102`, pi v0.79.0, `src/utils/oauth/`).
//!
//! Structure follows the locked `pi-rs-ai` decision row: one PKCE
//! authorization-code engine ([`engine`]) with flows as data rows —
//! anthropic ([`anthropic`]) is the first user. The device-code engine
//! and the irreducibly weird flows (codex exchange, copilot) arrive
//! with WS5 breadth, sharing this machinery. Resurrected from the attic
//! (`rebuild` @ `e8cb418`, `pi-rs-ai-auth`) and reshaped to the spec's
//! surface, messages, and pages.
//!
//! Layering (locked): `types → auth → transport → …` — this crate sits
//! below `pi-rs-ai`'s transport, so its token POST is the spec's plain
//! 30s `postJson`, not the retrying stream transport.

mod anthropic;
mod callback_server;
mod engine;
mod error;
mod oauth_page;
mod pkce;
mod registry;
mod types;

pub use anthropic::{anthropic_flow, login_anthropic, refresh_anthropic_token};
pub use callback_server::{CallbackCode, CallbackPages, CallbackServer};
pub use engine::{PkceFlow, login_pkce, parse_authorization_input, refresh_pkce};
pub use error::AuthError;
pub use oauth_page::{oauth_error_html, oauth_success_html};
pub use pkce::{Pkce, challenge_for, generate_pkce};
pub use registry::{
    OAuthApiKeyResult, get_oauth_api_key, get_oauth_provider, get_oauth_providers,
    register_oauth_provider, reset_oauth_providers, unregister_oauth_provider,
};
pub use types::{
    AuthFuture, OAuthAuthInfo, OAuthCredentials, OAuthDeviceCodeInfo, OAuthLoginCallbacks,
    OAuthPrompt, OAuthProviderId, OAuthProviderInterface, OAuthSelectOption, OAuthSelectPrompt,
};
