//! pi-tui in Rust: a port of upstream `packages/tui`.
//!
//! Strings are Rust `String`s, so every text offset (cursor columns, slice positions) is a byte
//! offset into UTF-8 where upstream uses UTF-16 code units. Widths, wrapping and rendered output
//! are the same.

mod unicode_tables;
pub mod unicode;
pub mod utils;
