//! `pi --login [provider]` — OAuth login for the bare core
//! (doctrine 06).
//!
//! pi's login lives in interactive `/login` (a TUI overlay); the bare
//! core has no frontend, so the spec's `OAuthLoginCallbacks` are bound
//! to stdio here. The interactive frontend pack supplies its own
//! callbacks in WS8 — this module is the degenerate frontend's pair of
//! `AuthStorage.login`.

use std::io::Write;

use pi_rs_ai_auth::{
    AuthFuture, OAuthAuthInfo, OAuthDeviceCodeInfo, OAuthLoginCallbacks, OAuthPrompt,
    OAuthSelectPrompt,
};

use crate::core::auth_storage::{AuthStorage, AuthStorageError};

fn read_stdin_line() -> Result<String, pi_rs_ai_auth::AuthError> {
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => Err(pi_rs_ai_auth::AuthError::Cancelled),
        Ok(_) => Ok(line.trim().to_owned()),
        Err(error) => Err(pi_rs_ai_auth::AuthError::Io(error)),
    }
}

async fn prompt_line() -> Result<String, pi_rs_ai_auth::AuthError> {
    tokio::task::spawn_blocking(read_stdin_line)
        .await
        .map_err(|e| pi_rs_ai_auth::AuthError::Message(e.to_string()))?
}

/// The spec's `OAuthLoginCallbacks` bound to stdio.
pub struct StdioLoginCallbacks;

impl OAuthLoginCallbacks for StdioLoginCallbacks {
    fn on_auth(&self, info: OAuthAuthInfo) {
        println!("Open the following URL in your browser to authorize:");
        println!();
        println!("  {}", info.url);
        println!();
        if let Some(instructions) = info.instructions {
            println!("{instructions}");
        }
        let _ = std::io::stdout().flush();
    }

    fn on_device_code(&self, info: OAuthDeviceCodeInfo) {
        println!(
            "Visit {} and enter code: {}",
            info.verification_uri, info.user_code
        );
        let _ = std::io::stdout().flush();
    }

    fn on_prompt(&self, prompt: OAuthPrompt) -> AuthFuture<'_, String> {
        Box::pin(async move {
            println!("{}", prompt.message);
            if let Some(placeholder) = &prompt.placeholder {
                println!("  (e.g. {placeholder})");
            }
            print!("> ");
            let _ = std::io::stdout().flush();
            loop {
                let line = prompt_line().await?;
                if !line.is_empty() || prompt.allow_empty {
                    return Ok(line);
                }
                print!("> ");
                let _ = std::io::stdout().flush();
            }
        })
    }

    fn on_select(&self, prompt: OAuthSelectPrompt) -> AuthFuture<'_, Option<String>> {
        Box::pin(async move {
            println!("{}", prompt.message);
            for (index, option) in prompt.options.iter().enumerate() {
                println!("  {}) {}", index + 1, option.label);
            }
            print!("> ");
            let _ = std::io::stdout().flush();
            let line = prompt_line().await?;
            let choice = line
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| prompt.options.get(i))
                .map(|option| option.id.clone());
            Ok(choice)
        })
    }

    fn on_progress(&self, message: &str) {
        println!("{message}");
        let _ = std::io::stdout().flush();
    }

    fn on_manual_code_input(&self) -> Option<AuthFuture<'_, String>> {
        // Race manual paste against the callback server, as pi's
        // interactive login does.
        Some(Box::pin(async {
            println!("Waiting for the browser callback — or paste the authorization code here:");
            print!("> ");
            let _ = std::io::stdout().flush();
            prompt_line().await
        }))
    }
}

/// Run `--login`: OAuth into `provider` (default anthropic) and persist
/// the credential to auth.json.
pub async fn run_login(
    auth_storage: &mut AuthStorage,
    provider: Option<&str>,
) -> Result<(), AuthStorageError> {
    let provider_id = provider.unwrap_or("anthropic");
    auth_storage
        .login(provider_id, &StdioLoginCallbacks)
        .await?;
    println!("Logged in to {provider_id}.");
    Ok(())
}
