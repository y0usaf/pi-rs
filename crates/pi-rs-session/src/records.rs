//! Named-record CRUD over the session append-only JSONL (P2).
//!
//! Records are ordinary entries in the same log the session manager writes:
//! `{"type": "record", "collection": ..., "key": ..., "op": "put"|"delete",
//! "value": ...}`. Collection names are data, never store verbs; Rust knows
//! none of the product schema (memories, skills, prompts, subagents,
//! refinements — those are Lua policy at P4).
//!
//! Semantics: latest op wins per (collection, key); a delete is a tombstone
//! that hides the key on get/list; appends are tolerant (a partial trailing
//! line from a kill mid-append is skipped, leaving the store readable).
//! Store compaction is deferred until a measured problem exists (plan P2).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const RECORD_ENTRY_TYPE: &str = "record";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordOp {
    Put,
    Delete,
}

/// One record entry as it appears in the JSONL.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordEntry {
    #[serde(rename = "type", default = "default_record_type")]
    pub entry_type: String,
    pub collection: String,
    pub key: String,
    pub op: RecordOp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

fn default_record_type() -> String {
    RECORD_ENTRY_TYPE.to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("record store read failed: {0}")]
    Read(String),
    #[error("record store append failed: {0}")]
    Append(String),
    #[error("record entry malformed: {0}")]
    Malformed(String),
}

/// A record store over one session file. Appends are opened with
/// O_APPEND and write a single line; readers tolerate a partial trailing
/// line (kill mid-append leaves the store readable).
pub struct RecordStore {
    path: PathBuf,
}

impl RecordStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn put(
        &self,
        collection: &str,
        key: &str,
        value: Value,
    ) -> Result<(), RecordError> {
        self.append(RecordEntry {
            entry_type: RECORD_ENTRY_TYPE.to_string(),
            collection: collection.to_string(),
            key: key.to_string(),
            op: RecordOp::Put,
            value: Some(value),
        })
    }

    pub fn delete(&self, collection: &str, key: &str) -> Result<(), RecordError> {
        self.append(RecordEntry {
            entry_type: RECORD_ENTRY_TYPE.to_string(),
            collection: collection.to_string(),
            key: key.to_string(),
            op: RecordOp::Delete,
            value: None,
        })
    }

    fn append(&self, entry: RecordEntry) -> Result<(), RecordError> {
        let mut line = serde_json::to_string(&entry)
            .map_err(|e| RecordError::Append(e.to_string()))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| RecordError::Append(e.to_string()))?;
        file.write_all(line.as_bytes())
            .map_err(|e| RecordError::Append(e.to_string()))?;
        file.flush().map_err(|e| RecordError::Append(e.to_string()))
    }

    /// Latest value for one key, or None when absent or tombstoned.
    pub fn get(&self, collection: &str, key: &str) -> Result<Option<Value>, RecordError> {
        Ok(self.fold()?.get(&(collection.to_string(), key.to_string())).cloned())
    }

    /// All live (non-tombstoned) records in a collection, in log order of
    /// their latest put.
    pub fn list(&self, collection: &str) -> Result<Vec<(String, Value)>, RecordError> {
        let fold = self.fold()?;
        let mut out: Vec<(String, Value)> = fold
            .into_iter()
            .filter(|((c, _), _)| c == collection)
            .map(|((_, k), v)| (k, v))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Latest-value fold over the whole log: put stores, delete tombstones
    /// (removes the key), latest op wins. Fails loudly on malformed record
    /// entries (corruption reported, never silently dropped).
    pub fn fold(&self) -> Result<HashMap<(String, String), Value>, RecordError> {
        let mut state: HashMap<(String, String), Value> = HashMap::new();
        // A store that does not exist yet reads as empty (fresh session).
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(state),
            Err(e) => return Err(RecordError::Read(e.to_string())),
        };
        let reader = BufReader::new(file);
        for (line_no, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| RecordError::Read(e.to_string()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                // Partial trailing line from a kill mid-append: skip.
                continue;
            };
            let Some(entry_type) = value.get("type").and_then(Value::as_str) else {
                continue;
            };
            if entry_type != RECORD_ENTRY_TYPE {
                continue;
            }
            let entry: RecordEntry = serde_json::from_value(value).map_err(|e| {
                RecordError::Malformed(format!("line {}: {e}", line_no + 1))
            })?;
            let key = (entry.collection, entry.key);
            match entry.op {
                RecordOp::Put => {
                    if let Some(value) = entry.value {
                        state.insert(key, value);
                    }
                }
                RecordOp::Delete => {
                    state.remove(&key);
                }
            }
        }
        Ok(state)
    }

    /// Number of record entries in the log (including tombstones), for
    /// store-limit accounting.
    pub fn count_entries(&self) -> Result<usize, RecordError> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(RecordError::Read(e.to_string())),
        };
        let reader = BufReader::new(file);
        let mut count = 0usize;
        for line in reader.lines() {
            let line = line.map_err(|e| RecordError::Read(e.to_string()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) == Some(RECORD_ENTRY_TYPE) {
                count += 1;
            }
        }
        Ok(count)
    }
}

/// Convenience: a record entry as a raw JSON object (for tests and for the
/// session file's base fields, which the session manager stamps).
pub fn record_entry_object(
    collection: &str,
    key: &str,
    op: RecordOp,
    value: Option<Value>,
) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("type".into(), Value::String(RECORD_ENTRY_TYPE.into()));
    m.insert("collection".into(), Value::String(collection.into()));
    m.insert("key".into(), Value::String(key.into()));
    m.insert("op".into(), serde_json::to_value(op).unwrap_or(Value::Null));
    if let Some(value) = value {
        m.insert("value".into(), value);
    }
    m
}
