//! Select-list component mechanism — port of `packages/tui/src/components/select-list.ts`.
//!
//! Styling is injected as ANSI open/close code pairs, matching pi's
//! `SelectListTheme` functions (the coding agent passes `theme.fg(…)`).

use crate::utils::{truncate_to_width, visible_width};

const DEFAULT_PRIMARY_COLUMN_WIDTH: usize = 32;
const PRIMARY_COLUMN_GAP: usize = 2;
const MIN_DESCRIPTION_WIDTH: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// ANSI style slot: open/close code pair (`theme.fg` in pi closes with
/// `\x1b[39m`). Empty open means unstyled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub open: String,
    pub close: String,
}

impl Style {
    fn apply(&self, text: &str) -> String {
        if self.open.is_empty() {
            text.to_owned()
        } else {
            format!("{}{text}{}", self.open, self.close)
        }
    }
}

/// Spec: `SelectListTheme` (`selectedPrefix` exists in the interface but the
/// component renders selection through `selectedText`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectListTheme {
    pub selected_text: Style,
    pub description: Style,
    pub scroll_info: Style,
    pub no_match: Style,
}

/// Spec: `SelectListLayoutOptions` (minus the `truncatePrimary` callback,
/// which nothing in the coding-agent scope passes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectListLayout {
    pub min_primary_column_width: Option<usize>,
    pub max_primary_column_width: Option<usize>,
}

/// Spec: `(?:[\r\n]+)` collapsed to one space, then trimmed.
fn normalize_to_single_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_break = false;
    for ch in text.chars() {
        if ch == '\r' || ch == '\n' {
            if !in_break {
                out.push(' ');
                in_break = true;
            }
        } else {
            out.push(ch);
            in_break = false;
        }
    }
    out.trim().to_owned()
}

#[derive(Clone, Debug)]
pub struct SelectList {
    items: Vec<SelectItem>,
    filtered: Vec<usize>,
    selected: usize,
    max_visible: usize,
    theme: SelectListTheme,
    layout: SelectListLayout,
}

impl SelectList {
    pub fn new(items: Vec<SelectItem>, max_visible: usize) -> Self {
        Self::with_theme(
            items,
            max_visible,
            SelectListTheme::default(),
            SelectListLayout::default(),
        )
    }

    pub fn with_theme(
        items: Vec<SelectItem>,
        max_visible: usize,
        theme: SelectListTheme,
        layout: SelectListLayout,
    ) -> Self {
        let filtered = (0..items.len()).collect();
        Self {
            items,
            filtered,
            selected: 0,
            max_visible: max_visible.max(1),
            theme,
            layout,
        }
    }

