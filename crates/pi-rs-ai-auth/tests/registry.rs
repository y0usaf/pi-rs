//! Parity tests for the provider registry + `getOAuthApiKey` (spec:
//! `utils/oauth/index.ts`). The registry is process-global (the spec's
//! module-level Map), so all mutation lives in one test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;

use pi_rs_ai_auth::{
    AuthError, AuthFuture, OAuthCredentials, OAuthLoginCallbacks, OAuthProviderInterface,
    get_oauth_api_key, get_oauth_provider, get_oauth_providers, register_oauth_provider,
    reset_oauth_providers, unregister_oauth_provider,
};

/// A custom provider double: refresh returns fixed credentials or a
/// canned failure; api key derives from the access token.
struct FakeProvider {
    id: String,
    refresh_fails: bool,
}

impl OAuthProviderInterface for FakeProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Fake Provider"
    }

    fn login<'a>(
        &'a self,
        _callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(async { Err(AuthError::Cancelled) })
    }

    fn refresh_token<'a>(
        &'a self,
        _credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials> {
        let fails = self.refresh_fails;
        Box::pin(async move {
            if fails {
                Err(AuthError::Message("boom".into()))
            } else {
                Ok(creds("refreshed-access", i64::MAX))
            }
        })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        format!("key-{}", credentials.access)
    }
}

fn creds(access: &str, expires: i64) -> OAuthCredentials {
    OAuthCredentials {
        refresh: "r".into(),
        access: access.into(),
        expires,
        extra: serde_json::Map::new(),
    }
}

#[tokio::test]
async fn registry_and_get_oauth_api_key() {
    // Spec built-ins, in `BUILT_IN_OAUTH_PROVIDERS` order.
    let ids: Vec<String> = get_oauth_providers()
        .iter()
        .map(|p| p.id().to_owned())
        .collect();
    assert_eq!(ids, ["anthropic", "github-copilot", "openai-codex"]);
    let anthropic = get_oauth_provider("anthropic").unwrap();
    assert_eq!(anthropic.name(), "Anthropic (Claude Pro/Max)");
    assert!(anthropic.uses_callback_server());
    assert_eq!(anthropic.get_api_key(&creds("tok", 0)), "tok");
    assert_eq!(
        get_oauth_provider("github-copilot").unwrap().name(),
        "GitHub Copilot"
    );
    let codex = get_oauth_provider("openai-codex").unwrap();
    assert_eq!(codex.name(), "ChatGPT Plus/Pro (Codex Subscription)");
    assert!(codex.uses_callback_server());

    // Register a custom provider; appears in insertion order.
    register_oauth_provider(Arc::new(FakeProvider {
        id: "fake".into(),
        refresh_fails: false,
    }));
    let ids: Vec<String> = get_oauth_providers()
        .iter()
        .map(|p| p.id().to_owned())
        .collect();
    assert_eq!(ids, ["anthropic", "github-copilot", "openai-codex", "fake"]);

    // Re-registering a built-in id replaces in place (Map.set).
    register_oauth_provider(Arc::new(FakeProvider {
        id: "anthropic".into(),
        refresh_fails: false,
    }));
    let replaced = get_oauth_provider("anthropic").unwrap();
    assert_eq!(replaced.name(), "Fake Provider");
    assert_eq!(get_oauth_providers()[0].id(), "anthropic");

    // Unregistering a built-in restores the built-in implementation.
    unregister_oauth_provider("anthropic");
    assert_eq!(
        get_oauth_provider("anthropic").unwrap().name(),
        "Anthropic (Claude Pro/Max)"
    );

    // Unregistering a custom provider removes it completely.
    unregister_oauth_provider("fake");
    assert!(get_oauth_provider("fake").is_none());

    // getOAuthApiKey: unknown provider → spec error.
    let empty = HashMap::new();
    let err = get_oauth_api_key("nope", &empty).await.unwrap_err();
    assert_eq!(err.to_string(), "Unknown OAuth provider: nope");

    // No stored credentials → None.
    register_oauth_provider(Arc::new(FakeProvider {
        id: "fake".into(),
        refresh_fails: false,
    }));
    assert!(get_oauth_api_key("fake", &empty).await.unwrap().is_none());

    // Unexpired credentials pass through untouched.
    let mut stored = HashMap::new();
    stored.insert("fake".to_owned(), creds("live-access", i64::MAX));
    let result = get_oauth_api_key("fake", &stored).await.unwrap().unwrap();
    assert_eq!(result.api_key, "key-live-access");
    assert_eq!(result.new_credentials.access, "live-access");

    // Expired credentials are refreshed.
    stored.insert("fake".to_owned(), creds("stale-access", 0));
    let result = get_oauth_api_key("fake", &stored).await.unwrap().unwrap();
    assert_eq!(result.api_key, "key-refreshed-access");
    assert_eq!(result.new_credentials.access, "refreshed-access");

    // Refresh failure → spec's wrapped message.
    register_oauth_provider(Arc::new(FakeProvider {
        id: "fake".into(),
        refresh_fails: true,
    }));
    let err = get_oauth_api_key("fake", &stored).await.unwrap_err();
    assert_eq!(err.to_string(), "Failed to refresh OAuth token for fake");

    // Reset restores the built-in set.
    reset_oauth_providers();
    let ids: Vec<String> = get_oauth_providers()
        .iter()
        .map(|p| p.id().to_owned())
        .collect();
    assert_eq!(ids, ["anthropic", "github-copilot", "openai-codex"]);
}

#[test]
fn credentials_round_trip_with_extra_fields() {
    // Spec: OAuthCredentials carries an open index signature.
    let json = r#"{"refresh":"r1","access":"a1","expires":123,"enterpriseDomain":"x.ghe.com"}"#;
    let credentials: OAuthCredentials = serde_json::from_str(json).unwrap();
    assert_eq!(credentials.access, "a1");
    assert_eq!(
        credentials.extra.get("enterpriseDomain").unwrap(),
        "x.ghe.com"
    );
    let back = serde_json::to_value(&credentials).unwrap();
    assert_eq!(
        back,
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
}
