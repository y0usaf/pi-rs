//! Cursor movement: graphemes, words, visual lines, pages, history, character jumps.

use super::text::is_paste_marker;
use super::{Editor, Jump, VisualLine};
use crate::unicode::{WordSegment, utf16_len};
use crate::word_navigation::{WordNavigationOptions, find_word_backward, find_word_forward};

impl Editor {
    /// Set the cursor column and drop the sticky column.
    pub(crate) fn set_cursor_col(&mut self, col: usize) {
        self.state.cursor_col = col;
        self.preferred_visual_col = None;
        self.snapped_from_cursor_col = None;
    }

    pub(crate) fn current_line(&self) -> &str {
        self.state
            .lines
            .get(self.state.cursor_line)
            .map_or("", String::as_str)
    }

    pub(crate) fn text_before_cursor(&self) -> String {
        let l = self.current_line();
        l[..self.state.cursor_col.min(l.len())].to_string()
    }

    pub(crate) fn last_grapheme_len(&self, s: &str) -> usize {
        self.seg_graphemes(s).last().map_or(1, |g| g.segment.len())
    }

    pub(crate) fn first_grapheme_len(&self, s: &str) -> usize {
        self.seg_graphemes(s).first().map_or(1, |g| g.segment.len())
    }

    pub(crate) fn is_editor_empty(&self) -> bool {
        self.state.lines.len() == 1 && self.state.lines[0].is_empty()
    }

    pub(crate) fn is_on_first_visual_line(&self) -> bool {
        let vls = self.build_visual_line_map(self.last_width);
        self.find_current_visual_line(&vls) == 0
    }

    pub(crate) fn is_on_last_visual_line(&self) -> bool {
        let vls = self.build_visual_line_map(self.last_width);
        self.find_current_visual_line(&vls) + 1 == vls.len()
    }

    fn compute_vertical_move_column(
        &mut self,
        cur: usize,
        source_max: usize,
        target_max: usize,
    ) -> usize {
        let in_middle = cur < source_max;
        let too_short = target_max < cur;
        match self.preferred_visual_col {
            Some(p) if !in_middle => {
                if too_short || target_max < p {
                    target_max
                } else {
                    self.preferred_visual_col = None;
                    p
                }
            }
            _ => {
                if too_short {
                    self.preferred_visual_col = Some(cur);
                    target_max
                } else {
                    self.preferred_visual_col = None;
                    cur
                }
            }
        }
    }

    /// Visual lines measured in UTF-16 units, as upstream measures columns.
    fn visual_lines_u16(&self, vls: &[VisualLine]) -> Vec<VisualLine> {
        vls.iter()
            .map(|vl| {
                let line = &self.state.lines[vl.logical_line];
                VisualLine {
                    logical_line: vl.logical_line,
                    start_col: utf16_len(&line[..vl.start_col]),
                    length: utf16_len(&line[vl.start_col..vl.start_col + vl.length]),
                }
            })
            .collect()
    }

    /// Upstream `moveToVisualLine`; columns, the sticky column and the snap origin are UTF-16 units.
    pub(crate) fn move_to_visual_line(
        &mut self,
        vls: &[VisualLine],
        cur_i: usize,
        target_i: usize,
    ) {
        let u = self.visual_lines_u16(vls);
        let (Some(&cur), Some(&target)) = (u.get(cur_i), u.get(target_i)) else {
            return;
        };
        let cursor16 = utf16_len(&self.current_line()[..self.state.cursor_col]);
        let cur_col = match self.snapped_from_cursor_col {
            Some(s) => {
                let i = Self::find_visual_line_at(&u, cur.logical_line, s);
                s as i64 - u[i].start_col as i64
            }
            None => cursor16 as i64 - cur.start_col as i64,
        };
        let cur_col = cur_col.max(0) as usize;
        let max_col = |i: usize, vl: VisualLine| {
            if Self::is_last_segment(&u, i) {
                vl.length
            } else {
                vl.length.saturating_sub(1)
            }
        };
        let col = self.compute_vertical_move_column(
            cur_col,
            max_col(cur_i, cur),
            max_col(target_i, target),
        );
        self.state.cursor_line = target.logical_line;
        let line = self
            .state
            .lines
            .get(target.logical_line)
            .cloned()
            .unwrap_or_default();
        let c16 = (target.start_col + col).min(utf16_len(&line));
        self.state.cursor_col = byte_floor(&line, c16);
        let mut pos16 = 0;
        for seg in self.seg_graphemes(&line) {
            let seg16 = utf16_len(seg.segment);
            let start16 = pos16;
            pos16 += seg16;
            if start16 > c16 {
                break;
            }
            if seg16 <= 1 {
                continue;
            }
            if c16 < start16 + seg16 {
                if start16 < target.start_col && target_i > cur_i {
                    let seg_end = start16 + seg16;
                    let mut next = target_i + 1;
                    while next < u.len()
                        && u[next].logical_line == target.logical_line
                        && u[next].start_col < seg_end
                    {
                        next += 1;
                    }
                    if next < u.len() {
                        self.move_to_visual_line(vls, cur_i, next);
                        return;
                    }
                }
                self.snapped_from_cursor_col = Some(c16);
                self.state.cursor_col = seg.index;
                return;
            }
        }
        self.snapped_from_cursor_col = None;
    }