    pub fn set_filter(&mut self, filter: &str) {
        let query = filter.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.value.to_lowercase().starts_with(&query))
            .map(|(i, _)| i)
            .collect();
        self.selected = 0;
    }

    pub fn selected(&self) -> Option<&SelectItem> {
        self.filtered
            .get(self.selected)
            .and_then(|i| self.items.get(*i))
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn set_selected_index(&mut self, index: usize) {
        if !self.filtered.is_empty() {
            self.selected = index.min(self.filtered.len() - 1);
        }
    }

    pub fn handle(&mut self, input: &str) -> &'static str {
        if self.filtered.is_empty() {
            return "none";
        }
        match crate::editor::decode_key(input).as_deref() {
            Some("up") => {
                self.selected = if self.selected == 0 {
                    self.filtered.len() - 1
                } else {
                    self.selected - 1
                };
                "changed"
            }
            Some("down") => {
                self.selected = (self.selected + 1) % self.filtered.len();
                "changed"
            }
            Some("enter") => "confirm",
            Some("escape") | Some("ctrl+c") => "cancel",
            _ => "none",
        }
    }

    pub fn render(&self, width: usize) -> Vec<String> {
        if self.filtered.is_empty() {
            return vec![self.theme.no_match.apply("  No matching commands")];
        }
        let primary_column_width = self.primary_column_width();
        let half = self.max_visible / 2;
        let start = (self.selected.saturating_sub(half))
            .min(self.filtered.len().saturating_sub(self.max_visible));
        let end = (start + self.max_visible).min(self.filtered.len());
        let mut lines = Vec::with_capacity(end - start + 1);
        for pos in start..end {
            if let Some(item) = self.filtered.get(pos).and_then(|i| self.items.get(*i)) {
                let description = item.description.as_deref().map(normalize_to_single_line);
                lines.push(self.render_item(
                    item,
                    pos == self.selected,
                    width,
                    description.as_deref(),
                    primary_column_width,
                ));
            }
        }
        if start > 0 || end < self.filtered.len() {
            let scroll_text = format!("  ({}/{})", self.selected + 1, self.filtered.len());
            lines.push(self.theme.scroll_info.apply(&truncate_to_width(
                &scroll_text,
                width.saturating_sub(2),
                "",
                false,
            )));
        }
        lines
    }

    fn render_item(
        &self,
        item: &SelectItem,
        is_selected: bool,
        width: usize,
        description_single_line: Option<&str>,
        primary_column_width: usize,
    ) -> String {
        let prefix = if is_selected { "→ " } else { "  " };
        let prefix_width = 2usize;

        if let Some(description) = description_single_line.filter(|d| !d.is_empty())
            && width > 40
        {
            let effective_primary = primary_column_width
                .min(width.saturating_sub(prefix_width + 4))
                .max(1);
            let max_primary_width = effective_primary.saturating_sub(PRIMARY_COLUMN_GAP).max(1);
            let truncated_value = self.truncate_primary(item, max_primary_width);
            let truncated_value_width = visible_width(&truncated_value);
            let spacing = " ".repeat(
                effective_primary
                    .saturating_sub(truncated_value_width)
                    .max(1),
            );
            let description_start = prefix_width + truncated_value_width + spacing.len();
            // Spec: remainingWidth = width - descriptionStart - 2, compared
            // with a signed >; guard the subtraction accordingly.
            if width as isize - description_start as isize - 2 > MIN_DESCRIPTION_WIDTH as isize {
                let remaining_width = width - description_start - 2;
                let truncated_desc = truncate_to_width(description, remaining_width, "", false);
                if is_selected {
                    return self.theme.selected_text.apply(&format!(
                        "{prefix}{truncated_value}{spacing}{truncated_desc}"
                    ));
                }
                let desc_text = self
                    .theme
                    .description
                    .apply(&format!("{spacing}{truncated_desc}"));
                return format!("{prefix}{truncated_value}{desc_text}");
            }
        }

        let max_width = width.saturating_sub(prefix_width + 2);
        let truncated_value = self.truncate_primary(item, max_width);
        if is_selected {
            return self
                .theme
                .selected_text
                .apply(&format!("{prefix}{truncated_value}"));
        }
        format!("{prefix}{truncated_value}")
    }

    fn primary_column_width(&self) -> usize {
        let (min, max) = self.primary_column_bounds();
        let widest = self
            .filtered
            .iter()
            .filter_map(|i| self.items.get(*i))
            .map(|item| visible_width(Self::display_value(item)) + PRIMARY_COLUMN_GAP)
            .max()
            .unwrap_or(0);
        widest.clamp(min, max)
    }

    fn primary_column_bounds(&self) -> (usize, usize) {
        let raw_min = self
            .layout
            .min_primary_column_width
            .or(self.layout.max_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        let raw_max = self
            .layout
            .max_primary_column_width
            .or(self.layout.min_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        (raw_min.min(raw_max).max(1), raw_min.max(raw_max).max(1))
    }

    fn truncate_primary(&self, item: &SelectItem, max_width: usize) -> String {
        truncate_to_width(Self::display_value(item), max_width, "", false)
    }

    fn display_value(item: &SelectItem) -> &str {
        if item.label.is_empty() {
            &item.value
        } else {
            &item.label
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn items() -> Vec<SelectItem> {
        vec![
            SelectItem {
                value: "alpha".into(),
                label: "alpha".into(),
                description: Some("first".into()),
            },
            SelectItem {
                value: "beta".into(),
                label: "beta".into(),
                description: None,
            },
        ]
    }
    fn list() -> SelectList {
        SelectList::new(items(), 1)
    }
    #[test]
    fn filter_resets_and_wraps() {
        let mut list = list();
        list.set_filter("b");
        assert_eq!(list.selected().map(|x| x.value.as_str()), Some("beta"));
        assert_eq!(list.handle("\x1b[A"), "changed");
        assert_eq!(list.selected_index(), 0);
    }
    #[test]
    fn render_marks_selection_and_scroll() {
        // Narrow width (≤ 40) takes the primary-only fallback.
        let lines = list().render(20);
        assert_eq!(lines, vec!["→ alpha", "  (1/2)"]);
    }
    #[test]
    fn render_columns_description_when_wide() {
        // primaryColumnWidth = max(len("alpha")+2, len("beta")+2) = 7,
        // clamped into [32, 32] -> 32. Selected row styles the whole line.
        let themed = SelectList::with_theme(
            items(),
            5,
            SelectListTheme {
                selected_text: Style {
                    open: "<".into(),
                    close: ">".into(),
                },
                description: Style {
                    open: "[".into(),
                    close: "]".into(),
                },
                ..SelectListTheme::default()
            },
            SelectListLayout::default(),
        );
        let lines = themed.render(60);
        assert_eq!(lines[0], format!("<→ alpha{}first>", " ".repeat(27)));
        assert_eq!(lines[1], "  beta");
    }
    #[test]
    fn render_slash_layout_clamps_primary_column() {
        let themed = SelectList::with_theme(
            items(),
            5,
            SelectListTheme::default(),
            SelectListLayout {
                min_primary_column_width: Some(12),
                max_primary_column_width: Some(32),
            },
        );
        // widest primary = len("alpha") + 2 = 7, clamped into [12, 32] -> 12;
        // spacing = 12 - 5 = 7.
        let lines = themed.render(60);
        assert_eq!(lines[0], format!("→ alpha{}first", " ".repeat(7)));
    }
    #[test]
    fn render_clips_wide_labels_by_terminal_columns() {
        let list = SelectList::new(
            vec![SelectItem {
                value: "wide".into(),
                label: "界界界".into(),
                description: None,
            }],
            5,
        );
        assert!(
            list.render(5)
                .iter()
                .all(|line| crate::utils::visible_width(line) <= 5)
        );
    }
}
