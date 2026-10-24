//! Codemode scripts in a QuickJS child process (`packages/codemode`).
//!
//! The host side ([`host`]) runs one script per child of the pi-rs binary, speaking upstream's
//! protocol over piped stdio; the child ([crate::child]) runs the prelude and the script.

pub mod child;
pub mod protocol;

pub use protocol::{HostToWorker, OutputItem, VmStartData, WorkerToHost};
