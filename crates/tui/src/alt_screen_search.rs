use std::rc::Rc;

use crate::components::input::{Input, InputOptions};
use crate::keybindings::get_keybindings;
use crate::tui::Component;
use crate::unicode::{graphemes, is_js_whitespace};
use crate::utils::{strip_terminal_sequences, truncate_to_width, visible_width};

pub type NavigationButtonStyle = Rc<dyn Fn(&str, bool) -> String>;

struct SourceSpan {
    text_start: usize,
    text_end: usize,
    row: i64,
    start_col: i64,
    end_col: i64,
    linear_columns: bool,
}

struct Corpus {
    text: String,
    spans: Vec<SourceSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchSegment {
    pub row: i64,
    pub start_col: i64,
    pub end_col: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub segments: Vec<SearchSegment>,
}

impl SearchMatch {
    pub fn key(&self) -> String {
        match (self.segments.first(), self.segments.last()) {
            (Some(first), Some(last)) => format!(
                "{}:{}:{}:{}",
                first.row, first.start_col, last.row, last.end_col
            ),
            _ => String::new(),
        }
    }
}

impl Corpus {
    fn push(
        &mut self,
        pending_separator: &mut bool,
        text: &str,
        row: i64,
        (start_col, end_col): (i64, i64),
        linear_columns: bool,
    ) {
        if std::mem::take(pending_separator) {
            self.text.push(' ');
        }
        let text_start = self.text.len();
        self.text.push_str(text);
        self.spans.push(SourceSpan {
            text_start,
            text_end: self.text.len(),
            row,
            start_col,
            end_col,
            linear_columns,
        });
    }
}

fn build_corpus(lines: &[String]) -> Corpus {
    let mut corpus = Corpus {
        text: String::new(),
        spans: Vec::new(),
    };
    let mut pending_separator = false;
    for (row, line) in lines.iter().enumerate() {
        let row = row as i64;
        let line = strip_terminal_sequences(line);
        let mut column = 0i64;
        if line.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            for (index, word) in line.split(' ').enumerate() {
                if index > 0 {
                    if !corpus.text.is_empty() {
                        pending_separator = true;
                    }
                    column += 1;
                }
                if word.is_empty() {
                    continue;
                }
                let width = word.len() as i64;
                corpus.push(
                    &mut pending_separator,
                    word,
                    row,
                    (column, column + width),
                    true,
                );
                column += width;
            }
        } else {
            for grapheme in graphemes(&line) {
                let width = visible_width(grapheme) as i64;
                if grapheme.chars().all(is_js_whitespace) {
                    if !corpus.text.is_empty() {
                        pending_separator = true;
                    }
                    column += width;
                    continue;
                }
                corpus.push(
                    &mut pending_separator,
                    grapheme,
                    row,
                    (column, column + width),
                    false,
                );
                column += width;
            }
        }
        if !corpus.text.is_empty() {
            pending_separator = true;
        }
    }
    corpus
}

