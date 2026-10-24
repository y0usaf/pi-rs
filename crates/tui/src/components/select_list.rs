//! A scrolling list to pick from (upstream `components/select-list.ts`).

use crate::keybindings::get_keybindings;
use crate::tui::{Component, MouseButton, MouseEventType, TuiMouseEvent, TuiMouseEventResult};
use crate::utils::{js_trim, truncate_to_width, visible_width};
use std::rc::Rc;

const DEFAULT_PRIMARY_COLUMN_WIDTH: usize = 32;
const PRIMARY_COLUMN_GAP: usize = 2;
const MIN_DESCRIPTION_WIDTH: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

impl SelectItem {
    pub fn new(value: &str, label: &str, description: Option<&str>) -> Self {
        SelectItem {
            value: value.into(),
            label: label.into(),
            description: description.map(Into::into),
        }
    }
}

pub type Style = Rc<dyn Fn(&str) -> String>;

#[derive(Clone)]
pub struct SelectListTheme {
    pub selected_prefix: Style,
    pub selected_text: Style,
    pub description: Style,
    pub scroll_info: Style,
    pub no_match: Style,
}

/// Arguments to a custom primary-column truncation.
pub struct TruncatePrimaryContext<'a> {
    pub text: &'a str,
    pub max_width: usize,
    pub column_width: usize,
    pub item: &'a SelectItem,
    pub is_selected: bool,
}

#[derive(Clone, Default)]
pub struct SelectListLayoutOptions {
    pub min_primary_column_width: Option<usize>,
    pub max_primary_column_width: Option<usize>,
    pub truncate_primary: Option<TruncatePrimaryFn>,
}

fn single_line(text: &str) -> String {
    let mut out = String::new();
    let mut in_break = false;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
        } else {
            out.push(c);
            in_break = false;
        }
    }
    js_trim(&out).to_string()
}

pub struct SelectList {
    items: Vec<SelectItem>,
    filtered: Vec<SelectItem>,
    selected: usize,
    mouse_pressed: Option<usize>,
    max_visible: usize,
    theme: SelectListTheme,
    layout: SelectListLayoutOptions,
    pub on_select: Option<ItemCallback>,
    pub on_cancel: Option<Box<dyn FnMut()>>,
    pub on_selection_change: Option<ItemCallback>,
}

impl SelectList {
    pub fn new(
        items: Vec<SelectItem>,
        max_visible: usize,
        theme: SelectListTheme,
        layout: SelectListLayoutOptions,
    ) -> Self {
        SelectList {
            filtered: items.clone(),
            items,
            selected: 0,
            mouse_pressed: None,
            max_visible,
            theme,
            layout,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
        }
    }

    /// Keep items whose value starts with `filter` (case-insensitive) and select the first.
    pub fn set_filter(&mut self, filter: &str) {
        let f = filter.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .filter(|i| i.value.to_lowercase().starts_with(&f))
            .cloned()
            .collect();
        self.selected = 0;
    }

    pub fn set_selected_index(&mut self, index: usize) {
        self.selected = index.min(self.filtered.len().saturating_sub(1));
    }

    pub fn selected_item(&self) -> Option<&SelectItem> {
        self.filtered.get(self.selected)
    }

    fn visible_range(&self) -> (usize, usize) {
        let len = self.filtered.len() as i64;
        let mv = self.max_visible as i64;
        let start = (self.selected as i64 - mv.div_euclid(2))
            .min(len - mv)
            .max(0);
        (start as usize, (start + mv).min(len) as usize)
    }

    fn display_value(item: &SelectItem) -> &str {
        if item.label.is_empty() {
            &item.value
        } else {
            &item.label
        }
    }

