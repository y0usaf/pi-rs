//! Differential renderer on the terminal's main screen and scrollback (upstream
//! `tui-main-screen.ts`).
//!
//! Rendering is driven by the caller: [`TuiMainScreen::tick`] draws a pending frame when the
//! throttle allows, [`TuiMainScreen::render_now`] draws at once. Terminal output goes through
//! the [`Terminal`] in chunks, as upstream's bounded writer does.

use crate::terminal::{Terminal, TerminalEvent};
use crate::tui::{
    ComponentRef, TuiCore, apply_line_resets, extract_cursor_position, is_image_line,
    resolve_fake_cursors,
};
use crate::utils::visible_width;
use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};

const MAX_RENDER_WRITE_BYTES: usize = 1024 * 1024;
const KITTY_SEQUENCE_PREFIX: &str = "\x1b_G";
/// Minimum time between throttled frames.
pub const MIN_RENDER_INTERVAL_MS: u64 = 16;

struct BoundedWriter<'a> {
    buffer: String,
    term: &'a mut dyn Terminal,
}

impl<'a> BoundedWriter<'a> {
    fn new(term: &'a mut dyn Terminal) -> Self {
        BoundedWriter {
            buffer: String::new(),
            term,
        }
    }
    fn append(&mut self, mut value: &str) {
        while !value.is_empty() {
            let cap = MAX_RENDER_WRITE_BYTES - self.buffer.len();
            if cap == 0 {
                self.flush();
                continue;
            }
            let mut end = value.len().min(cap);
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                self.flush();
                continue;
            }
            self.buffer.push_str(&value[..end]);
            value = &value[end..];
            if self.buffer.len() == MAX_RENDER_WRITE_BYTES {
                self.flush();
            }
        }
    }
    fn flush(&mut self) {
        if !self.buffer.is_empty() {
            self.term.write(&self.buffer);
            self.buffer.clear();
        }
    }
}

fn kitty_image_header(line: &str) -> Option<(Vec<u64>, u64)> {
    let start = line.find(KITTY_SEQUENCE_PREFIX)? + KITTY_SEQUENCE_PREFIX.len();
    let end = start + line[start..].find(';')?;
    let mut ids = Vec::new();
    let mut rows = 1;
    for param in line[start..end].split(',') {
        let Some((k, v)) = param.split_once('=') else {
            continue;
        };
        let v = v.split('=').next().unwrap_or("");
        let Some(n) = js_number_integer(v) else {
            continue;
        };
        if n == 0 || n > 0xffff_ffff {
            continue;
        }
        if k == "i" {
            ids.push(n);
        } else if k == "r" {
            rows = n;
        }
    }
    Some((ids, rows))
}

/// `Number(v)` when it is a non-negative integer.
fn js_number_integer(v: &str) -> Option<u64> {
    let t = v.trim();
    if t.is_empty() {
        return Some(0);
    }
    let f: f64 = t.parse().ok()?;
    (f.fract() == 0.0 && f >= 0.0 && f.is_finite()).then_some(f as u64)
}

fn kitty_ids(line: &str) -> Vec<u64> {
    kitty_image_header(line).map(|h| h.0).unwrap_or_default()
}

fn kitty_rows(line: &str) -> u64 {
    kitty_image_header(line).map_or(1, |h| h.1)
}

/// Kitty command deleting one image.
pub fn delete_kitty_image(id: u64) -> String {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")
}

fn is_termux() -> bool {
    std::env::var_os("TERMUX_VERSION").is_some_and(|v| !v.is_empty())
}

/// The renderer state saved across a handover between TUIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainScreenRenderState {
    pub previous_lines: Vec<String>,
    pub previous_width: i64,
    pub previous_height: i64,
    pub cursor_row: i64,
    pub hardware_cursor_row: i64,
    pub max_lines_rendered: i64,
    pub previous_viewport_top: i64,
}

/// A TUI rendering into the main screen.
pub struct TuiMainScreen<T: Terminal> {
    pub core: TuiCore,
    pub terminal: T,
    previous_lines: Vec<String>,
    previous_kitty_ids: BTreeSet<u64>,
    previous_width: i64,
    previous_height: i64,
    cursor_row: i64,
    hardware_cursor_row: i64,
    max_lines_rendered: i64,
    previous_viewport_top: i64,
    full_redraw_count: u64,
    last_render_at: Option<u64>,
}

impl<T: Terminal> Deref for TuiMainScreen<T> {
    type Target = TuiCore;
    fn deref(&self) -> &TuiCore {
        &self.core
    }
}

