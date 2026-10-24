//! Components (upstream `components/`).

pub mod editor;
pub mod input;
pub mod loader;
pub mod markdown;
pub mod select_list;
pub mod spacer;
pub mod text;
pub mod truncated_text;

/// A callback given the component's text.
pub type TextCallback = Box<dyn FnMut(&str)>;
