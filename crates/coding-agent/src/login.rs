//! `core/model-runtime.ts`'s login and `interactive-mode.ts`'s `/login` and `/logout`: which providers
//! this port can log into, the flow for each, and the credential it writes to `auth.json`.
//!
//! A login runs on its own thread. The flow in `pi_ai::oauth` asks for input and reports progress
//! through `ChannelInteraction`, which hands every step to the screen over a channel and waits for
//! the answer, so a browser callback can withdraw a paste prompt and a cancel reaches the network
//! call.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use pi_durable::ai::oauth::anthropic;
use pi_durable::ai::oauth::interaction::{Event, Interaction, Prompt};
use pi_durable::ai::oauth::openai_codex;
use pi_durable::ai::oauth::{self, kimi_coding, meta, openrouter, xai};
use pi_durable::cancel::CancelToken;
use serde_json::json;

use crate::auth::AuthStore;

/// How a provider is authenticated (upstream `AuthType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    Oauth,
    ApiKey,
}

impl AuthType {
    /// `formatAuthSelectorProviderType`.
    pub fn label(self, subscription: Option<bool>) -> &'static str {
        match self {
            AuthType::ApiKey => "API key",
            AuthType::Oauth if subscription == Some(false) => "account",
            AuthType::Oauth => "subscription",
        }
    }
}

/// One way of logging into a provider, as the selector lists it (upstream
/// `AuthSelectorProvider`).
pub struct Provider {
    pub id: &'static str,
    /// The provider's display name, which the selector shows.
    pub provider: &'static str,
    pub auth: AuthType,
    /// The auth method's display name, for example `Anthropic API key`.
    pub name: &'static str,
    /// The prompt an API-key login asks, `None` when the key comes from the environment.
    pub key_prompt: Option<&'static str>,
    /// `Some(false)` labels an OAuth login an account rather than a subscription.
    pub subscription: Option<bool>,
}

const fn oauth(
    id: &'static str,
    provider: &'static str,
    name: &'static str,
    subscription: Option<bool>,
) -> Provider {
    Provider {
        id,
        provider,
        auth: AuthType::Oauth,
        name,
        key_prompt: None,
        subscription,
    }
}

const fn api_key(
    id: &'static str,
    provider: &'static str,
    name: &'static str,
    key_prompt: &'static str,
) -> Provider {
    Provider {
        id,
        provider,
        auth: AuthType::ApiKey,
        name,
        key_prompt: Some(key_prompt),
        subscription: None,
    }
}

/// What `/login` offers: every provider whose flow this port has, sorted as upstream sorts the
/// selector. Upstream lists the rest beside these; the README says which are missing.
static PROVIDERS: &[Provider] = &[
    oauth(
        "anthropic",
        "Anthropic",
        "Anthropic (Claude Pro/Max)",
        Some(true),
    ),
    api_key(
        "anthropic",
        "Anthropic",
        "Anthropic API key",
        "Enter Anthropic API key",
    ),
    oauth(
        "kimi-coding",
        "Kimi For Coding",
        "Kimi Code (subscription)",
        Some(true),
    ),
    api_key(
        "kimi-coding",
        "Kimi For Coding",
        "Kimi API key",
        "Enter Kimi API key",
    ),
    oauth("meta", "Meta", "Meta (Muse subscription)", Some(true)),
    api_key(
        "meta",
        "Meta",
        "Meta Model API key",
        "Enter Meta Model API key",
    ),
    oauth(
        "openai-codex",
        "OpenAI Codex (legacy)",
        "OpenAI (ChatGPT Plus/Pro)",
        Some(true),
    ),
    oauth("openrouter", "OpenRouter", "OpenRouter OAuth", Some(false)),
    api_key(
        "openrouter",
        "OpenRouter",
        "OpenRouter API key",
        "Enter OpenRouter API key",
    ),
    oauth("xai", "xAI", "xAI (Grok/X subscription)", Some(true)),
    api_key("xai", "xAI", "xAI API key", "Enter xAI API key"),
];

/// The providers a selector shows for one auth type, or every one when `auth` is `None`.
pub fn providers_for(auth: Option<AuthType>) -> Vec<&'static Provider> {
    PROVIDERS
        .iter()
        .filter(|provider| auth.is_none_or(|want| provider.auth == want))
        .collect()
}

/// The options `/login <provider>` matches by id or display name, as upstream's
/// `findLoginProviderOptions` does.
pub fn find(reference: &str) -> Vec<&'static Provider> {
    let reference = reference.trim().to_lowercase();
    if reference.is_empty() {
        return Vec::new();
    }
    PROVIDERS
        .iter()
        .filter(|provider| {
            provider.id.to_lowercase() == reference || provider.provider.to_lowercase() == reference
        })
        .collect()
}

