//! `packages/agent` port. Product behavior is the embedded Lua pack; this
//! crate contains only the typed seam and the pack descriptor.

use serde::{Deserialize, Serialize};

/// Open event envelope at the Rust/Lua seam. Event names remain strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub payload: serde_json::Map<String, serde_json::Value>,
}

/// The first-party agent policy, loaded through the same path as user packs.
pub const PACK: pi_rs_host::EmbeddedPack = pi_rs_host::EmbeddedPack {
    name: "pi-rs-agent",
    source: include_str!("../lua/agent.lua"),
};
