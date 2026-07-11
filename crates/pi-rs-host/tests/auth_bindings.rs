//! Public Lua seam exercisers for the `pi.auth` mechanism bindings:
//! credential storage over auth.json and the login-flow channel bridge.
//!
//! This file is its own test binary: it owns the process-global
//! `PI_CODING_AGENT_DIR` and the OAuth provider registry.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pi_rs_ai_auth::{
    AuthFuture, OAuthCredentials, OAuthLoginCallbacks, OAuthProviderInterface,
    register_oauth_provider,
};
use pi_rs_host::{Host, HostConfig};

/// Scripted provider: emits one auth URL, prompts once, then succeeds
/// (or fails with the prompt's rejection).
struct ScriptedProvider;

impl OAuthProviderInterface for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted-oauth"
    }

    fn name(&self) -> &str {
        "Scripted OAuth"
    }

    fn login<'a>(
        &'a self,
        callbacks: &'a dyn OAuthLoginCallbacks,
    ) -> AuthFuture<'a, OAuthCredentials> {
        Box::pin(async move {
            callbacks.on_auth(pi_rs_ai_auth::OAuthAuthInfo {
                url: "https://example.invalid/authorize".into(),
                instructions: Some("scripted instructions".into()),
            });
            let code = callbacks
                .on_prompt(pi_rs_ai_auth::OAuthPrompt {
                    message: "Paste the code:".into(),
                    placeholder: None,
                    allow_empty: false,
                })
                .await?;
            callbacks.on_progress("Exchanging...");
            Ok(OAuthCredentials {
                refresh: format!("refresh-{code}"),
                access: format!("access-{code}"),
                expires: pi_rs_ai_types::now_ms() + 60_000,
                extra: serde_json::Map::new(),
            })
        })
    }

    fn refresh_token<'a>(
        &'a self,
        credentials: &'a OAuthCredentials,
    ) -> AuthFuture<'a, OAuthCredentials> {
        let refreshed = credentials.clone();
        Box::pin(async move { Ok(refreshed) })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }
}

fn host() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/auth-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    host
}

#[test]
fn auth_bindings_round_trip_through_the_public_surface() {
    let agent_dir = tempfile::tempdir().unwrap();
    // SAFETY: single test binary; set before any Host is created.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    register_oauth_provider(Arc::new(ScriptedProvider));

    let host = host();
    let result = host
        .call_command(
            "auth-demo",
            &serde_json::json!({ "provider": "demo", "key": "sk-1", "value": "plain" }).to_string(),
        )
        .unwrap()
        .unwrap();

    assert_eq!(
        result["stored"],
        serde_json::json!({ "type": "api_key", "key": "sk-1" })
    );
    assert_eq!(result["status"]["configured"], true);
    assert_eq!(result["status"]["source"], "stored");
    assert_eq!(result["listed"], serde_json::json!(["demo"]));
    assert_eq!(result["had"], true);
    assert_eq!(result["removed"], true);
    assert_eq!(result["resolved"], "plain");
    let auth_path = agent_dir.path().join("auth.json");
    assert_eq!(result["auth_path"], auth_path.to_string_lossy().as_ref());
    // The set + remove persisted through the file backend.
    let disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(disk, serde_json::json!({}));
    // The registry mirror includes all pinned subscription providers plus the
    // scripted extension provider, in registration order.
    let oauth = result["oauth"].as_array().unwrap();
    assert_eq!(
        oauth,
        serde_json::json!([
            "anthropic",
            "github-copilot",
            "openai-codex",
            "scripted-oauth"
        ])
        .as_array()
        .unwrap()
    );

    // Login bridge: auth event -> prompt -> respond -> progress -> done,
    // persisting the credential (spec authStorage.login).
    let flow = r#"
      local pi = ...
      pi.register_command("auth-login-script", {
        handler = function(args)
          local request = pi.json.decode(args)
          local handle = pi.auth.login_start(request.provider)
          local events = {}
          while true do
            local event = handle:next_event(5000)
            if event == nil then break end
            events[#events + 1] = event.type
            if event.type == "prompt" then
              if request.cancel then handle:cancel() else handle:respond("c0de") end
            elseif event.type == "done" or event.type == "error" then
              if event.type == "error" then events[#events + 1] = event.message end
              break
            end
          end
          return { events = events, credential = pi.auth.get(request.provider) }
        end,
      })
    "#;
    host.load("<auth-login-script>", flow).unwrap();

    let result = host
        .call_command(
            "auth-login-script",
            &serde_json::json!({ "provider": "scripted-oauth" }).to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        result["events"],
        serde_json::json!(["auth", "prompt", "progress", "done"])
    );
    assert_eq!(result["credential"]["type"], "oauth");
    assert_eq!(result["credential"]["access"], "access-c0de");
    assert_eq!(result["credential"]["refresh"], "refresh-c0de");

    // Cancel while a prompt is pending rejects with the spec's
    // "Login cancelled" message and persists nothing new.
    host.call_command(
        "auth-demo",
        &serde_json::json!({ "provider": "scripted-oauth" }).to_string(),
    )
    .unwrap();
    let result = host
        .call_command(
            "auth-login-script",
            &serde_json::json!({ "provider": "scripted-oauth", "cancel": true }).to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        result["events"],
        serde_json::json!(["auth", "prompt", "error", "Login cancelled"])
    );
    assert_eq!(result["credential"], serde_json::Value::Null);

    // Unknown providers surface the spec's error message.
    let result = host
        .call_command(
            "auth-login-script",
            &serde_json::json!({ "provider": "nope" }).to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        result["events"],
        serde_json::json!(["error", "Unknown OAuth provider: nope"])
    );
}
