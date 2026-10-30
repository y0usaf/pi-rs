//! pi-tui in Rust: a port of upstream `packages/tui`.
//!
//! Strings are Rust `String`s, so every text offset (cursor columns, slice positions) is a byte
//! offset into UTF-8 where upstream uses UTF-16 code units. Widths, wrapping and rendered output
//! are the same.

pub mod autocomplete;
pub mod colors;
pub mod components;
pub mod fuzzy;
pub mod keybindings;
pub mod keys;
pub mod kill_ring;
pub mod latex;
pub mod layout;
pub mod program_status;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_colors;
pub mod tui;
pub mod tui_alt_screen;
pub mod tui_main_screen;
pub mod unicode;
mod unicode_tables;
pub mod utils;
pub mod wheel_scroll;
pub mod word_navigation;
