//! Interactive mode (`modes/interactive`): the editor, streamed assistant Markdown, tool rows and
//! the footer, on pi-durable's agent events.

pub mod bash_mode;
pub mod chat;
pub mod clipboard;
pub mod external_editor;
pub mod footer;
pub mod header;
pub mod keybindings;
pub mod messages;
pub mod open_browser;
pub mod pending;
pub mod resources;
pub mod run;
pub mod screen;
pub mod selectors;
pub mod signals;
pub mod slash_commands;
pub mod status;
pub mod syntax;
pub mod theme;
pub mod tool_row;
