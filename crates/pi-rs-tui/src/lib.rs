//! Generic terminal mechanisms: bounded input decoding, Unicode cell layout,
//! retained display batches, differential ANSI presentation, terminal images,
//! and process lifecycle cleanup.

pub mod display;
pub mod process;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_image;
pub mod ui_harness;
pub mod utils;
