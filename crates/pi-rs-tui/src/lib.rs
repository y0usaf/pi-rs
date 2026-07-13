//! Generic terminal mechanisms: bounded input decoding, Unicode cell layout,
//! retained display batches, differential ANSI presentation, terminal images,
//! and process lifecycle cleanup.

pub mod autocomplete;
pub mod box_component;
pub mod component;
pub mod display;
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
