//! \`ai/src/auth/types.ts\`: what a login flow asks the app for, and what it tells it.

use pi_rt::cancel::CancelToken;

/// A question shown while logging in (upstream \`AuthPrompt\`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// A one-of list; the answer is the chosen option's id.
    Select {
        message: String,
        options: Vec<(String, String)>,
    },
    /// Free text, typed as a secret.
    Secret { message: String },
    /// Free text.
    Text { message: String },
    /// A pasted authorization code, redirect URL or \`code#state\`.
    ManualCode {
        message: String,
        placeholder: String,
    },
}

/// Something the flow reports while it runs (upstream \`AuthEvent\`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
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
    Info {
        message: String,
        links: Vec<(String, String)>,
    },
}

/// The app's side of a login (upstream \`ProviderAuthInteraction\`).
pub trait Interaction {
    /// Asks the user, and returns \`None\` when the flow withdrew the question because something else
    /// answered it, such as a browser callback. \`dismiss\` is cancelled by the flow in that case.
    fn prompt(&mut self, prompt: Prompt, dismiss: &CancelToken) -> Result<Option<String>, String>;

    fn notify(&mut self, event: Event);
}
