//! Upstream `core/extensions`: the user's extension packages, run in one Node child.
//!
//! The child loads each path as TypeScript, calls the factory with the API pi-rs implements, and
//! reports what it registered. Nothing here decides *which* paths to load; that is the agent's
//! `extensions` module, which knows the settings and the flags.

mod host;

use std::time::Duration;

pub use host::{Call, Ctx, Event, Failed, Host, Reply};
use serde::{Deserialize, Serialize};

/// How long the child may take to import the extensions and run their factories.
pub const LOAD_BUDGET: Duration = Duration::from_secs(20);

/// How long one event handler or command handler may take; past it the child is dead to us.
pub const EVENT_BUDGET: Duration = Duration::from_secs(10);

/// One extension the child loaded, with what its factory registered.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub path: String,
    pub resolved_path: String,
    pub events: Vec<String>,
    pub commands: Vec<Command>,
    /// The API members the factory reached for that pi-rs does not implement yet.
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

/// What one load produced: upstream's `LoadExtensionsResult`, minus warnings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Load {
    pub extensions: Vec<Loaded>,
    pub errors: Vec<LoadError>,
}