/// A provider's display name, or its id when this port has no entry for it.
pub fn display_name(provider: &str) -> String {
    PROVIDERS
        .iter()
        .find(|option| option.id == provider)
        .map(|option| option.provider.to_owned())
        .unwrap_or_else(|| provider.to_owned())
}

/// Whether one auth type has anything to offer, so the method selector can skip a question with a
/// single answer.
pub fn has_auth(auth: AuthType) -> bool {
    PROVIDERS.iter().any(|provider| provider.auth == auth)
}

/// A full login: one provider, one auth type.
#[derive(Clone, PartialEq, Eq)]
pub struct LoginRequest {
    pub provider: String,
    pub auth: AuthType,
}

/// What the flow tells the screen.
pub enum UiEvent {
    AuthUrl {
        url: String,
        instructions: Option<String>,
    },
    DeviceCode {
        user_code: String,
        verification_uri: String,
        interval_seconds: u64,
        expires_in_seconds: u64,
    },
    Progress(String),
    /// A question with an id the screen answers through `Login::answer`.
    Prompt {
        id: u64,
        prompt: Prompt,
    },
    /// The flow answered the question itself; the screen drops it.
    Dismiss(u64),
    Done(Result<Outcome, String>),
}

/// A credential that is now in `auth.json`.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub provider: String,
    pub name: String,
    pub auth: AuthType,
    pub path: String,
}

/// A login running on its own thread, with the channel the screen drives it through.
pub struct Login {
    pub events: Receiver<UiEvent>,
    answers: Sender<(u64, Option<String>)>,
    cancel: CancelToken,
    worker: Option<JoinHandle<()>>,
}

impl Login {
    pub fn start(request: LoginRequest, agent_dir: &str) -> Self {
        let (event_tx, events) = channel();
        let (answers, answer_rx) = channel();
        let cancel = CancelToken::new();
        let worker_cancel = cancel.clone();
        let name = provider_name(&request);
        let agent_dir = agent_dir.to_owned();
        let worker =
            std::thread::spawn(move || {
                let mut interaction = ChannelInteraction {
                    events: event_tx.clone(),
                    answers: answer_rx,
                    next_id: 0,
                };
                let result = authenticate(&mut interaction, &request, &agent_dir, &worker_cancel)
                    .map(|path| Outcome {
                        provider: request.provider.clone(),
                        name,
                        auth: request.auth,
                        path,
                    });
                let _ = event_tx.send(UiEvent::Done(result));
            });
        Login {
            events,
            answers,
            cancel,
            worker: Some(worker),
        }
    }

    /// Answers a pending prompt; `None` means the user dismissed it.
    pub fn answer(&self, id: u64, value: Option<String>) {
        let _ = self.answers.send((id, value));
    }

    /// Stops the flow: the network calls, the callback listener and a pending prompt.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn provider_name(request: &LoginRequest) -> String {
    PROVIDERS
        .iter()
        .find(|provider| provider.id == request.provider && provider.auth == request.auth)
        .map(|provider| provider.provider.to_owned())
        .unwrap_or_else(|| request.provider.clone())
}

/// The `auth.json` this port shares with upstream.
pub fn auth_path(agent_dir: &str) -> String {
    crate::paths::join(agent_dir, "auth.json")
}

