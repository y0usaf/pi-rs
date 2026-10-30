//! Upstream `core/extensions`: the user's extension packages, run in one Node child.
//!
//! The child runs upstream's own loader and `ExtensionRunner` over upstream's packages, with what
//! reads or changes the agent's state calling back into pi-rs, and reports what each extension
//! registered. Nothing here decides *which* paths to load; that is the agent's `extensions`
//! module, which knows the settings and the flags.

mod host;

use std::time::Duration;

pub use host::{Call, Ctx, Failed, Host, Peer, Pending, Sources};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long the child may take to import the extensions and run their factories.
pub const LOAD_BUDGET: Duration = Duration::from_secs(20);

pub const EVENT_BUDGET: Duration = Duration::from_secs(10);

/// One extension the child loaded, with what its factory registered.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub path: String,
    pub resolved_path: String,
    pub events: Vec<String>,
    pub commands: Vec<Command>,
    /// What the extension registered that upstream acts on in this mode and pi-rs does not yet.
    pub unsupported: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Command {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoadError {
    pub path: String,
    pub error: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub parameters: Value,
    #[serde(default)]
    pub prompt_snippet: Option<String>,
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub exposure: Option<String>,
    #[serde(default)]
    pub default_active: Option<bool>,
    pub path: String,
}

/// A provider an extension registered: `pi.registerProvider`'s object as pi-rs needs it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The login it offers, when it has one.
    #[serde(default)]
    pub oauth: Option<OAuthInfo>,
    /// Whether requests to its models go to its own `streamSimple`.
    #[serde(default)]
    pub streams: bool,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthInfo {
    #[serde(default)]
    pub name: String,
    /// The built-in provider whose login flow and token refresh this login runs.
    #[serde(default)]
    pub flow: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct TrustDecision {
    pub trusted: bool,
    pub remember: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProjectTrust {
    pub decided: Option<TrustDecision>,
    pub errors: Vec<Failed>,
}

/// What one load produced: upstream's `LoadExtensionsResult`, minus warnings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Load {
    pub extensions: Vec<Loaded>,
    pub errors: Vec<LoadError>,
    #[serde(default)]
    pub tools: Vec<ToolInfo>,
    #[serde(default)]
    pub providers: Vec<ProviderInfo>,
    #[serde(default)]
    pub flag_errors: Vec<String>,
}