impl<T: Terminal> DerefMut for TuiMainScreen<T> {
    fn deref_mut(&mut self) -> &mut TuiCore {
        &mut self.core
    }
}

impl<T: Terminal> TuiMainScreen<T> {
    pub fn new(terminal: T) -> Self {
        TuiMainScreen {
            core: TuiCore::default(),
            terminal,
            previous_lines: Vec::new(),
            previous_kitty_ids: BTreeSet::new(),
            previous_width: 0,
            previous_height: 0,
            cursor_row: 0,
            hardware_cursor_row: 0,
            max_lines_rendered: 0,
            previous_viewport_top: 0,
            full_redraw_count: 0,
            last_render_at: None,
        }
    }

    pub fn into_parts(self) -> (TuiCore, T) {
        (self.core, self.terminal)
    }

    pub fn from_parts(mut core: TuiCore, terminal: T) -> Self {
        let mut screen = Self::new(terminal);
        core.stopped = false;
        screen.core = core;
        screen
    }

    pub fn with_hardware_cursor(terminal: T, show: bool) -> Self {
        let mut s = Self::new(terminal);
        s.core.show_hardware_cursor = show;
        s
    }

    /// How many full redraws happened.
    pub fn full_redraws(&self) -> u64 {
        self.full_redraw_count
    }

    pub fn capture_render_state(&self) -> MainScreenRenderState {
        MainScreenRenderState {
            previous_lines: self.previous_lines.clone(),
            previous_width: self.previous_width,
            previous_height: self.previous_height,
            cursor_row: self.cursor_row,
            hardware_cursor_row: self.hardware_cursor_row,
            max_lines_rendered: self.max_lines_rendered,
            previous_viewport_top: self.previous_viewport_top,
        }
    }

    pub fn restore_render_state(&mut self, s: MainScreenRenderState) {
        self.previous_lines = s
            .previous_lines
            .into_iter()
            .map(|l| if is_image_line(&l) { String::new() } else { l })
            .collect();
        self.previous_kitty_ids.clear();
        self.previous_width = s.previous_width;
        self.previous_height = s.previous_height;
        self.cursor_row = s.cursor_row;
        self.hardware_cursor_row = s.hardware_cursor_row;
        self.max_lines_rendered = s.max_lines_rendered;
        self.previous_viewport_top = s.previous_viewport_top;
    }

    fn reset_render_state(&mut self) {
        self.previous_lines.clear();
        self.previous_width = -1;
        self.previous_height = -1;
        self.cursor_row = 0;
        self.hardware_cursor_row = 0;
        self.max_lines_rendered = 0;
        self.previous_viewport_top = 0;
    }

    fn flush_cursor_hide(&mut self) {
        if std::mem::take(&mut self.core.pending_hide_cursor) && !self.core.stopped {
            self.terminal.hide_cursor();
        }
    }

    /// Start the terminal and request the first frame.
    pub fn start(&mut self) {
        self.core.stopped = false;
        self.terminal.start();
        self.terminal.hide_cursor();
        self.terminal.set_exit_restore("\x1b[?25h", "");
        self.core.request_render(false);
    }

    /// Stop: move below the content (unless `preserve_screen`), show the cursor, restore the terminal.
    pub fn stop(&mut self, preserve_screen: bool) {
        self.core.stopped = true;
        if !preserve_screen && !self.previous_lines.is_empty() {
            self.terminal.write(" ");
            let diff = self.previous_lines.len() as i64 - self.hardware_cursor_row;
            if diff > 0 {
                self.terminal.write(&format!("\x1b[{diff}B"));
            } else if diff < 0 {
                self.terminal.write(&format!("\x1b[{}A", -diff));
            }
            self.terminal.write("\r\n");
        }
        self.terminal.show_cursor();
        self.terminal.stop();
    }

    /// Feed terminal events: input goes to the focused component, resizes request a frame.
    pub fn handle_events(&mut self, events: Vec<TerminalEvent>) {
        for e in events {
            match e {
                TerminalEvent::Input(d) => {
                    self.core.term_size = (self.terminal.columns(), self.terminal.rows());
                    self.core.handle_terminal_input(&d);
                }
                TerminalEvent::Resize => self.core.request_render(false),
            }
        }
        self.flush_cursor_hide();
    }

    /// Draw a frame now. `force` clears and redraws everything.
    pub fn render_now(&mut self, force: bool, now: u64) {
        if force {
            self.reset_render_state();
        }
        self.core.render_requested = false;
        self.core.force_requested = false;
        self.core.immediate = false;
        self.last_render_at = Some(now);
        self.do_render();
    }

