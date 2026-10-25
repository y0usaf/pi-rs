//! The messages between the host and the sandbox child (`packages/codemode/src/runtime/protocol.ts`).
//!
//! Tool arguments, results and values cross as JSON strings: the child passes them into and out of
//! the VM as strings and never builds structured values itself. One message per line.

use serde::{Deserialize, Serialize};

/// Everything a VM needs to run one script, independent of where it runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmStartData {
    pub code: String,
    /// `js_name` is the identifier the script uses; `description` is listed in `ALL_TOOLS`.
    pub tools: Vec<DeclaredTool>,
    pub globals: Vec<DeclaredGlobal>,
    #[serde(rename = "memoryLimitBytes", skip_serializing_if = "Option::is_none")]
    pub memory_limit_bytes: Option<usize>,
    /// Snapshot for `load()`: key to JSON text.
    pub store: std::collections::BTreeMap<String, String>,
}

/// `toCodemodeIdentifier`: characters that are not valid in a JavaScript identifier become `_`.
pub fn to_identifier(name: &str) -> String {
    let mut identifier = String::new();
    for character in name.chars() {
        let valid = if identifier.is_empty() {
            character.is_ascii_alphabetic() || character == '_' || character == '$'
        } else {
            character.is_ascii_alphanumeric() || character == '_' || character == '$'
        };
        identifier.push(if valid { character } else { '_' });
    }
    if identifier.is_empty() {
        "_".to_owned()
    } else {
        identifier
    }
}

impl DeclaredTool {
    /// A tool whose script identifier is `to_identifier(name)`.
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_owned(),
            js_name: to_identifier(name),
            description: description.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclaredTool {
    pub name: String,
    #[serde(rename = "jsName")]
    pub js_name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclaredGlobal {
    pub name: String,
    pub spread: bool,
}

/// One item of the script's output, in the order the script produced it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OutputItem {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        console: bool,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallTarget {
    Tool,
    Global,
}

/// What the sandbox reports while a script runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WorkerToHost {
    Call {
        id: u64,
        target: CallTarget,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<String>,
    },
    Output {
        item: OutputItem,
    },
    /// `ok: false` carries `error`, the JSON of `{ name?, message, stack? }`.
    Done {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        writes: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The VM failed outside the script's control, for example a wasm trap.
    Crash {
        message: String,
    },
}

/// The host's answer to a call: `payload` is the JSON result when `ok`, else the error message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HostToWorker {
    Result {
        id: u64,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<String>,
        /// A rejected call: a JSON object whose properties the script's `Error` gets.
        #[serde(skip_serializing_if = "Option::is_none")]
        properties: Option<String>,
    },
}

impl VmStartData {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

impl WorkerToHost {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

impl HostToWorker {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}