    pub(crate) fn move_cursor(&mut self, delta_line: i64, delta_col: i64) {
        self.last_action = None;
        let vls = self.build_visual_line_map(self.last_width);
        let cur = self.find_current_visual_line(&vls);
        if delta_line != 0 {
            let t = cur as i64 + delta_line;
            if t >= 0 && (t as usize) < vls.len() {
                self.move_to_visual_line(&vls, cur, t as usize);
            }
        }
        if delta_col > 0 {
            let line = self.current_line().to_string();
            if self.state.cursor_col < line.len() {
                let n = self.first_grapheme_len(&line[self.state.cursor_col..]);
                self.set_cursor_col(self.state.cursor_col + n);
            } else if self.state.cursor_line + 1 < self.state.lines.len() {
                self.state.cursor_line += 1;
                self.set_cursor_col(0);
            } else if let Some(vl) = vls.get(cur) {
                let line = self.current_line();
                let col16 = utf16_len(&line[..self.state.cursor_col])
                    - utf16_len(&line[..vl.start_col.min(self.state.cursor_col)]);
                self.preferred_visual_col = Some(col16);
            }
        } else if delta_col < 0 {
            if self.state.cursor_col > 0 {
                let before = self.text_before_cursor();
                let n = self.last_grapheme_len(&before);
                self.set_cursor_col(self.state.cursor_col - n);
            } else if self.state.cursor_line > 0 {
                self.state.cursor_line -= 1;
                self.set_cursor_col(self.current_line().len());
            }
        }
        if self.autocomplete_state.is_some() {
            self.update_autocomplete();
        }
    }

    pub(crate) fn page_scroll(&mut self, direction: i64) {
        self.last_action = None;
        let page = self.max_visible_lines() as i64;
        let vls = self.build_visual_line_map(self.last_width);
        let cur = self.find_current_visual_line(&vls);
        let t = (cur as i64 + direction * page)
            .clamp(0, vls.len() as i64 - 1)
            .max(0) as usize;
        self.move_to_visual_line(&vls, cur, t);
    }

    pub(crate) fn move_to_line_start(&mut self) {
        self.last_action = None;
        self.set_cursor_col(0);
    }

    pub(crate) fn move_to_line_end(&mut self) {
        self.last_action = None;
        self.set_cursor_col(self.current_line().len());
    }

    fn word_target(&self, forward: bool) -> usize {
        let line = self.current_line();
        let seg = coerce(|t| self.seg_words(t));
        let o = WordNavigationOptions {
            segment: Some(&seg),
            is_atomic_segment: Some(&is_paste_marker),
        };
        if forward {
            find_word_forward(line, self.state.cursor_col, &o)
        } else {
            find_word_backward(line, self.state.cursor_col, &o)
        }
    }

    pub(crate) fn move_word_backwards(&mut self) {
        self.last_action = None;
        if self.state.cursor_col == 0 {
            if self.state.cursor_line > 0 {
                self.state.cursor_line -= 1;
                self.set_cursor_col(self.current_line().len());
            }
            return;
        }
        let c = self.word_target(false);
        self.set_cursor_col(c);
    }

