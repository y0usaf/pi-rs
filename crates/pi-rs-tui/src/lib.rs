//! pi-rs-tui — the `packages/tui` port (spec: `ref/pi` @ `c5582102`, pi
//! v0.79.0).
//!
//! WS6 owns the component tree (differential renderer, editor,
//! autocomplete, …). The crate is seeded early with the one module the
//! WS2 bare binary needs: [`fuzzy`] (`--list-models` search).

pub mod autocomplete;
pub mod box_component;
pub mod component;
pub mod editor;
pub mod fuzzy;
pub mod input;
pub mod kill_ring;
pub mod loader;
pub mod markdown;
pub mod process;
pub mod select_list;
pub mod settings_list;
pub mod spacer;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_image;
pub mod truncated_text;
pub mod tui;
pub mod ui_harness;
pub mod undo_stack;
pub mod utils;
