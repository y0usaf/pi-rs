//! pi-rs-ai-auth — the `packages/ai` OAuth port (spec: `ref/pi` @
//! `c5582102`, pi v0.79.0, `src/utils/oauth/`).
//!
//! Structure follows the locked `pi-rs-ai` decision row: one PKCE
//! authorization-code engine ([`engine`]), one RFC 8628 polling engine, and
//! provider-specific code only where the wire flow is irreducibly different.
//! Anthropic, GitHub Copilot, and OpenAI Codex match the pinned built-in OAuth
//! registry. Resurrected from the attic (`rebuild` @ `e8cb418`,
//! `pi-rs-ai-auth`) and reshaped to the spec's surface, messages, and pages.
//!
//! Layering (locked): `types → auth → transport → …` — this crate sits
//! below `pi-rs-ai`'s transport, so its token POST is the spec's plain
//! 30s `postJson`, not the retrying stream transport.

mod anthropic;
mod callback_server;
mod device_code;
mod engine;
mod error;
mod github_copilot;
mod oauth_page;
mod openai_codex;
mod pkce;
mod registry;
mod types;

pub use anthropic::{anthropic_flow, login_anthropic, refresh_anthropic_token};
pub use callback_server::{CallbackCode, CallbackPages, CallbackServer};
pub use device_code::{DeviceCodePoll, poll_device_code};
pub use engine::{PkceFlow, login_pkce, parse_authorization_input, refresh_pkce};
pub use error::AuthError;
pub use github_copilot::{
    GitHubCopilotEndpoints, GitHubCopilotFlow, github_copilot_base_url, github_copilot_flow,
    normalize_github_domain,
};
pub use oauth_page::{oauth_error_html, oauth_success_html};
pub use openai_codex::{
    OPENAI_CODEX_BROWSER_LOGIN_METHOD, OPENAI_CODEX_DEVICE_CODE_LOGIN_METHOD, OpenAiCodexEndpoints,
    OpenAiCodexFlow, openai_codex_flow,
};
pub use pkce::{Pkce, challenge_for, generate_pkce};
pub use registry::{
    OAuthApiKeyResult, get_oauth_api_key, get_oauth_provider, get_oauth_providers,
    register_oauth_provider, reset_oauth_providers, unregister_oauth_provider,
};
pub use types::{
    AuthFuture, OAuthAuthInfo, OAuthCredentials, OAuthDeviceCodeInfo, OAuthLoginCallbacks,
    OAuthPrompt, OAuthProviderId, OAuthProviderInterface, OAuthSelectOption, OAuthSelectPrompt,
};