    pub(crate) fn move_word_forwards(&mut self) {
        self.last_action = None;
        if self.state.cursor_col >= self.current_line().len() {
            if self.state.cursor_line + 1 < self.state.lines.len() {
                self.state.cursor_line += 1;
                self.set_cursor_col(0);
            }
            return;
        }
        let c = self.word_target(true);
        self.set_cursor_col(c);
    }

    /// Jump to the next (or previous) occurrence of `ch`, across lines; case-sensitive.
    pub(crate) fn jump_to_char(&mut self, ch: &str, dir: Jump) {
        self.last_action = None;
        let forward = dir == Jump::Forward;
        let n = self.state.lines.len() as i64;
        let mut li = self.state.cursor_line as i64;
        while li >= 0 && li < n {
            let line = &self.state.lines[li as usize];
            let current = li as usize == self.state.cursor_line;
            let found = if forward {
                let from = if current {
                    self.state.cursor_col + 1
                } else {
                    0
                };
                if from > line.len() {
                    None
                } else {
                    find_from(line, ch, from)
                }
            } else if current {
                // lastIndexOf with a negative start only matches at 0.
                match self.state.cursor_col.checked_sub(1) {
                    Some(from) => rfind_upto(line, ch, from),
                    None => line.starts_with(ch).then_some(0),
                }
            } else {
                line.rfind(ch)
            };
            if let Some(i) = found {
                self.state.cursor_line = li as usize;
                self.set_cursor_col(i);
                return;
            }
            li += if forward { 1 } else { -1 };
        }
    }

    pub(crate) fn navigate_history(&mut self, direction: i64) {
        self.last_action = None;
        if self.history.is_empty() {
            return;
        }
        let ni = self.history_index - direction;
        if ni < -1 || ni >= self.history.len() as i64 {
            return;
        }
        if self.history_index == -1 && ni >= 0 {
            self.push_undo_snapshot();
            self.history_draft = Some(self.state.clone());
        }
        self.history_index = ni;
        if ni == -1 {
            match self.history_draft.take() {
                Some(d) => {
                    self.state = d;
                    self.preferred_visual_col = None;
                    self.snapped_from_cursor_col = None;
                    self.scroll_offset = 0;
                    self.emit_change();
                }
                None => self.set_text_internal("", false),
            }
        } else {
            let t = self.history[ni as usize].clone();
            self.set_text_internal(&t, direction == -1);
        }
    }

    pub(crate) fn exit_history_browsing(&mut self) {
        self.history_index = -1;
        self.history_draft = None;
    }

    /// Replace the text without touching history; cursor at the start or the end.
    pub(crate) fn set_text_internal(&mut self, text: &str, at_start: bool) {
        self.state.lines = text.split('\n').map(str::to_string).collect();
        self.state.cursor_line = if at_start {
            0
        } else {
            self.state.lines.len() - 1
        };
        let col = if at_start {
            0
        } else {
            self.current_line().len()
        };
        self.set_cursor_col(col);
        self.scroll_offset = 0;
        self.emit_change();
    }
}

fn coerce<F: for<'a> Fn(&'a str) -> Vec<WordSegment<'a>>>(f: F) -> F {
    f
}

/// `line.indexOf(ch, from)` with a byte `from` (snapped forward to a char boundary).
fn find_from(line: &str, ch: &str, mut from: usize) -> Option<usize> {
    while from < line.len() && !line.is_char_boundary(from) {
        from += 1;
    }
    line[from..].find(ch).map(|i| i + from)
}

/// `line.lastIndexOf(ch, from)`: the last match starting at or before `from`.
fn rfind_upto(line: &str, ch: &str, from: usize) -> Option<usize> {
    line.match_indices(ch)
        .map(|(i, _)| i)
        .take_while(|&i| i <= from)
        .last()
}

/// Byte offset of UTF-16 offset `u16_off`, rounded down to a char boundary.
fn byte_floor(t: &str, u16_off: usize) -> usize {
    let mut units = 0;
    for (i, c) in t.char_indices() {
        units += c.len_utf16();
        if units > u16_off {
            return i;
        }
    }
    t.len()
}
