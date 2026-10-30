//! The headless modes: `--mode json` (`modes/print-mode.ts` with its JSON event stream,
//! `modes/json-event.ts`) and `--mode rpc` (`modes/rpc/rpc-mode.ts`). Both stream the durable
//! harness's agent events as one JSON object per line; `json_event` maps them to the shapes the
//! TypeScript session events have.
//!
//! One line per event, LF only, flushed as written (`modes/rpc/jsonl.ts`).

pub mod json_event;
pub mod json_mode;
pub mod rpc;

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::stdout;

/// A stdout line writer both the event stream and the command handler use, so their lines never
/// interleave inside one object.
#[derive(Clone, Default)]
pub struct Output(Arc<Mutex<Option<Vec<Value>>>>);

impl Output {
    pub fn line(&self, value: &Value) {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_mut() {
            Some(lines) => lines.push(value.clone()),
            None => write(value),
        }
    }

    pub fn hold(&self) {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        held.get_or_insert_with(Vec::new);
    }

    pub fn release(&self, first: &Value) {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        write(first);
        for line in held.take().into_iter().flatten() {
            write(&line);
        }
    }
}

fn write(value: &Value) {
    let _ = stdout::write(&format!("{value}\n"));
}

pub fn value<T: serde::Serialize>(item: &T) -> Value {
    let mut value = serde_json::to_value(item).unwrap_or(Value::Null);
    crate::sessions::export::upstream_usage(&mut value);
    value
}