    /// Draw the pending frame if one is due at `now` (immediately after input, else at most every
    /// 16 ms). Returns when the next frame is due, if one is pending.
    pub fn tick(&mut self, now: u64) -> Option<u64> {
        self.flush_cursor_hide();
        if self.core.stopped || !self.core.render_requested {
            return None;
        }
        let due = if self.core.immediate {
            now
        } else {
            self.last_render_at
                .map_or(now, |t| t + MIN_RENDER_INTERVAL_MS)
        };
        if now < due {
            return Some(due);
        }
        let force = self.core.force_requested;
        self.render_now(force, now);
        None
    }

    fn reserved_rows(lines: &[String], index: usize, max_index: usize) -> usize {
        let rows = kitty_rows(lines.get(index).map_or("", |s| s)) as usize;
        if rows <= 1 {
            return 1;
        }
        let max_rows = rows.min(max_index + 1 - index).min(lines.len() - index);
        let mut r = 1;
        while r < max_rows {
            let l = lines.get(index + r).map_or("", |s| s);
            if is_image_line(l) || visible_width(l) > 0 {
                break;
            }
            r += 1;
        }
        r
    }

    fn expand_for_images(&self, first: i64, last: i64, new_lines: &[String]) -> (i64, i64) {
        let (mut f, mut l) = (first, last);
        for lines in [&self.previous_lines[..], new_lines] {
            for i in 0..lines.len() {
                if kitty_ids(&lines[i]).is_empty() {
                    continue;
                }
                let end = (i + Self::reserved_rows(lines, i, lines.len() - 1) - 1) as i64;
                let i = i as i64;
                if i >= first || (i <= last && end >= first) {
                    f = f.min(i);
                    l = l.max(end);
                }
            }
        }
        (f, l)
    }

    fn delete_changed_images(&self, first: i64, last: i64) -> String {
        if first < 0 || last < first {
            return String::new();
        }
        let mut ids = BTreeSet::new();
        let max = last.min(self.previous_lines.len() as i64 - 1);
        let mut i = first;
        while i <= max {
            ids.extend(kitty_ids(&self.previous_lines[i as usize]));
            i += 1;
        }
        ids.into_iter().map(delete_kitty_image).collect()
    }

    fn collect_ids(lines: &[String]) -> BTreeSet<u64> {
        lines.iter().flat_map(|l| kitty_ids(l)).collect()
    }

    fn position_hardware_cursor(&mut self, pos: Option<(usize, usize)>, total: usize) {
        let Some((row, col)) = pos.filter(|_| total > 0) else {
            self.terminal.hide_cursor();
            return;
        };
        let target = row.min(total - 1) as i64;
        let delta = target - self.hardware_cursor_row;
        let mut buf = String::new();
        if delta > 0 {
            buf += &format!("\x1b[{delta}B");
        } else if delta < 0 {
            buf += &format!("\x1b[{}A", -delta);
        }
        buf += &format!("\x1b[{}G", col + 1);
        self.terminal.write(&buf);
        self.hardware_cursor_row = target;
        if self.core.show_hardware_cursor {
            self.terminal.show_cursor();
        } else {
            self.terminal.hide_cursor();
        }
    }

    fn full_render(
        &mut self,
        clear: bool,
        new_lines: Vec<String>,
        cursor: Option<(usize, usize)>,
        width: i64,
        height: i64,
    ) {
        self.full_redraw_count += 1;
        let ids = if clear {
            self.previous_kitty_ids
                .iter()
                .map(|&i| delete_kitty_image(i))
                .collect::<String>()
        } else {
            String::new()
        };
        {
            let mut out = BoundedWriter::new(&mut self.terminal);
            out.append("\x1b[?2026h");
            if clear {
                out.append(&ids);
                out.append("\x1b[2J\x1b[H\x1b[3J");
            }
            let mut i = 0;
            while i < new_lines.len() {
                if i > 0 {
                    out.append("\r\n");
                }
                let line = &new_lines[i];
                let reserved = if is_image_line(line) {
                    Self::reserved_rows(&new_lines, i, new_lines.len() - 1)
                } else {
                    1
                };
                if reserved > 1 && reserved as i64 <= height {
                    for _ in 1..reserved {
                        out.append("\r\n");
                    }
                    out.append(&format!("\x1b[{}A", reserved - 1));
                    out.append(line);
                    out.append(&format!("\x1b[{}B", reserved - 1));
                    i += reserved;
                    continue;
                }
                out.append(line);
                i += 1;
            }
            out.append("\x1b[?2026l");
            out.flush();
        }
        let n = new_lines.len() as i64;
        self.cursor_row = (n - 1).max(0);
        self.hardware_cursor_row = self.cursor_row;
        self.max_lines_rendered = if clear {
            n
        } else {
            self.max_lines_rendered.max(n)
        };
        self.previous_viewport_top = (height.max(n) - height).max(0);
        self.position_hardware_cursor(cursor, new_lines.len());
        self.previous_kitty_ids = Self::collect_ids(&new_lines);
        self.previous_lines = new_lines;
        self.previous_width = width;
        self.previous_height = height;
    }