    fn column_bounds(&self) -> (usize, usize) {
        let l = &self.layout;
        let raw_min = l
            .min_primary_column_width
            .or(l.max_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        let raw_max = l
            .max_primary_column_width
            .or(l.min_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        (raw_min.min(raw_max).max(1), raw_min.max(raw_max).max(1))
    }

    fn primary_column_width(&self) -> usize {
        let (min, max) = self.column_bounds();
        let widest = self
            .filtered
            .iter()
            .map(|i| visible_width(Self::display_value(i)) + PRIMARY_COLUMN_GAP)
            .max()
            .unwrap_or(0);
        widest.clamp(min, max)
    }

    fn truncate_primary(
        &self,
        item: &SelectItem,
        is_selected: bool,
        max_width: i64,
        column_width: i64,
    ) -> String {
        let dv = Self::display_value(item);
        let mw = max_width.max(0) as usize;
        let t = match &self.layout.truncate_primary {
            Some(f) => f(&TruncatePrimaryContext {
                text: dv,
                max_width: mw,
                column_width: column_width.max(0) as usize,
                item,
                is_selected,
            }),
            None => truncate_to_width(dv, mw, "", false),
        };
        truncate_to_width(&t, mw, "", false)
    }

    fn render_item(
        &self,
        item: &SelectItem,
        is_selected: bool,
        width: usize,
        desc: Option<&str>,
        primary_width: usize,
    ) -> String {
        let prefix = if is_selected { "→ " } else { "  " };
        let pw = visible_width(prefix) as i64;
        let w = width as i64;
        if let Some(desc) = desc.filter(|d| !d.is_empty())
            && width > 40
        {
            let eff = (primary_width as i64).min(w - pw - 4).max(1);
            let max_primary = (eff - PRIMARY_COLUMN_GAP as i64).max(1);
            let value = self.truncate_primary(item, is_selected, max_primary, eff);
            let vw = visible_width(&value) as i64;
            let spacing = " ".repeat((eff - vw).max(1) as usize);
            let start = pw + vw + spacing.len() as i64;
            let remaining = w - start - 2;
            if remaining > MIN_DESCRIPTION_WIDTH {
                let d = truncate_to_width(desc, remaining as usize, "", false);
                if is_selected {
                    return (self.theme.selected_text)(&format!("{prefix}{value}{spacing}{d}"));
                }
                return format!(
                    "{prefix}{value}{}",
                    (self.theme.description)(&format!("{spacing}{d}"))
                );
            }
        }
        let max_width = w - pw - 2;
        let value = self.truncate_primary(item, is_selected, max_width, max_width);
        if is_selected {
            return (self.theme.selected_text)(&format!("{prefix}{value}"));
        }
        format!("{prefix}{value}")
    }

    fn notify_selection_change(&mut self) {
        if let Some(item) = self.filtered.get(self.selected).cloned()
            && let Some(f) = self.on_selection_change.as_mut()
        {
            f(&item);
        }
    }
}

impl Component for SelectList {
    fn render(&mut self, width: usize) -> Vec<String> {
        if self.filtered.is_empty() {
            return vec![(self.theme.no_match)("  No matching commands")];
        }
        let pw = self.primary_column_width();
        let (start, end) = self.visible_range();
        let mut lines = Vec::new();
        for i in start..end {
            let item = &self.filtered[i];
            let desc = item.description.as_deref().map(single_line);
            lines.push(self.render_item(item, i == self.selected, width, desc.as_deref(), pw));
        }
        if start > 0 || end < self.filtered.len() {
            let text = format!("  ({}/{})", self.selected + 1, self.filtered.len());
            lines.push((self.theme.scroll_info)(&truncate_to_width(
                &text,
                width.saturating_sub(2),
                "",
                false,
            )));
        }
        lines
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        let n = self.filtered.len();
        if kb.matches(data, "tui.select.up") {
            self.selected = if self.selected == 0 {
                n.saturating_sub(1)
            } else {
                self.selected - 1
            };
            self.notify_selection_change();
        } else if kb.matches(data, "tui.select.down") {
            self.selected = if n == 0 || self.selected == n - 1 {
                0
            } else {
                self.selected + 1
            };
            self.notify_selection_change();
        } else if kb.matches(data, "tui.select.confirm") {
            if let Some(item) = self.filtered.get(self.selected).cloned()
                && let Some(f) = self.on_select.as_mut()
            {
                f(&item);
            }
        } else if kb.matches(data, "tui.select.cancel")
            && let Some(f) = self.on_cancel.as_mut()
        {
            f();
        }
    }

    fn handle_mouse(&mut self, e: &TuiMouseEvent) -> Option<TuiMouseEventResult> {
        if self.filtered.is_empty() {
            return None;
        }
        if e.kind == MouseEventType::Wheel && e.wheel_delta.is_some_and(|d| d != 0) {
            let d: i64 = if e.wheel_delta.unwrap() < 0 { -1 } else { 1 };
            let prev = self.selected;
            self.selected =
                (self.selected as i64 + d).clamp(0, self.filtered.len() as i64 - 1) as usize;
            if self.selected != prev {
                self.notify_selection_change();
            }
            return Some(TuiMouseEventResult {
                handled: true,
                render: Some(self.selected != prev),
                ..Default::default()
            });
        }
        if e.button != MouseButton::Left
            || !matches!(e.kind, MouseEventType::Press | MouseEventType::Click)
        {
            return None;
        }
        let (start, end) = self.visible_range();
        let idx = start as i64 + e.y;
        if idx < start as i64 || idx >= end as i64 {
            return None;
        }
        let idx = idx as usize;
        if e.kind == MouseEventType::Press {
            self.mouse_pressed = Some(idx);
            if self.selected != idx {
                self.selected = idx;
                self.notify_selection_change();
            }
            return Some(TuiMouseEventResult {
                handled: true,
                focus: true,
                ..Default::default()
            });
        }
        let clicked = self.mouse_pressed.take().unwrap_or(idx);
        let changed = self.selected != clicked;
        self.selected = clicked;
        if changed {
            self.notify_selection_change();
        }
        if let Some(item) = self.filtered.get(self.selected).cloned()
            && let Some(f) = self.on_select.as_mut()
        {
            f(&item);
        }
        Some(TuiMouseEventResult {
            handled: true,
            ..Default::default()
        })
    }
}

/// Truncates a primary column.
pub type TruncatePrimaryFn = Rc<dyn Fn(&TruncatePrimaryContext) -> String>;
/// A callback given an item.
pub type ItemCallback = Box<dyn FnMut(&SelectItem)>;
