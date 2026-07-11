//! Terminal-independent TUI lifecycle and differential rendering.
//!
//! This is the render mechanism from pi's `tui.ts` expressed as a
//! deterministic state machine.  A host supplies rendered component lines and
//! decides when to service a coalesced render request; no product policy lives
//! here.  The differential algorithm, viewport tracking, per-line resets, and
//! hardware-cursor positioning are a 1:1 port of `TUI.doRender`.

use crate::terminal::TerminalState;
use crate::terminal_image::is_image_line;
use crate::utils::{normalize_terminal_output, visible_width};

/// Zero-width marker emitted by a focused component at its logical cursor.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// Reset appended to every rendered line (pi `TUI.SEGMENT_RESET`).
pub const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPosition {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderState {
    Idle,
    Requested,
    ForceRequested,
    Stopped,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TuiError {
    #[error("rendered line {line} exceeds terminal width ({actual} > {width})")]
    LineTooWide {
        line: usize,
        actual: usize,
        width: usize,
    },
}

/// Find the last cursor marker in the visible viewport and remove it.
///
/// Like upstream, markers above the bottom `height` lines are left untouched.
pub fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<CursorPosition> {
    let viewport_top = lines.len().saturating_sub(height);
    for row in (viewport_top..lines.len()).rev() {
        if let Some(marker) = lines[row].find(CURSOR_MARKER) {
            let column = visible_width(&lines[row][..marker]);
            lines[row].replace_range(marker..marker + CURSOR_MARKER.len(), "");
            return Some(CursorPosition { row, column });
        }
    }
    None
}

/// Deterministic process-loop state for a TUI.
///
/// `request_render` coalesces arbitrary wakeups. `render_if_requested` is the
/// event-loop boundary: the caller computes product-owned component lines and
/// supplies them only when it is ready to flush a frame.
pub struct Tui {
    terminal: TerminalState,
    render_state: RenderState,
    previous_lines: Vec<String>,
    /// `0` = never rendered, `-1` = force-invalidated (pi uses `-1` too).
    previous_width: i64,
    previous_height: i64,
    cursor_row: usize,
    hardware_cursor_row: usize,
    max_lines_rendered: usize,
    previous_viewport_top: usize,
    show_hardware_cursor: bool,
    clear_on_shrink: bool,
    full_redraws: usize,
}

impl Tui {
    pub fn new(terminal: TerminalState, show_hardware_cursor: bool) -> Self {
        Self {
            terminal,
            render_state: RenderState::Stopped,
            previous_lines: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            cursor_row: 0,
            hardware_cursor_row: 0,
            max_lines_rendered: 0,
            previous_viewport_top: 0,
            show_hardware_cursor,
            clear_on_shrink: std::env::var("PI_CLEAR_ON_SHRINK").is_ok_and(|v| v == "1"),
            full_redraws: 0,
        }
    }

    pub fn start(&mut self) {
        self.terminal.start();
        self.terminal.hide_cursor();
        self.render_state = RenderState::Requested;
    }

    pub fn render_state(&self) -> RenderState {
        self.render_state
    }

    pub fn request_render(&mut self, force: bool) {
        if self.render_state == RenderState::Stopped {
            return;
        }
        if force {
            self.previous_lines = Vec::new();
            self.previous_width = -1;
            self.previous_height = -1;
            self.cursor_row = 0;
            self.hardware_cursor_row = 0;
            self.max_lines_rendered = 0;
            self.previous_viewport_top = 0;
            self.render_state = RenderState::ForceRequested;
        } else if self.render_state == RenderState::Idle {
            self.render_state = RenderState::Requested;
        }
    }

    /// Feed terminal input, returning product-owned input events to the host.
    /// Any delivered input requests one coalesced render, matching `tui.ts`.
    pub fn feed_input(&mut self, bytes: &[u8]) -> Vec<String> {
        if self.render_state == RenderState::Stopped {
            return Vec::new();
        }
        let input = self.terminal.feed_input(bytes);
        if !input.is_empty() {
            self.request_render(false);
        }
        input
    }

    pub fn flush_input(&mut self) -> Vec<String> {
        if self.render_state == RenderState::Stopped {
            return Vec::new();
        }
        let input = self.terminal.flush_input();
        if !input.is_empty() {
            self.request_render(false);
        }
        input
    }

    pub fn resize(&mut self, columns: Option<u16>, rows: Option<u16>) {
        if self.render_state == RenderState::Stopped {
            return;
        }
        self.terminal.resize(columns, rows);
        self.request_render(false);
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.terminal.columns(), self.terminal.rows())
    }

    pub fn keyboard_negotiation_pending(&self) -> bool {
        self.terminal.keyboard_negotiation_pending()
    }

    pub fn flush_keyboard_negotiation(&mut self) -> Vec<String> {
        let input = self.terminal.flush_keyboard_negotiation();
        if !input.is_empty() {
            self.request_render(false);
        }
        input
    }

    pub fn begin_drain(&mut self) {
        self.terminal.begin_drain();
    }

    pub fn set_title(&mut self, title: &str) {
        self.terminal.set_title(title);
    }

    pub fn set_progress(&mut self, active: bool) {
        self.terminal.set_progress(active);
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.show_hardware_cursor = enabled;
        self.request_render(false);
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.clear_on_shrink = enabled;
    }

    /// Flush one requested frame. Returns `Ok(false)` when no frame was due.
    pub fn render_if_requested(&mut self, lines: Vec<String>) -> Result<bool, TuiError> {
        if matches!(self.render_state, RenderState::Idle | RenderState::Stopped) {
            return Ok(false);
        }
        self.render_state = RenderState::Idle;
        self.do_render(lines)?;
        Ok(true)
    }

    fn apply_line_resets(lines: &mut [String]) {
        for line in lines.iter_mut() {
            if !is_image_line(line) {
                let mut next = normalize_terminal_output(line);
                next.push_str(SEGMENT_RESET);
                *line = next;
            }
        }
    }

    fn check_line_widths(lines: &[String], width: usize) -> Result<(), TuiError> {
        for (line, text) in lines.iter().enumerate() {
            if is_image_line(text) {
                continue;
            }
            let actual = visible_width(text);
            if actual > width {
                return Err(TuiError::LineTooWide {
                    line,
                    actual,
                    width,
                });
            }
        }
        Ok(())
    }

    fn full_render(
        &mut self,
        clear: bool,
        new_lines: Vec<String>,
        cursor: Option<CursorPosition>,
        width: usize,
        height: usize,
    ) {
        self.full_redraws += 1;
        let mut buffer = String::from("\x1b[?2026h");
        if clear {
            buffer.push_str("\x1b[2J\x1b[H\x1b[3J");
        }
        for (i, line) in new_lines.iter().enumerate() {
            if i > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str(line);
        }
        buffer.push_str("\x1b[?2026l");
        self.terminal.write(&buffer);
        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = self.cursor_row;
        if clear {
            self.max_lines_rendered = new_lines.len();
        } else {
            self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        }
        let buffer_length = height.max(new_lines.len());
        self.previous_viewport_top = buffer_length.saturating_sub(height);
        self.position_hardware_cursor(cursor, new_lines.len());
        self.previous_lines = new_lines;
        self.previous_width = width as i64;
        self.previous_height = height as i64;
    }

    fn do_render(&mut self, mut new_lines: Vec<String>) -> Result<(), TuiError> {
        let width = usize::from(self.terminal.columns());
        let height = usize::from(self.terminal.rows());
        let width_changed = self.previous_width != 0 && self.previous_width != width as i64;
        let height_changed = self.previous_height != 0 && self.previous_height != height as i64;
        let previous_buffer_length = if self.previous_height > 0 {
            self.previous_viewport_top + self.previous_height as usize
        } else {
            height
        };
        let mut prev_viewport_top = if height_changed {
            previous_buffer_length.saturating_sub(height)
        } else {
            self.previous_viewport_top
        };
        let mut viewport_top = prev_viewport_top;
        let mut hardware_cursor_row = self.hardware_cursor_row as i64;

        Self::check_line_widths(&new_lines, width)?;
        let cursor = extract_cursor_position(&mut new_lines, height);
        Self::apply_line_resets(&mut new_lines);

        // First render - just output everything without clearing.
        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            self.full_render(false, new_lines, cursor, width, height);
            return Ok(());
        }

        // Width changes always need a full re-render because wrapping changes.
        // Height changes need one to keep the visible viewport aligned.
        if width_changed || height_changed {
            self.full_render(true, new_lines, cursor, width, height);
            return Ok(());
        }

        // Content shrunk below the working area - re-render to clear empty rows.
        if self.clear_on_shrink && new_lines.len() < self.max_lines_rendered {
            self.full_render(true, new_lines, cursor, width, height);
            return Ok(());
        }

        // Find first and last changed lines.
        let mut first_changed: Option<usize> = None;
        let mut last_changed: Option<usize> = None;
        let max_lines = new_lines.len().max(self.previous_lines.len());
        for i in 0..max_lines {
            let old_line = self.previous_lines.get(i).map_or("", |l| l.as_str());
            let new_line = new_lines.get(i).map_or("", |l| l.as_str());
            if old_line != new_line {
                if first_changed.is_none() {
                    first_changed = Some(i);
                }
                last_changed = Some(i);
            }
        }
        let appended_lines = new_lines.len() > self.previous_lines.len();
        if appended_lines {
            if first_changed.is_none() {
                first_changed = Some(self.previous_lines.len());
            }
            last_changed = Some(new_lines.len() - 1);
        }

        // No changes - still update the hardware cursor position if it moved.
        let Some(first_changed) = first_changed else {
            self.position_hardware_cursor(cursor, new_lines.len());
            self.previous_viewport_top = prev_viewport_top;
            self.previous_height = height as i64;
            return Ok(());
        };
        let last_changed = last_changed.unwrap_or(first_changed);
        let append_start =
            appended_lines && first_changed == self.previous_lines.len() && first_changed > 0;

        let compute_line_diff =
            |target_row: usize, hardware_cursor_row: i64, prev_top: usize, top: usize| -> i64 {
                let current_screen_row = hardware_cursor_row - prev_top as i64;
                let target_screen_row = target_row as i64 - top as i64;
                target_screen_row - current_screen_row
            };

        // All changes are in deleted lines (nothing to render, just clear).
        if first_changed >= new_lines.len() {
            if self.previous_lines.len() > new_lines.len() {
                let mut buffer = String::from("\x1b[?2026h");
                let target_row = new_lines.len().saturating_sub(1);
                if target_row < prev_viewport_top {
                    self.full_render(true, new_lines, cursor, width, height);
                    return Ok(());
                }
                let line_diff = compute_line_diff(
                    target_row,
                    hardware_cursor_row,
                    prev_viewport_top,
                    viewport_top,
                );
                if line_diff > 0 {
                    buffer.push_str(&format!("\x1b[{line_diff}B"));
                } else if line_diff < 0 {
                    buffer.push_str(&format!("\x1b[{}A", -line_diff));
                }
                buffer.push('\r');
                let extra_lines = self.previous_lines.len() - new_lines.len();
                if extra_lines > height {
                    self.full_render(true, new_lines, cursor, width, height);
                    return Ok(());
                }
                let clear_start_offset = usize::from(!new_lines.is_empty());
                if extra_lines > 0 && clear_start_offset > 0 {
                    buffer.push_str(&format!("\x1b[{clear_start_offset}B"));
                }
                for i in 0..extra_lines {
                    buffer.push_str("\r\x1b[2K");
                    if i < extra_lines - 1 {
                        buffer.push_str("\x1b[1B");
                    }
                }
                let move_back = (extra_lines + clear_start_offset).saturating_sub(1);
                if move_back > 0 {
                    buffer.push_str(&format!("\x1b[{move_back}A"));
                }
                buffer.push_str("\x1b[?2026l");
                self.terminal.write(&buffer);
                self.cursor_row = target_row;
                self.hardware_cursor_row = target_row;
            }
            self.position_hardware_cursor(cursor, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_width = width as i64;
            self.previous_height = height as i64;
            self.previous_viewport_top = prev_viewport_top;
            return Ok(());
        }

        // Differential rendering can only touch what was actually visible.
        if first_changed < prev_viewport_top {
            self.full_render(true, new_lines, cursor, width, height);
            return Ok(());
        }

        let mut buffer = String::from("\x1b[?2026h");
        let prev_viewport_bottom = prev_viewport_top + height - 1;
        let move_target_row = if append_start {
            first_changed - 1
        } else {
            first_changed
        };
        if move_target_row > prev_viewport_bottom {
            let current_screen_row = (hardware_cursor_row - prev_viewport_top as i64)
                .clamp(0, height as i64 - 1) as usize;
            let move_to_bottom = height - 1 - current_screen_row;
            if move_to_bottom > 0 {
                buffer.push_str(&format!("\x1b[{move_to_bottom}B"));
            }
            let scroll = move_target_row - prev_viewport_bottom;
            buffer.push_str(&"\r\n".repeat(scroll));
            prev_viewport_top += scroll;
            viewport_top += scroll;
            hardware_cursor_row = move_target_row as i64;
        }

        let line_diff = compute_line_diff(
            move_target_row,
            hardware_cursor_row,
            prev_viewport_top,
            viewport_top,
        );
        if line_diff > 0 {
            buffer.push_str(&format!("\x1b[{line_diff}B"));
        } else if line_diff < 0 {
            buffer.push_str(&format!("\x1b[{}A", -line_diff));
        }
        buffer.push_str(if append_start { "\r\n" } else { "\r" });

        // Only render changed lines, not all lines to the end.
        let render_end = last_changed.min(new_lines.len() - 1);
        for (i, line) in new_lines
            .iter()
            .enumerate()
            .take(render_end + 1)
            .skip(first_changed)
        {
            if i > first_changed {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\x1b[2K");
            buffer.push_str(line);
        }

        let mut final_cursor_row = render_end;
        if self.previous_lines.len() > new_lines.len() {
            if render_end < new_lines.len() - 1 {
                let move_down = new_lines.len() - 1 - render_end;
                buffer.push_str(&format!("\x1b[{move_down}B"));
                final_cursor_row = new_lines.len() - 1;
            }
            let extra_lines = self.previous_lines.len() - new_lines.len();
            for _ in new_lines.len()..self.previous_lines.len() {
                buffer.push_str("\r\n\x1b[2K");
            }
            buffer.push_str(&format!("\x1b[{extra_lines}A"));
        }
        buffer.push_str("\x1b[?2026l");
        self.terminal.write(&buffer);

        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = final_cursor_row;
        self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        self.previous_viewport_top =
            prev_viewport_top.max((final_cursor_row + 1).saturating_sub(height));
        self.position_hardware_cursor(cursor, new_lines.len());
        self.previous_lines = new_lines;
        self.previous_width = width as i64;
        self.previous_height = height as i64;
        Ok(())
    }

    fn position_hardware_cursor(&mut self, cursor: Option<CursorPosition>, total_lines: usize) {
        let Some(cursor) = cursor else {
            self.terminal.hide_cursor();
            return;
        };
        if total_lines == 0 {
            self.terminal.hide_cursor();
            return;
        }
        let target_row = cursor.row.min(total_lines - 1);
        let target_col = cursor.column;
        let row_delta = target_row as i64 - self.hardware_cursor_row as i64;
        let mut buffer = String::new();
        if row_delta > 0 {
            buffer.push_str(&format!("\x1b[{row_delta}B"));
        } else if row_delta < 0 {
            buffer.push_str(&format!("\x1b[{}A", -row_delta));
        }
        buffer.push_str(&format!("\x1b[{}G", target_col + 1));
        self.terminal.write(&buffer);
        self.hardware_cursor_row = target_row;
        if self.show_hardware_cursor {
            self.terminal.show_cursor();
        } else {
            self.terminal.hide_cursor();
        }
    }

    /// Stop the lifecycle and leave the process cursor below rendered content.
    pub fn stop(&mut self) {
        if self.render_state == RenderState::Stopped {
            return;
        }
        if !self.previous_lines.is_empty() {
            let target_row = self.previous_lines.len() as i64;
            let line_diff = target_row - self.hardware_cursor_row as i64;
            if line_diff != 0 {
                self.terminal.move_by(line_diff as i32);
            }
            self.terminal.write("\r\n");
        }
        self.terminal.show_cursor();
        self.terminal.stop();
        self.render_state = RenderState::Stopped;
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        self.terminal.take_output()
    }

    pub fn full_redraws(&self) -> usize {
        self.full_redraws
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{
        CURSOR_MARKER, CursorPosition, RenderState, SEGMENT_RESET, Tui, TuiError,
        extract_cursor_position,
    };
    use crate::terminal::{START_SEQUENCE, TerminalState};

    fn output(tui: &mut Tui) -> String {
        String::from_utf8_lossy(&tui.take_output()).into_owned()
    }

    #[test]
    fn start_input_and_resize_coalesce_one_render() {
        let mut tui = Tui::new(TerminalState::new(Some(8), Some(3)), false);
        tui.start();
        assert_eq!(tui.render_state(), RenderState::Requested);
        tui.request_render(false);
        assert_eq!(tui.feed_input(b"a"), ["a"]);
        assert_eq!(tui.render_state(), RenderState::Requested);
        assert!(
            tui.render_if_requested(vec!["idle".into()])
                .is_ok_and(|v| v)
        );
        assert_eq!(tui.render_state(), RenderState::Idle);
        assert!(
            tui.render_if_requested(vec!["ignored".into()])
                .is_ok_and(|v| !v)
        );
        assert_eq!(tui.full_redraws(), 1);
    }

    #[test]
    fn force_render_resets_snapshot_and_clears() {
        let mut tui = Tui::new(TerminalState::new(Some(8), Some(3)), false);
        tui.start();
        let _ = tui.render_if_requested(vec!["idle".into()]);
        let _ = tui.take_output();
        tui.request_render(true);
        assert_eq!(tui.render_state(), RenderState::ForceRequested);
        assert!(
            tui.render_if_requested(vec!["idle".into()])
                .is_ok_and(|v| v)
        );
        assert!(output(&mut tui).contains("\x1b[2J\x1b[H\x1b[3J"));
    }

    #[test]
    fn cursor_marker_uses_visible_column_and_bottom_viewport() {
        let mut lines = vec![
            format!("old{CURSOR_MARKER}"),
            "plain".into(),
            format!("\x1b[31m界{CURSOR_MARKER}x\x1b[0m"),
        ];
        assert_eq!(
            extract_cursor_position(&mut lines, 2),
            Some(CursorPosition { row: 2, column: 2 })
        );
        assert!(!lines[2].contains(CURSOR_MARKER));
        assert!(lines[0].contains(CURSOR_MARKER));
    }

    #[test]
    fn lines_receive_segment_resets_like_pi() {
        let mut tui = Tui::new(TerminalState::new(Some(12), Some(4)), false);
        tui.start();
        assert!(tui.render_if_requested(vec!["ab".into()]).is_ok_and(|v| v));
        let rendered = output(&mut tui);
        assert!(rendered.contains(&format!("ab{SEGMENT_RESET}")));
    }

    #[test]
    fn render_positions_cursor_and_stop_emits_terminal_cleanup() {
        let mut tui = Tui::new(TerminalState::new(Some(12), Some(4)), true);
        tui.start();
        assert!(output(&mut tui).starts_with(START_SEQUENCE));
        let frame = vec!["heading".into(), format!("ab{CURSOR_MARKER}cd")];
        assert!(tui.render_if_requested(frame).is_ok_and(|v| v));
        let rendered = output(&mut tui);
        assert!(!rendered.contains(CURSOR_MARKER));
        // Cursor left on the marker row (row 1 = end of content), column 3.
        assert!(rendered.ends_with("\x1b[3G\x1b[?25h"));
        tui.stop();
        assert_eq!(tui.render_state(), RenderState::Stopped);
        assert_eq!(output(&mut tui), "\x1b[1B\r\n\x1b[?25h\x1b[?2004l\x1b[<u");
    }

    #[test]
    fn differential_frame_clears_and_rewrites_only_changed_rows() {
        let mut tui = Tui::new(TerminalState::new(Some(20), Some(8)), true);
        tui.start();
        let first = vec![
            "header".into(),
            format!("a{CURSOR_MARKER}"),
            "footer one".into(),
            "footer two".into(),
        ];
        assert!(tui.render_if_requested(first).is_ok_and(|v| v));
        let _ = output(&mut tui);

        assert_eq!(tui.feed_input(b"b"), ["b"]);
        let second = vec![
            "header".into(),
            format!("ab{CURSOR_MARKER}"),
            "footer one".into(),
            "footer two".into(),
        ];
        assert!(tui.render_if_requested(second).is_ok_and(|v| v));
        let rendered = output(&mut tui);
        // Hardware cursor was on row 1 (marker row); the only changed row is 1.
        assert!(rendered.starts_with("\x1b[?2026h\r\x1b[2Kab"));
        assert!(!rendered.contains("\r\nheader"));
    }

    #[test]
    fn shrinking_content_clears_removed_rows() {
        let mut tui = Tui::new(TerminalState::new(Some(20), Some(8)), false);
        tui.start();
        let first: Vec<String> = vec!["one".into(), "two".into(), "three".into()];
        assert!(tui.render_if_requested(first).is_ok_and(|v| v));
        let _ = output(&mut tui);
        tui.request_render(false);
        let second: Vec<String> = vec!["one".into()];
        assert!(tui.render_if_requested(second).is_ok_and(|v| v));
        let rendered = output(&mut tui);
        // Changes are only in deleted rows: move to end of new content and clear.
        assert!(rendered.contains("\r\x1b[2K"));
    }

    #[test]
    fn oversized_line_is_typed_and_reported() {
        let mut tui = Tui::new(TerminalState::new(Some(3), Some(2)), false);
        tui.start();
        let error = tui.render_if_requested(vec!["four".into()]);
        assert_eq!(
            error,
            Err(TuiError::LineTooWide {
                line: 0,
                actual: 4,
                width: 3
            })
        );
    }
}