    fn write_bounded(&mut self, s: &str) {
        let mut out = BoundedWriter::new(&mut self.terminal);
        out.append(s);
        out.flush();
    }

    fn do_render(&mut self) {
        if self.core.stopped {
            return;
        }
        let width = self.terminal.columns() as i64;
        let height = self.terminal.rows() as i64;
        self.core.term_size = (width as usize, height as usize);
        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let height_changed = self.previous_height != 0 && self.previous_height != height;
        let prev_buffer_len = if self.previous_height > 0 {
            self.previous_viewport_top + self.previous_height
        } else {
            height
        };
        let mut prev_top = if height_changed {
            (prev_buffer_len - height).max(0)
        } else {
            self.previous_viewport_top
        };
        let mut viewport_top = prev_top;
        let mut hw_row = self.hardware_cursor_row;

        let show_hw = self.core.show_hardware_cursor;
        let rendered = self
            .core
            .root
            .children
            .clone()
            .iter()
            .flat_map(|c| c.borrow_mut().render(width as usize))
            .collect();
        let mut new_lines = resolve_fake_cursors(rendered, show_hw);
        if !self.core.overlays.is_empty() {
            new_lines = self
                .core
                .composite_overlays(new_lines, width as usize, height as usize);
        }
        let cursor = extract_cursor_position(&mut new_lines, height as usize);
        apply_line_resets(&mut new_lines);
        self.flush_cursor_hide();

        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            return self.full_render(false, new_lines, cursor, width, height);
        }
        if width_changed {
            return self.full_render(true, new_lines, cursor, width, height);
        }
        if height_changed && !is_termux() {
            return self.full_render(true, new_lines, cursor, width, height);
        }
        let has_overlays = !self.core.overlays.is_empty();
        if self.core.clear_on_shrink
            && (new_lines.len() as i64) < self.max_lines_rendered
            && !has_overlays
        {
            return self.full_render(true, new_lines, cursor, width, height);
        }

        let prev_len = self.previous_lines.len() as i64;
        let new_len = new_lines.len() as i64;
        let (mut first, mut last) = (-1i64, -1i64);
        for i in 0..prev_len.max(new_len) as usize {
            let old = self.previous_lines.get(i).map_or("", |s| s);
            let new = new_lines.get(i).map_or("", |s| s);
            if old != new {
                if first == -1 {
                    first = i as i64;
                }
                last = i as i64;
            }
        }
        let appended = new_len > prev_len;
        if appended {
            if first == -1 {
                first = prev_len;
            }
            last = new_len - 1;
        }
        if first != -1 {
            (first, last) = self.expand_for_images(first, last, &new_lines);
        }
        let append_start = appended && first == prev_len && first > 0;

        if first == -1 {
            self.position_hardware_cursor(cursor, new_lines.len());
            self.previous_viewport_top = prev_top;
            self.previous_height = height;
            return;
        }

        let line_diff = |hw_row: i64, prev_top: i64, viewport_top: i64, target: i64| {
            (target - viewport_top) - (hw_row - prev_top)
        };

        if first >= new_len {
            if prev_len > new_len {
                let mut out = String::from("\x1b[?2026h");
                out += &self.delete_changed_images(first, last);
                let target = (new_len - 1).max(0);
                if target < prev_top {
                    return self.full_render(true, new_lines, cursor, width, height);
                }
                let d = line_diff(hw_row, prev_top, viewport_top, target);
                if d > 0 {
                    out += &format!("\x1b[{d}B");
                } else if d < 0 {
                    out += &format!("\x1b[{}A", -d);
                }
                out += "\r";
                let extra = prev_len - new_len;
                if extra > height {
                    return self.full_render(true, new_lines, cursor, width, height);
                }
                let clear_start = if new_len == 0 { 0 } else { 1 };
                if extra > 0 && clear_start > 0 {
                    out += &format!("\x1b[{clear_start}B");
                }
                for i in 0..extra {
                    out += "\r\x1b[2K";
                    if i < extra - 1 {
                        out += "\x1b[1B";
                    }
                }
                let back = (extra - 1 + clear_start).max(0);
                if back > 0 {
                    out += &format!("\x1b[{back}A");
                }
                out += "\x1b[?2026l";
                self.write_bounded(&out);
                self.cursor_row = target;
                self.hardware_cursor_row = target;
            }
            self.position_hardware_cursor(cursor, new_lines.len());
            self.previous_kitty_ids = Self::collect_ids(&new_lines);
            self.previous_lines = new_lines;
            self.previous_width = width;
            self.previous_height = height;
            self.previous_viewport_top = prev_top;
            return;
        }

