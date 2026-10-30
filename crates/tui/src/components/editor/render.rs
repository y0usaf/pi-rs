//! Layout, rendering and mouse handling.

use super::text::{create_scroll_border, word_wrap_line};
use super::{Editor, VisualLine};
use crate::tui::{
    CURSOR_MARKER, Component, MouseButton, MouseEventType, TuiMouseEvent, TuiMouseEventResult,
    render_fake_cursor,
};
use crate::utils::visible_width;

pub(crate) struct LayoutLine {
    text: String,
    cursor_pos: Option<usize>,
}

fn handled() -> Option<TuiMouseEventResult> {
    Some(TuiMouseEventResult {
        handled: true,
        focus: true,
        ..Default::default()
    })
}

impl Editor {
    fn effective_padding(&self, width: usize) -> usize {
        self.padding_x.min(width.saturating_sub(1) / 2)
    }

    pub(crate) fn max_visible_lines(&self) -> usize {
        (self.terminal_rows * 3 / 10).max(5)
    }

    pub(crate) fn render_top_border(&self, width: usize, hidden: usize) -> String {
        if let Some(line) = self
            .top_border
            .as_ref()
            .and_then(|hook| hook(width, hidden, &self.border_color))
        {
            return line;
        }
        let b = if hidden > 0 {
            create_scroll_border("↑", hidden, width)
        } else {
            "─".repeat(width)
        };
        (self.border_color)(&b)
    }

    pub(crate) fn render_bottom_border(&self, width: usize, hidden: usize) -> String {
        let b = if hidden > 0 {
            create_scroll_border("↓", hidden, width)
        } else {
            "─".repeat(width)
        };
        (self.border_color)(&b)
    }

    fn layout_text(&self, content_width: usize) -> Vec<LayoutLine> {
        let st = &self.state;
        if st.lines.is_empty() || (st.lines.len() == 1 && st.lines[0].is_empty()) {
            return vec![LayoutLine {
                text: String::new(),
                cursor_pos: Some(0),
            }];
        }
        let mut out = Vec::new();
        for (i, line) in st.lines.iter().enumerate() {
            let current = i == st.cursor_line;
            if visible_width(line) <= content_width {
                out.push(LayoutLine {
                    text: line.clone(),
                    cursor_pos: current.then_some(st.cursor_col),
                });
                continue;
            }
            let segs = self.seg_graphemes(line);
            let chunks = word_wrap_line(line, content_width, Some(&segs));
            let n = chunks.len();
            for (ci, c) in chunks.into_iter().enumerate() {
                let cp = st.cursor_col;
                let mut pos = None;
                if current {
                    if ci == n - 1 {
                        if cp >= c.start_index {
                            pos = Some(cp - c.start_index);
                        }
                    } else if cp >= c.start_index && cp < c.end_index {
                        pos = Some((cp - c.start_index).min(c.text.len()));
                    }
                }
                out.push(LayoutLine {
                    text: c.text,
                    cursor_pos: pos,
                });
            }
        }
        out
    }

    /// Upstream `buildVisualLineMap`.
    pub(crate) fn build_visual_line_map(&self, width: usize) -> Vec<VisualLine> {
        let mut out = Vec::new();
        for (i, line) in self.state.lines.iter().enumerate() {
            if line.is_empty() {
                out.push(VisualLine {
                    logical_line: i,
                    start_col: 0,
                    length: 0,
                });
            } else if visible_width(line) <= width {
                out.push(VisualLine {
                    logical_line: i,
                    start_col: 0,
                    length: line.len(),
                });
            } else {
                let segs = self.seg_graphemes(line);
                for c in word_wrap_line(line, width, Some(&segs)) {
                    out.push(VisualLine {
                        logical_line: i,
                        start_col: c.start_index,
                        length: c.end_index - c.start_index,
                    });
                }
            }
        }
        out
    }

    pub(crate) fn is_last_segment(vls: &[VisualLine], i: usize) -> bool {
        i + 1 >= vls.len() || vls[i + 1].logical_line != vls[i].logical_line
    }

    /// Upstream `findVisualLineAt` (returns `len - 1` when not found, as upstream does).
    pub(crate) fn find_visual_line_at(vls: &[VisualLine], line: usize, col: usize) -> usize {
        for (i, vl) in vls.iter().enumerate() {
            if vl.logical_line != line || col < vl.start_col {
                continue;
            }
            let off = col - vl.start_col;
            if off < vl.length || (Self::is_last_segment(vls, i) && off == vl.length) {
                return i;
            }
        }
        vls.len().saturating_sub(1)
    }

    pub(crate) fn find_current_visual_line(&self, vls: &[VisualLine]) -> usize {
        Self::find_visual_line_at(vls, self.state.cursor_line, self.state.cursor_col)
    }
}

