//! pi-rs-repl: persistent Python/IPython kernel bridge.
//!
//! One long-lived Python child per agent scope, speaking a framed JSON-lines
//! stdio protocol (protocol.rs). The child is the vendored IPython shim
//! (shim/kernel-shim.py); the vendored prime-agent-runtime `rlm` package
//! runs inside it with host_request redirected from Jupyter comms to this
//! protocol's host_request frame.
//!
//! Mechanism only: no agent vocabulary here. The Lua seam (`pi.repl` in
//! pi-rs-host) and the RLM loop (Lua policy) own product behavior.
//!
//! This is a deliberate divergence from prime-agent's Jupyter ZMQ transport,
//! keeping the behavioral contract: ExecuteResult shape, host_request
//! semantics, dill snapshot/restore, typed display MIME payloads.

pub mod framing;
pub mod kernel;
pub mod protocol;

pub use kernel::{KernelConfig, KernelError, KernelManager};
pub use protocol::{ExecuteResult, KernelAttachment, KernelDiffDisplay, KernelSentAgentMessage, RestoreResult, SnapshotResult};