/// Runs one login and saves its credential, returning the file it was written to.
fn authenticate(
    interaction: &mut dyn Interaction,
    request: &LoginRequest,
    agent_dir: &str,
    cancel: &CancelToken,
) -> Result<String, String> {
    let provider = request.provider.as_str();
    let credential = match (provider, request.auth) {
        ("anthropic", AuthType::Oauth) => {
            let credentials =
                anthropic::login(interaction, &anthropic::Endpoints::from_env(), cancel)?;
            oauth::stored(provider, &credentials)
        }
        ("openai-codex", AuthType::Oauth) => {
            let credentials =
                openai_codex::login(interaction, &openai_codex::Endpoints::from_env(), cancel)?;
            oauth::stored(provider, &credentials)
        }
        ("kimi-coding", AuthType::Oauth) => {
            let credentials =
                kimi_coding::login(interaction, &kimi_coding::Endpoints::from_env(), cancel)?;
            oauth::stored(provider, &credentials)
        }
        ("meta", AuthType::Oauth) => {
            let credentials = meta::login(interaction, &meta::Endpoints::from_env(), cancel)?;
            oauth::stored(provider, &credentials)
        }
        ("openrouter", AuthType::Oauth) => {
            let credentials =
                openrouter::login(interaction, &openrouter::Endpoints::from_env(), cancel)?;
            oauth::stored(provider, &credentials)
        }
        ("xai", AuthType::Oauth) => {
            let credentials = xai::login(interaction, &xai::Endpoints::from_env(), cancel)?;
            oauth::stored(provider, &credentials)
        }
        (_, AuthType::ApiKey) => {
            let provider_entry = PROVIDERS
                .iter()
                .find(|p| p.id == provider && p.auth == AuthType::ApiKey)
                .ok_or_else(|| format!("{provider} does not support API key login"))?;
            let message = provider_entry
                .key_prompt
                .ok_or_else(|| format!("{} is configured outside pi", provider_entry.name))?;
            let key = interaction
                .prompt(
                    Prompt::Secret {
                        message: message.to_owned(),
                    },
                    cancel,
                )?
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| "Login cancelled".to_owned())?;
            json!({ "type": "api_key", "key": key })
        }
        _ => return Err(format!("{provider} does not support OAuth login")),
    };
    let path = auth_path(agent_dir);
    let mut store = AuthStore::open(&path);
    store.write(provider, credential)?;
    Ok(path)
}

/// `/logout`: removes the stored credential.
pub fn logout(agent_dir: &str, provider: &str) -> Result<(), String> {
    let mut store = AuthStore::open(&auth_path(agent_dir));
    store.remove(provider)
}

/// The providers with a stored credential, for `/logout`'s list: the id, the display name and
/// the auth type of the stored credential.
pub fn stored(agent_dir: &str) -> Result<Vec<(String, String, AuthType)>, String> {
    let store = AuthStore::open(&auth_path(agent_dir));
    Ok(store
        .providers()
        .into_iter()
        .filter_map(|id| {
            let auth = configured(agent_dir, &id)?;
            let name = PROVIDERS
                .iter()
                .find(|provider| provider.id == id)
                .map(|provider| provider.provider.to_owned())
                .unwrap_or_else(|| id.clone());
            Some((id, name, auth))
        })
        .collect())
}

/// Which auth type `provider` is configured with, for the selector's status line: a stored
/// credential, else an API key in the provider's environment variable. `None` is not configured.
pub fn configured(agent_dir: &str, provider: &str) -> Option<AuthType> {
    match AuthStore::open(&auth_path(agent_dir)).read(provider) {
        Some(crate::auth::Credential::OAuth(_)) => Some(AuthType::Oauth),
        Some(crate::auth::Credential::ApiKey { key, .. }) => key
            .is_some_and(|key| !key.is_empty())
            .then_some(AuthType::ApiKey),
        None => pi_durable::ai::auth::env_api_key(provider, &std::collections::BTreeMap::new())
            .is_some()
            .then_some(AuthType::ApiKey),
    }
}

/// The login thread's side of the screen: prompts wait for an answer, and a `dismiss` token
/// withdraws one that the flow resolved itself.
struct ChannelInteraction {
    events: Sender<UiEvent>,
    answers: Receiver<(u64, Option<String>)>,
    next_id: u64,
}

impl Interaction for ChannelInteraction {
    fn prompt(&mut self, prompt: Prompt, dismiss: &CancelToken) -> Result<Option<String>, String> {
        let id = self.next_id;
        self.next_id += 1;
        if self.events.send(UiEvent::Prompt { id, prompt }).is_err() {
            return Err("Login cancelled".to_owned());
        }
        loop {
            if dismiss.is_cancelled() {
                let _ = self.events.send(UiEvent::Dismiss(id));
                return Ok(None);
            }
            match self.answers.recv_timeout(Duration::from_millis(50)) {
                Ok((answer_id, value)) if answer_id == id => return Ok(value),
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Login cancelled".to_owned());
                }
            }
        }
    }

    fn notify(&mut self, event: Event) {
        let update = match event {
            Event::AuthUrl { url, instructions } => UiEvent::AuthUrl { url, instructions },
            Event::DeviceCode {
                user_code,
                verification_uri,
                interval_seconds,
                expires_in_seconds,
            } => UiEvent::DeviceCode {
                user_code,
                verification_uri,
                interval_seconds,
                expires_in_seconds,
            },
            Event::Progress(text) => UiEvent::Progress(text),
            Event::Info { message, .. } => UiEvent::Progress(message),
        };
        let _ = self.events.send(update);
    }
}