impl Component for Editor {
    fn render(&mut self, width: usize) -> Vec<String> {
        let padding_x = self.effective_padding(width);
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        let layout_width = content_width
            .saturating_sub(if padding_x > 0 { 0 } else { 1 })
            .max(1);
        self.last_width = layout_width;
        let layout = self.layout_text(layout_width);
        let max_visible = self.max_visible_lines();
        let cursor_idx = layout
            .iter()
            .position(|l| l.cursor_pos.is_some())
            .unwrap_or(0);
        if cursor_idx < self.scroll_offset {
            self.scroll_offset = cursor_idx;
        } else if cursor_idx >= self.scroll_offset + max_visible {
            self.scroll_offset = cursor_idx + 1 - max_visible;
        }
        self.scroll_offset = self
            .scroll_offset
            .min(layout.len().saturating_sub(max_visible));
        let end = (self.scroll_offset + max_visible).min(layout.len());
        let visible = &layout[self.scroll_offset..end];
        self.rendered_visible_line_count = visible.len();

        let left = " ".repeat(padding_x);
        let mut result = vec![self.render_top_border(width, self.scroll_offset)];
        for ll in visible {
            let mut display = ll.text.clone();
            let mut line_width = visible_width(&ll.text);
            let mut cursor_in_padding = false;
            if let Some(pos) = ll.cursor_pos {
                let pos = pos.min(ll.text.len());
                let (before, after) = ll.text.split_at(pos);
                let marker = if self.focused { CURSOR_MARKER } else { "" };
                if !after.is_empty() {
                    let first = self.seg_graphemes(after).first().map_or("", |s| s.segment);
                    display = format!(
                        "{before}{marker}{}{}",
                        render_fake_cursor(first),
                        &after[first.len()..]
                    );
                } else {
                    display = format!("{before}{marker}{}", render_fake_cursor(" "));
                    line_width += 1;
                    if line_width > content_width && padding_x > 0 {
                        cursor_in_padding = true;
                    }
                }
            }
            let pad = " ".repeat(content_width.saturating_sub(line_width));
            let right = if cursor_in_padding {
                &left[1..]
            } else {
                &left[..]
            };
            result.push(format!("{left}{display}{pad}{right}"));
        }
        let below = layout.len() - end;
        result.push(self.render_bottom_border(width, below));

        self.rendered_autocomplete_height = 0;
        if self.autocomplete_state.is_some()
            && let Some(list) = self.autocomplete_list.as_mut()
        {
            let lines = list.render(content_width);
            self.rendered_autocomplete_height = lines.len();
            for line in lines {
                let pad = " ".repeat(content_width.saturating_sub(visible_width(&line)));
                result.push(format!("{left}{line}{pad}{left}"));
            }
        }
        result
    }

    fn handle_input(&mut self, data: &str) {
        self.handle_key(data);
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_mouse(&mut self, e: &TuiMouseEvent) -> Option<TuiMouseEventResult> {
        let ac_start = self.rendered_visible_line_count as i64 + 2;
        let width = e.width.max(0) as usize;
        let padding_x = self.effective_padding(width);
        if self.autocomplete_state.is_some()
            && self.autocomplete_list.is_some()
            && e.y >= ac_start
            && e.y < ac_start + self.rendered_autocomplete_height as i64
        {
            let content_width = width.saturating_sub(padding_x * 2).max(1);
            let ev = TuiMouseEvent {
                x: e.x - padding_x as i64,
                y: e.y - ac_start,
                width: content_width as i64,
                height: self.rendered_autocomplete_height as i64,
                ..*e
            };
            let r = self
                .autocomplete_list
                .as_mut()
                .and_then(|l| l.handle_mouse(&ev));
            self.apply_picked();
            return r.map(|r| TuiMouseEventResult { focus: true, ..r });
        }
        if e.kind != MouseEventType::Click || e.button != MouseButton::Left {
            return None;
        }
        if e.y <= 0 || e.y > self.rendered_visible_line_count as i64 {
            return handled();
        }
        let vls = self.build_visual_line_map(self.last_width);
        let vi = self.scroll_offset + e.y as usize - 1;
        let Some(&vl) = vls.get(vi) else {
            return handled();
        };
        let logical = self
            .state
            .lines
            .get(vl.logical_line)
            .cloned()
            .unwrap_or_default();
        let chunk = &logical[vl.start_col..vl.start_col + vl.length];
        let target = (e.x - padding_x as i64).max(0) as usize;
        let mut col = 0;
        let mut idx = chunk.len();
        let mut last_idx = 0;
        for g in self.seg_graphemes(chunk) {
            let next = col + visible_width(g.segment);
            last_idx = g.index;
            if target < next {
                idx = g.index;
                break;
            }
            col = next;
        }
        if !Self::is_last_segment(&vls, vi) && idx == chunk.len() && !chunk.is_empty() {
            idx = last_idx;
        }
        self.state.cursor_line = vl.logical_line;
        self.set_cursor_col(vl.start_col + idx);
        self.last_action = None;
        self.exit_history_browsing();
        if self.autocomplete_state.is_some() {
            self.update_autocomplete();
        }
        handled()
    }
}