fn normalize_query(query: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in query.chars() {
        if is_js_whitespace(c) {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

fn find_matches(corpus: &Corpus, query: &str) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(expression) = fancy_regex::Regex::new(&format!("(?i){}", fancy_regex::escape(query)))
    else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    let mut span_index = 0;
    for found in expression.find_iter(&corpus.text).map_while(Result::ok) {
        let (start, end) = (found.start(), found.end());
        while span_index < corpus.spans.len() && corpus.spans[span_index].text_end <= start {
            span_index += 1;
        }
        let mut segments: Vec<SearchSegment> = Vec::new();
        for span in &corpus.spans[span_index..] {
            if span.text_start >= end {
                break;
            }
            if span.text_end <= start {
                continue;
            }
            let (start_col, end_col) = if span.linear_columns {
                (
                    span.start_col + (start.max(span.text_start) - span.text_start) as i64,
                    span.start_col + (end.min(span.text_end) - span.text_start) as i64,
                )
            } else {
                (span.start_col, span.end_col)
            };
            match segments.last_mut() {
                Some(previous) if previous.row == span.row && start_col <= previous.end_col => {
                    previous.end_col = previous.end_col.max(end_col);
                }
                _ => segments.push(SearchSegment {
                    row: span.row,
                    start_col,
                    end_col,
                }),
            }
        }
        while span_index < corpus.spans.len() && corpus.spans[span_index].text_end <= end {
            span_index += 1;
        }
        if !segments.is_empty() {
            matches.push(SearchMatch { segments });
        }
    }
    matches
}

#[derive(Default)]
pub struct AltScreenSearchIndex {
    source_lines: Option<Vec<String>>,
    corpus: Option<Corpus>,
    normalized_query: Option<String>,
    matches: Rc<Vec<SearchMatch>>,
}

impl AltScreenSearchIndex {
    pub fn search(&mut self, lines: &[String], query: &str) -> (Rc<Vec<SearchMatch>>, bool) {
        let source_changed = self.source_lines.as_deref() != Some(lines);
        if source_changed || self.corpus.is_none() {
            self.source_lines = Some(lines.to_vec());
            self.corpus = Some(build_corpus(lines));
        }
        let normalized = normalize_query(query);
        let changed = source_changed || self.normalized_query.as_deref() != Some(&normalized);
        if changed && let Some(corpus) = &self.corpus {
            self.matches = Rc::new(find_matches(corpus, &normalized));
            self.normalized_query = Some(normalized);
        }
        (self.matches.clone(), changed)
    }
}

pub struct AltScreenSearchComponent {
    input: Input,
    navigation_button_style: NavigationButtonStyle,
    result_count: usize,
    result_index: i64,
    previous_button: (i64, i64),
    next_button: (i64, i64),
    hovered: Option<i64>,
}

fn format_key(key: Option<&String>) -> String {
    let Some(key) = key else {
        return "Unbound".to_owned();
    };
    key.split('+')
        .map(|part| {
            if cfg!(target_os = "macos") && part.eq_ignore_ascii_case("alt") {
                return "Option".to_owned();
            }
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join("+")
}

impl AltScreenSearchComponent {
    pub fn new(navigation_button_style: NavigationButtonStyle) -> Self {
        AltScreenSearchComponent {
            input: Input::new(InputOptions {
                prompt: Some(" ".to_owned()),
                placeholder: Some("Find in transcript".to_owned()),
                placeholder_style: Some(Rc::new(|text: &str| format!("\x1b[2m{text}\x1b[22m"))),
            }),
            navigation_button_style,
            result_count: 0,
            result_index: -1,
            previous_button: (-1, -1),
            next_button: (-1, -1),
            hovered: None,
        }
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    pub fn set_result(&mut self, index: i64, count: usize) {
        self.result_index = index;
        self.result_count = count;
    }

    pub fn navigation_direction_at(&self, row: i64, column: i64) -> Option<i64> {
        if row != 2 {
            return None;
        }
        if column >= self.previous_button.0 && column < self.previous_button.1 {
            return Some(-1);
        }
        if column >= self.next_button.0 && column < self.next_button.1 {
            return Some(1);
        }
        None
    }

    pub fn set_hovered_navigation_direction(&mut self, direction: Option<i64>) -> bool {
        if direction == self.hovered {
            return false;
        }
        self.hovered = direction;
        true
    }
}

impl Component for AltScreenSearchComponent {
    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }

    fn handle_input(&mut self, data: &str) {
        self.input.handle_input(data);
    }

    fn invalidate(&mut self) {
        self.input.invalidate();
    }

    fn render(&mut self, width: usize) -> Vec<String> {
        let safe_width = width.max(1);
        let inner_width = safe_width.saturating_sub(2);
        let keybindings = get_keybindings();
        let previous_key = format_key(keybindings.keys("tui.altScreen.searchPrevious").first());
        let next_key = format_key(keybindings.keys("tui.altScreen.searchNext").first());
        let result = if self.input.value().is_empty() {
            String::new()
        } else if self.result_count == 0 {
            "No matches".to_owned()
        } else {
            format!("{}/{}", self.result_index + 1, self.result_count)
        };
        let visible_result = truncate_to_width(&result, inner_width.saturating_sub(3), "", false);
        let result_text = if visible_result.is_empty() {
            String::new()
        } else {
            format!("\x1b[2m {visible_result} \x1b[22m")
        };
        let input_width = inner_width.saturating_sub(visible_width(&result_text));
        let rendered = self.input.render(input_width.max(1));
        let input_line = truncate_to_width(
            rendered.first().map_or("", String::as_str),
            input_width,
            "",
            false,
        );
        let input_padding = " ".repeat(input_width.saturating_sub(visible_width(&input_line)));
        let content = format!("{input_line}{input_padding}{result_text}");

        let mut previous_button = format!("↑ {previous_key}");
        let mut next_button = format!("↓ {next_key}");
        let mut separator = " · ";
        let outer_gap = 1;
        let available = inner_width.saturating_sub(outer_gap * 2 + 1);
        let mut controls = visible_width(&previous_button)
            + visible_width(separator)
            + visible_width(&next_button);
        if controls > available {
            previous_button = "↑".to_owned();
            next_button = "↓".to_owned();
            separator = " ";
            controls = visible_width(&previous_button)
                + visible_width(separator)
                + visible_width(&next_button);
        }
        let show_buttons = controls <= available;
        let rendered_buttons = if show_buttons {
            format!(
                "{}{separator}{}",
                (self.navigation_button_style)(&previous_button, self.hovered == Some(-1)),
                (self.navigation_button_style)(&next_button, self.hovered == Some(1))
            )
        } else {
            String::new()
        };
        let outer_gaps = if show_buttons { outer_gap * 2 } else { 0 };
        let right_rule =
            usize::from(!rendered_buttons.is_empty() && inner_width > controls + outer_gaps);
        let left_rule = inner_width
            .saturating_sub(if show_buttons { controls } else { 0 })
            .saturating_sub(outer_gaps)
            .saturating_sub(right_rule);
        let previous_start = (1 + left_rule + outer_gap) as i64;
        if show_buttons {
            let previous_end = previous_start + visible_width(&previous_button) as i64;
            let next_start = previous_end + visible_width(separator) as i64;
            self.previous_button = (previous_start, previous_end);
            self.next_button = (next_start, next_start + visible_width(&next_button) as i64);
        } else {
            self.previous_button = (-1, -1);
            self.next_button = (-1, -1);
        }

        if safe_width == 1 {
            return vec!["┌".to_owned(), "│".to_owned(), "└".to_owned()];
        }
        let gap = if rendered_buttons.is_empty() { "" } else { " " };
        vec![
            format!("┌{}┐", "─".repeat(inner_width)),
            format!("│{content}│"),
            format!(
                "└{}{gap}{rendered_buttons}{gap}{}┘",
                "─".repeat(left_rule),
                "─".repeat(right_rule)
            ),
        ]
    }
}
