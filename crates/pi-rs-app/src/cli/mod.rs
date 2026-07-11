//! `cli/` — mirrors the spec's `src/cli/`. WS2.6 lands the bare-core
//! entry points; session-picker, startup-ui, file-processor, and
//! initial-message land with their workstreams.

pub mod args;
pub mod list_models;
pub mod login;
pub mod session_select;
