//! Codemode scripts in a QuickJS child process (`packages/codemode`).
//!
//! The host side ([crate::host]) runs one script per child of the pi-rs binary, speaking upstream's
//! protocol over piped stdio; the child ([crate::child]) runs the prelude and the script.

pub mod child;
pub mod host;
pub mod protocol;
pub mod source;

pub use host::{ExecuteOptions, ScriptError, ScriptResult, execute};
pub use protocol::{
    DeclaredGlobal, DeclaredTool, HostToWorker, OutputItem, VmStartData, WorkerToHost,
};
pub use source::{ParsedSource, SourceOptions, parse as parse_source};
