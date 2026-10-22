//! Multi-line editor (upstream `components/editor.ts`). Cursor columns are byte offsets.
//! Autocomplete is synchronous: providers answer at once, so there is no debounce or abort.

pub mod text;
mod complete;
mod edit;
mod input;
mod kill;
mod nav;
mod render;

use crate::autocomplete::AutocompleteProvider;
use crate::components::select_list::{SelectItem, SelectList, SelectListTheme};
use crate::kill_ring::{KillRing, UndoStack};
use crate::unicode::{WordSegment, word_segments};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
pub use text::{TextChunk, word_wrap_line};

pub type Style = Rc<dyn Fn(&str) -> String>;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct EditorState {
    pub(crate) lines: Vec<String>,
    pub(crate) cursor_line: usize,
    pub(crate) cursor_col: usize,
}

#[derive(Clone)]
pub(crate) struct EditorSnapshot {
    state: EditorState,
    pastes: BTreeMap<u64, String>,
    paste_counter: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AutocompleteMode {
    Regular,
    Force,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Jump {
    Forward,
    Backward,
}

#[derive(Clone)]
pub struct EditorTheme {
    pub border_color: Style,
    pub select_list: SelectListTheme,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct EditorOptions {
    pub padding_x: Option<usize>,
    pub autocomplete_max_visible: Option<usize>,
}

/// One visual (wrapped) segment of a logical line, in bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct VisualLine {
    pub(crate) logical_line: usize,
    pub(crate) start_col: usize,
    pub(crate) length: usize,
}

pub struct Editor {
    pub(crate) state: EditorState,
    pub focused: bool,
    theme: EditorTheme,
    padding_x: usize,
    /// Terminal height, for the 30% visible-lines limit (upstream `tui.terminal.rows`).
    pub terminal_rows: usize,
    /// Called where upstream calls `tui.requestRender()`.
    pub request_render: Option<Rc<dyn Fn()>>,
    pub(crate) last_width: usize,
    rendered_visible_line_count: usize,
    rendered_autocomplete_height: usize,
    pub(crate) scroll_offset: usize,
    pub border_color: Style,
    autocomplete_provider: Option<Rc<dyn AutocompleteProvider>>,
    pub(crate) autocomplete_trigger_characters: Vec<String>,
    pub(crate) autocomplete_list: Option<SelectList>,
    pub(crate) autocomplete_state: Option<AutocompleteMode>,
    pub(crate) autocomplete_prefix: String,
    autocomplete_max_visible: usize,
    /// Set by the autocomplete list's `on_select` (mouse picks), applied by the editor.
    pub(crate) autocomplete_picked: Rc<RefCell<Option<SelectItem>>>,
    pub(crate) pastes: BTreeMap<u64, String>,
    pub(crate) paste_counter: u64,
    paste_buffer: String,
    is_in_paste: bool,
    pub(crate) history: Vec<String>,
    pub(crate) history_index: i64,
    history_draft: Option<EditorState>,
    pub(crate) kill_ring: KillRing,
    pub(crate) last_action: Option<LastAction>,
    jump_mode: Option<Jump>,
    pub(crate) preferred_visual_col: Option<usize>,
    pub(crate) snapped_from_cursor_col: Option<usize>,
    undo_stack: UndoStack<EditorSnapshot>,
    pub on_submit: Option<Box<dyn FnMut(&str)>>,
    pub on_change: Option<Box<dyn FnMut(&str)>>,
    pub disable_submit: bool,
}

fn clamp_max_visible(n: usize) -> usize {
    n.clamp(3, 20)
}

impl Editor {
    pub fn new(theme: EditorTheme, options: EditorOptions) -> Self {
        Editor {
            state: EditorState { lines: vec![String::new()], cursor_line: 0, cursor_col: 0 },
            focused: false,
            border_color: theme.border_color.clone(),
            theme,
            padding_x: options.padding_x.unwrap_or(0),
            terminal_rows: 24,
            request_render: None,
            last_width: 80,
            rendered_visible_line_count: 1,
            rendered_autocomplete_height: 0,
            scroll_offset: 0,
            autocomplete_provider: None,
            autocomplete_trigger_characters: text::DEFAULT_TRIGGER_CHARACTERS.iter().map(|s| s.to_string()).collect(),
            autocomplete_list: None,
            autocomplete_state: None,
            autocomplete_prefix: String::new(),
            autocomplete_max_visible: clamp_max_visible(options.autocomplete_max_visible.unwrap_or(5)),
            autocomplete_picked: Rc::new(RefCell::new(None)),
            pastes: BTreeMap::new(),
            paste_counter: 0,
            paste_buffer: String::new(),
            is_in_paste: false,
            history: Vec::new(),
            history_index: -1,
            history_draft: None,
            kill_ring: KillRing::default(),
            last_action: None,
            jump_mode: None,
            preferred_visual_col: None,
            snapped_from_cursor_col: None,
            undo_stack: UndoStack::default(),
            on_submit: None,
            on_change: None,
            disable_submit: false,
        }
    }

    pub(crate) fn render_requested(&self) {
        if let Some(r) = &self.request_render {
            r();
        }
    }

    pub(crate) fn emit_change(&mut self) {
        if self.on_change.is_some() {
            let t = self.get_text();
            if let Some(f) = self.on_change.as_mut() {
                f(&t);
            }
        }
    }

    /// Grapheme segments of `text`, with valid paste markers merged.
    pub(crate) fn seg_graphemes<'a>(&self, text: &'a str) -> Vec<WordSegment<'a>> {
        let p = &self.pastes;
        text::segment_with_markers(text, text::grapheme_segments(text), &|id| p.contains_key(&id), !p.is_empty())
    }

