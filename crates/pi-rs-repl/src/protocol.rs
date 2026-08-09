//! Wire protocol between the Rust host and the Python kernel shim.
//!
//! Framing: 4-byte big-endian length prefix + one UTF-8 JSON object.
//! One integer wire version; additive changes keep old shims working,
//! incompatible shims reject with a clear ready error.
//!
//! Host -> shim:  execute / interrupt / host_response / snapshot / restore / shutdown
//! Shim -> host:  ready / stream / result / host_request / snapshot_data / restore_data
//!
//! The shim (shim/kernel-shim.py) is the protocol's other half; keep the two
//! in lockstep. This is the documented divergence from prime-agent's Jupyter
//! ZMQ transport (DESIGN: pi-rs-repl framing), preserving the behavioral
//! contract: ExecuteResult shape, host_request semantics, snapshot/restore,
//! typed display MIME payloads.

use serde::{Deserialize, Serialize};

/// Current wire version. Bump on any breaking change; the host rejects
/// shims reporting a different version at ready time.
pub const WIRE_VERSION: u32 = 1;

/// Upper bound on a single frame, defense against a runaway shim.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Host -> shim frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMsg {
    Execute {
        v: u32,
        id: u64,
        code: String,
        #[serde(default = "default_max_chars")]
        max_chars: u64,
    },
    Interrupt {
        v: u32,
        id: u64,
    },
    HostResponse {
        v: u32,
        req_id: u64,
        status: String,
        payload: serde_json::Value,
    },
    Snapshot {
        v: u32,
        id: u64,
        path: String,
        manifest_path: String,
        #[serde(default = "default_max_bytes")]
        max_bytes: u64,
    },
    Restore {
        v: u32,
        id: u64,
        path: String,
    },
    Shutdown {
        v: u32,
        id: u64,
    },
}

/// Shim -> host frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShimMsg {
    Ready {
        v: u32,
        #[serde(default)]
        error: Option<String>,
    },
    Stream {
        v: u32,
        id: u64,
        name: String,
        chunk: String,
    },
    Result {
        v: u32,
        id: u64,
        ok: bool,
        value: serde_json::Value,
        error: Option<String>,
        duration_ms: u64,
    },
    HostRequest {
        v: u32,
        req_id: u64,
        kind: String,
        payload: serde_json::Value,
    },
    SnapshotData {
        v: u32,
        id: u64,
        ok: bool,
        value: Option<serde_json::Value>,
        error: Option<String>,
    },
    RestoreData {
        v: u32,
        id: u64,
        ok: bool,
        value: Option<serde_json::Value>,
        error: Option<String>,
    },
}

fn default_max_chars() -> u64 {
    65_536
}

fn default_max_bytes() -> u64 {
    256 * 1024 * 1024
}

/// ExecuteResult, the behavioral contract shared with prime-agent's
/// KernelManager.ExecuteResult (kernel/index.ts).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExecuteResult {
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub result: Option<String>,
    pub diffs: Vec<KernelDiffDisplay>,
    pub attachments: Vec<KernelAttachment>,
    pub sent_agent_messages: Vec<KernelSentAgentMessage>,
    pub status: String,
    pub error: Option<ExecutionError>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KernelDiffDisplay {
    pub path: String,
    pub old_str: String,
    pub new_str: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KernelAttachment {
    pub mime_type: String,
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KernelSentAgentMessage {
    pub id: String,
    pub message: String,
    pub delivery_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_role: Option<String>,
    pub target: AgentMessageTarget,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentMessageTarget {
    pub active_session_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionError {
    pub ename: String,
    pub evalue: String,
    pub traceback: Vec<String>,
}

/// Result of a snapshot request (mirrors state-snapshot.ts SnapshotResult).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotResult {
    pub saved: Vec<String>,
    pub skipped: Vec<serde_json::Value>,
    pub bytes: u64,
    pub path: String,
}

/// Result of a restore request (mirrors state-snapshot.ts RestoreResult).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RestoreResult {
    pub restored: Vec<String>,
    pub failed: Vec<serde_json::Value>,
    pub path: String,
}
