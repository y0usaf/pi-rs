//! Port of `utils/oauth/anthropic.ts` — the Anthropic (Claude Pro/Max)
//! OAuth flow as a [`PkceFlow`] data row over the shared engine.
//!
//! The spec obfuscates the client id with a base64 decode; the decoded
//! value is used directly here.

use crate::callback_server::CallbackPages;
use crate::engine::{PkceFlow, login_pkce, refresh_pkce};
use crate::error::AuthError;
use crate::types::{AuthFuture, OAuthCredentials, OAuthLoginCallbacks, OAuthProviderInterface};

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CALLBACK_PORT: u16 = 53692;
const CALLBACK_PATH: &str = "/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

/// The anthropic flow row (spec: the module-level constants plus
/// `anthropicOAuthProvider`).
pub fn anthropic_flow() -> PkceFlow {
    PkceFlow {
        id: "anthropic".into(),
        name: "Anthropic (Claude Pro/Max)".into(),
        error_label: "Anthropic".into(),
        client_id: CLIENT_ID.into(),
        authorize_url: AUTHORIZE_URL.into(),
        token_url: TOKEN_URL.into(),
        callback_port: CALLBACK_PORT,
        callback_path: CALLBACK_PATH.into(),
        scopes: SCOPES.into(),
        extra_auth_params: vec![("code".into(), "true".into())],
        instructions:
            "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here."
                .into(),
        pages: CallbackPages {
            success: "Anthropic authentication completed. You can close this window.".into(),
            denied: "Anthropic authentication did not complete.".into(),
        },
    }
}

/// Spec: `loginAnthropic(options)`.
pub async fn login_anthropic(
    callbacks: &dyn OAuthLoginCallbacks,
) -> Result<OAuthCredentials, AuthError> {
    login_pkce(&anthropic_flow(), callbacks).await
}

/// Spec: `refreshAnthropicToken(refreshToken)`.
pub async fn refresh_anthropic_token(refresh_token: &str) -> Result<OAuthCredentials, AuthError> {
    refresh_pkce(&anthropic_flow(), refresh_token).await
}

// Spec: `anthropicOAuthProvider` — a PKCE flow row *is* a provider.
impl OAuthProviderInterface for PkceFlow {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn uses_callback_server(&self) -> bool {
        true
    }

    fn login<'a>(
        &'a self,
        callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(login_pkce(self, callbacks))
    }

    fn refresh_token<'a>(
        &'a self,
        credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(refresh_pkce(self, &credentials.refresh))
    }

    /// Spec: `getApiKey(credentials) { return credentials.access }`.
    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }
}