    /// Word segments of `text`, with valid paste markers merged.
    pub(crate) fn seg_words<'a>(&self, text: &'a str) -> Vec<WordSegment<'a>> {
        let p = &self.pastes;
        text::segment_with_markers(text, word_segments(text), &|id| p.contains_key(&id), !p.is_empty())
    }

    pub fn padding_x(&self) -> usize {
        self.padding_x
    }

    pub fn set_padding_x(&mut self, padding: usize) {
        if self.padding_x != padding {
            self.padding_x = padding;
            self.render_requested();
        }
    }

    pub fn autocomplete_max_visible(&self) -> usize {
        self.autocomplete_max_visible
    }

    pub fn set_autocomplete_max_visible(&mut self, n: usize) {
        let n = clamp_max_visible(n);
        if self.autocomplete_max_visible != n {
            self.autocomplete_max_visible = n;
            self.render_requested();
        }
    }

    pub fn set_autocomplete_provider(&mut self, provider: Rc<dyn AutocompleteProvider>) {
        self.cancel_autocomplete();
        let triggers = provider.trigger_characters().unwrap_or_default();
        self.autocomplete_provider = Some(provider);
        self.set_autocomplete_trigger_characters(triggers);
    }

    /// Add a submitted prompt to history (most recent first, no consecutive duplicates, max 100).
    pub fn add_to_history(&mut self, text: &str) {
        let trimmed = crate::utils::js_trim(text);
        if trimmed.is_empty() || self.history.first().is_some_and(|h| h == trimmed) {
            return;
        }
        self.history.insert(0, trimmed.to_string());
        self.history.truncate(100);
    }

    pub fn get_text(&self) -> String {
        self.state.lines.join("\n")
    }

    pub fn get_lines(&self) -> Vec<String> {
        self.state.lines.clone()
    }

    /// (line, byte column).
    pub fn get_cursor(&self) -> (usize, usize) {
        (self.state.cursor_line, self.state.cursor_col)
    }

    /// The text with paste markers replaced by their content.
    pub fn get_expanded_text(&self) -> String {
        let mut result = self.get_text();
        for (id, content) in &self.pastes {
            let mut out = String::new();
            let mut i = 0;
            for (s, e, mid) in text::paste_markers(&result) {
                if mid == *id {
                    out.push_str(&result[i..s]);
                    out.push_str(content);
                    i = e;
                }
            }
            out.push_str(&result[i..]);
            result = out;
        }
        result
    }

    pub fn is_showing_autocomplete(&self) -> bool {
        self.autocomplete_state.is_some()
    }
}
