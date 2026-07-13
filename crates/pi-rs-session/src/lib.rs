//! Generic durable record storage.
//!
//! The crate persists opaque [`serde_json::Value`] records in versioned,
//! checksummed append-only logs. It provides durability, locking, bounded
//! cursors, prefix copies, diagnostics, and cancellation only; record meaning
//! remains entirely with the caller.

pub mod record_store;
// Retained as a generic identifier utility because the host clipboard
// mechanism already consumes it; it carries no record or product semantics.
pub mod uuid;

pub use record_store::{
    CancellationToken, CursorWindow, FORMAT_VERSION, RecordCursor, RecordStore, STORE_EXTENSION,
    StoreDiagnostic, StoreError, StoreInfo, StoreLimits, StoreListing,
};