        if first < prev_top {
            return self.full_render(true, new_lines, cursor, width, height);
        }

        let mut out = String::from("\x1b[?2026h");
        out += &self.delete_changed_images(first, last);
        let prev_bottom = prev_top + height - 1;
        let move_target = if append_start { first - 1 } else { first };
        if move_target > prev_bottom {
            let cur_screen = (hw_row - prev_top).min(height - 1).max(0);
            let to_bottom = height - 1 - cur_screen;
            if to_bottom > 0 {
                out += &format!("\x1b[{to_bottom}B");
            }
            let scroll = move_target - prev_bottom;
            out += &"\r\n".repeat(scroll as usize);
            prev_top += scroll;
            viewport_top += scroll;
            hw_row = move_target;
        }
        let d = line_diff(hw_row, prev_top, viewport_top, move_target);
        if d > 0 {
            out += &format!("\x1b[{d}B");
        } else if d < 0 {
            out += &format!("\x1b[{}A", -d);
        }
        out += if append_start { "\r\n" } else { "\r" };

        let render_end = last.min(new_len - 1);
        let mut i = first;
        while i <= render_end {
            if i > first {
                out += "\r\n";
            }
            let line = &new_lines[i as usize];
            let image = is_image_line(line);
            let reserved = if image {
                Self::reserved_rows(&new_lines, i as usize, render_end as usize) as i64
            } else {
                1
            };
            if reserved > 1 {
                let start_row = i - viewport_top;
                if start_row < 0 || start_row + reserved > height {
                    return self.full_render(true, new_lines, cursor, width, height);
                }
                out += "\x1b[2K";
                for _ in 1..reserved {
                    out += "\r\n\x1b[2K";
                }
                out += &format!("\x1b[{}A", reserved - 1);
                out += line;
                out += &format!("\x1b[{}B", reserved - 1);
                i += reserved;
                continue;
            }
            out += "\x1b[2K";
            if !image && visible_width(line) as i64 > width {
                self.crash(i as usize, &new_lines, width);
            }
            out += line;
            i += 1;
        }

        let mut final_row = render_end;
        if prev_len > new_len {
            if render_end < new_len - 1 {
                let down = new_len - 1 - render_end;
                out += &format!("\x1b[{down}B");
                final_row = new_len - 1;
            }
            let extra = prev_len - new_len;
            for _ in new_len..prev_len {
                out += "\r\n\x1b[2K";
            }
            out += &format!("\x1b[{extra}A");
        }
        out += "\x1b[?2026l";
        self.write_bounded(&out);

        self.cursor_row = (new_len - 1).max(0);
        self.hardware_cursor_row = final_row;
        self.max_lines_rendered = self.max_lines_rendered.max(new_len);
        self.previous_viewport_top = prev_top.max(final_row - height + 1);
        self.position_hardware_cursor(cursor, new_lines.len());
        self.previous_kitty_ids = Self::collect_ids(&new_lines);
        self.previous_lines = new_lines;
        self.previous_width = width;
        self.previous_height = height;
    }

    /// A line wider than the terminal is a component bug: log all lines, restore the terminal, panic.
    fn crash(&mut self, index: usize, lines: &[String], width: i64) -> ! {
        let path = std::env::temp_dir().join("pi-tui-crash.log");
        let mut data = format!(
            "Terminal width: {width}\nLine {index} visible width: {}\n\n=== All rendered lines ===\n",
            visible_width(&lines[index])
        );
        for (i, l) in lines.iter().enumerate() {
            data += &format!("[{i}] (w={}) {l}\n", visible_width(l));
        }
        let _ = std::fs::write(&path, data);
        self.stop(false);
        panic!(
            "Rendered line {index} exceeds terminal width ({} > {width}).\n\nThis is likely caused by a custom TUI component not truncating its output.\nUse visible_width() to measure and truncate_to_width() to truncate lines.\n\nDebug log written to: {}",
            visible_width(&lines[index]),
            path.display()
        );
    }
}

/// Convenience: a component handle as a [`ComponentRef`].
pub fn as_ref<C: crate::tui::Component + 'static>(
    c: &std::rc::Rc<std::cell::RefCell<C>>,
) -> ComponentRef {
    c.clone()
}
