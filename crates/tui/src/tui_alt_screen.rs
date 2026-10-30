use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::time::Instant;

use crate::components::scroll_view::{
    Overscroll, ScrollOptions, ScrollRef, ScrollState, ScrollView,
};
use crate::keybindings::get_keybindings;
use crate::keys::is_key_release;
use crate::layout::{
    LayoutFrame, render_layout_frame, scroll_view_box, scroll_views_at, scrollbar_geometry,
    strip_prompt_zones,
};
use crate::terminal::{Terminal, TerminalEvent};
use crate::tui::{
    CURSOR_MARKER, ComponentRef, Container, TuiCore, apply_line_resets, composite_tui_line,
    extract_cursor_position, resolve_fake_cursors,
};
use crate::tui_main_screen::MIN_RENDER_INTERVAL_MS;
use crate::utils::{slice_by_column, truncate_to_width, visible_width};
use crate::wheel_scroll::{WheelScroll, WheelScrollLines};

const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
const EXIT_ALT_SCREEN: &str = "\x1b[?1049l";
const DISABLE_AUTOWRAP: &str = "\x1b[?7l";
const ENABLE_AUTOWRAP: &str = "\x1b[?7h";
const ENABLE_BUTTON_MOTION_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1004h\x1b[?1006h";
const ENABLE_ALL_MOTION_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h";
const DISABLE_MOUSE: &str = "\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
const FOCUS_IN: &str = "\x1b[I";
const FOCUS_OUT: &str = "\x1b[O";
const BEGIN_SYNCHRONIZED_OUTPUT: &str = "\x1b[?2026h";
const END_SYNCHRONIZED_OUTPUT: &str = "\x1b[?2026l";
const PROMPT_START: &str = "\x1b]133;A";
const PAGE_SCROLL_OVERLAP: usize = 4;
const ALT_WHEEL_SCROLL_MULTIPLIER: i64 = 5;

pub type LabelFn = Rc<dyn Fn() -> String>;

pub struct AltScreenOptions {
    pub mouse: bool,
    pub wheel_scroll_lines: WheelScrollLines,
    pub scroll_to_end_indicator: Option<LabelFn>,
}

impl Default for AltScreenOptions {
    fn default() -> Self {
        AltScreenOptions {
            mouse: true,
            wheel_scroll_lines: WheelScrollLines::Lines(1.0),
            scroll_to_end_indicator: None,
        }
    }
}

struct SgrMouse {
    button: u32,
    x: i64,
    y: i64,
    release: bool,
}

fn parse_sgr_mouse(data: &str) -> Option<SgrMouse> {
    let body = data.strip_prefix("\x1b[<")?;
    let release = body.ends_with('m');
    let body = body.strip_suffix(['M', 'm'])?;
    let mut parts = body.split(';');
    let mut number = || -> Option<i64> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let button = number()?;
    let x = number()? - 1;
    let y = number()? - 1;
    if parts.next().is_some() {
        return None;
    }
    Some(SgrMouse {
        button: button as u32,
        x,
        y,
        release,
    })
}

fn parse_legacy_mouse(data: &str) -> Option<SgrMouse> {
    let bytes = data.as_bytes();
    if bytes.len() != 6 || !data.starts_with("\x1b[M") {
        return None;
    }
    Some(SgrMouse {
        button: u32::from(bytes[3]).wrapping_sub(32),
        x: i64::from(bytes[4]) - 33,
        y: i64::from(bytes[5]) - 33,
        release: false,
    })
}

fn wheel_event(data: &str) -> Option<(i64, SgrMouse)> {
    let event = parse_sgr_mouse(data).or_else(|| parse_legacy_mouse(data))?;
    if event.button & 64 == 0 {
        return None;
    }
    match event.button & 3 {
        0 => Some((-1, event)),
        1 => Some((1, event)),
        _ => None,
    }
}

fn is_mouse_sequence(data: &str) -> bool {
    parse_sgr_mouse(data).is_some() || (data.len() == 6 && data.starts_with("\x1b[M"))
}

fn mouse_sequence() -> &'static str {
    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();
    let multiplexed = ["TMUX", "ZELLIJ", "STY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
        || term.starts_with("tmux")
        || term.starts_with("screen");
    if multiplexed {
        ENABLE_BUTTON_MOTION_MOUSE
    } else {
        ENABLE_ALL_MOTION_MOUSE
    }
}

pub struct TuiAltScreen<T: Terminal> {
    pub core: TuiCore,
    pub terminal: T,
    layout_root: Option<ComponentRef>,
    implicit: ScrollRef,
    current: Option<LayoutFrame>,
    previous_screen: Vec<String>,
    previous_width: usize,
    previous_height: usize,
    active: bool,
    mouse: bool,
    wheel: WheelScroll,
    epoch: Instant,
    scroll_to_end_indicator: Option<LabelFn>,
    indicator_rect: Option<(i64, i64, i64)>,
    full_redraw_count: u64,
    last_render_at: Option<u64>,
}

impl<T: Terminal> Deref for TuiAltScreen<T> {
    type Target = TuiCore;
    fn deref(&self) -> &TuiCore {
        &self.core
    }
}

impl<T: Terminal> DerefMut for TuiAltScreen<T> {
    fn deref_mut(&mut self) -> &mut TuiCore {
        &mut self.core
    }
}

impl<T: Terminal> TuiAltScreen<T> {
    pub fn new(terminal: T, options: AltScreenOptions) -> Self {
        Self::from_parts(TuiCore::default(), terminal, options)
    }

    pub fn from_parts(mut core: TuiCore, terminal: T, options: AltScreenOptions) -> Self {
        core.stopped = false;
        TuiAltScreen {
            core,
            terminal,
            layout_root: None,
            implicit: Rc::new(RefCell::new(ScrollState::new(ScrollOptions {
                follow_end: true,
                primary: true,
                ..ScrollOptions::default()
            }))),
            current: None,
            previous_screen: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            active: false,
            mouse: options.mouse,
            wheel: WheelScroll::new(options.wheel_scroll_lines),
            epoch: Instant::now(),
            scroll_to_end_indicator: options.scroll_to_end_indicator,
            indicator_rect: None,
            full_redraw_count: 0,
            last_render_at: None,
        }
    }

    pub fn into_parts(self) -> (TuiCore, T) {
        (self.core, self.terminal)
    }

    pub fn full_redraws(&self) -> u64 {
        self.full_redraw_count
    }

    pub fn set_wheel_scroll_lines(&mut self, lines: WheelScrollLines) {
        self.wheel.set_lines(lines);
    }

    pub fn screen_lines(&self) -> Vec<String> {
        self.previous_screen.clone()
    }

    pub fn set_layout_root(&mut self, root: Option<ComponentRef>) {
        let same = match (&self.layout_root, &root) {
            (Some(a), Some(b)) => crate::tui::same(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.layout_root = root;
        self.current = None;
        self.core.request_render(false);
    }

    fn root(&self) -> ComponentRef {
        match &self.layout_root {
            Some(root) => root.clone(),
            None => {
                let document: ComponentRef = Rc::new(RefCell::new(Container {
                    children: self.core.root.children.clone(),
                }));
                Rc::new(RefCell::new(ScrollView::with_state(
                    document,
                    self.implicit.clone(),
                )))
            }
        }
    }

    pub fn render(&mut self, width: usize) -> Vec<String> {
        match &self.layout_root {
            Some(root) => root.borrow_mut().render(width),
            None => self
                .core
                .root
                .children
                .clone()
                .iter()
                .flat_map(|c| c.borrow_mut().render(width))
                .collect(),
        }
    }

    pub fn primary_scroll(&self) -> ScrollRef {
        self.current
            .as_ref()
            .and_then(|frame| frame.primary.clone())
            .unwrap_or_else(|| self.implicit.clone())
    }

    pub fn viewport_top(&self) -> usize {
        self.primary_scroll().borrow().scroll_top()
    }

    pub fn is_following_output(&self) -> bool {
        self.primary_scroll().borrow().is_following_end()
    }

    fn exit_restore(&self) -> (String, String) {
        if self.active {
            let mouse = if self.mouse { DISABLE_MOUSE } else { "" };
            (
                format!("{END_SYNCHRONIZED_OUTPUT}{mouse}{ENABLE_AUTOWRAP}\x1b[?25h"),
                format!("{EXIT_ALT_SCREEN}\x1b[?25h"),
            )
        } else {
            ("\x1b[?25h".to_owned(), String::new())
        }
    }

    pub fn start(&mut self) {
        self.core.stopped = false;
        self.active = true;
        self.reset_render_state();
        let mouse = if self.mouse { mouse_sequence() } else { "" };
        self.terminal.write(&format!(
            "{ENTER_ALT_SCREEN}{DISABLE_AUTOWRAP}{mouse}\x1b[2J\x1b[H\x1b[?25l"
        ));
        self.terminal.start();
        self.terminal.hide_cursor();
        let (before, after) = self.exit_restore();
        self.terminal.set_exit_restore(&before, &after);
        self.core.request_render(false);
    }

    pub fn stop(&mut self, preserve_screen: bool) {
        self.core.stopped = true;
        if self.active {
            let mouse = if self.mouse { DISABLE_MOUSE } else { "" };
            self.terminal.write(&format!(
                "{BEGIN_SYNCHRONIZED_OUTPUT}{mouse}{ENABLE_AUTOWRAP}{END_SYNCHRONIZED_OUTPUT}"
            ));
        }
        self.terminal.show_cursor();
        self.terminal.stop();
        if !self.active {
            return;
        }
        self.active = false;
        if preserve_screen {
            self.terminal.write(&format!(
                "{BEGIN_SYNCHRONIZED_OUTPUT}{EXIT_ALT_SCREEN}\x1b[?25h{END_SYNCHRONIZED_OUTPUT}"
            ));
            return;
        }
        let width = self.terminal.columns().max(1);
        let rendered = self.render(width);
        let mut document: Vec<String> =
            resolve_fake_cursors(rendered, self.core.show_hardware_cursor)
                .into_iter()
                .map(|line| strip_prompt_zones(&line).replace(CURSOR_MARKER, ""))
                .collect();
        apply_line_resets(&mut document);
        let mut buffer = format!("{BEGIN_SYNCHRONIZED_OUTPUT}{EXIT_ALT_SCREEN}{DISABLE_AUTOWRAP}");
        for (row, line) in document.iter().enumerate() {
            if row > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\r\x1b[2K");
            if visible_width(line) <= width {
                buffer.push_str(line);
            } else {
                buffer.push_str(&slice_by_column(line, 0, width, true));
            }
        }
        buffer.push_str(&format!(
            "\x1b[0m{ENABLE_AUTOWRAP}\r\n\x1b[?25h{END_SYNCHRONIZED_OUTPUT}"
        ));
        self.terminal.write(&buffer);
    }

    fn reset_render_state(&mut self) {
        self.previous_screen.clear();
        self.previous_width = 0;
        self.previous_height = 0;
        self.current = None;
    }

    pub fn scroll_by(&mut self, lines: i64) {
        self.primary_scroll().borrow_mut().scroll_by(lines);
        self.core.request_render(false);
    }

    pub fn scroll_to_top(&mut self) {
        self.primary_scroll().borrow_mut().scroll_to_start();
        self.core.request_render(false);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.primary_scroll().borrow_mut().scroll_to_end();
        self.core.request_render(false);
    }

    fn scroll_to_prompt(&mut self, direction: i64) {
        let Some(frame) = &self.current else {
            return;
        };
        let scroll = self.primary_scroll();
        let Some(lines) =
            scroll_view_box(frame, &scroll).and_then(|b| b.scroll_content_lines.clone())
        else {
            return;
        };
        let mut row = scroll.borrow().scroll_top() as i64 + direction;
        while row >= 0 && (row as usize) < lines.len() {
            if lines[row as usize].starts_with(PROMPT_START) {
                scroll.borrow_mut().scroll_to(row, false);
                self.core.request_render(false);
                return;
            }
            row += direction;
        }
    }

    fn flush_cursor_hide(&mut self) {
        if std::mem::take(&mut self.core.pending_hide_cursor) && !self.core.stopped {
            self.terminal.hide_cursor();
        }
    }

    pub fn handle_events(&mut self, events: Vec<TerminalEvent>) {
        for event in events {
            match event {
                TerminalEvent::Input(data) => {
                    self.core.term_size = (self.terminal.columns(), self.terminal.rows());
                    if !self.handle_viewport_input(&data) {
                        self.core.handle_terminal_input(&data);
                    }
                }
                TerminalEvent::Resize => self.core.request_render(false),
            }
        }
        self.flush_cursor_hide();
    }

    fn handle_viewport_input(&mut self, data: &str) -> bool {
        if data == FOCUS_OUT || data == FOCUS_IN {
            return true;
        }
        if let Some((direction, event)) = wheel_event(data) {
            let now = self.epoch.elapsed().as_secs_f64() * 1000.0;
            let lines = self.wheel.next(direction, now);
            let delta = direction
                * if event.button & 8 != 0 {
                    lines * ALT_WHEEL_SCROLL_MULTIPLIER
                } else {
                    lines
                };
            self.route_wheel(event.x, event.y, delta);
            return true;
        }
        if let Some(event) = parse_sgr_mouse(data) {
            self.handle_mouse(&event);
            return true;
        }
        if is_mouse_sequence(data) {
            return true;
        }
        let kb = get_keybindings();
        let release = is_key_release(data);
        if self.core.is_overlay_focused_any() {
            return false;
        }
        let viewport = self.primary_scroll().borrow().viewport_height();
        let scroll: Option<i64> = if kb.matches(data, "tui.altScreen.pageUp") {
            Some(-(viewport.saturating_sub(PAGE_SCROLL_OVERLAP).max(1) as i64))
        } else if kb.matches(data, "tui.altScreen.pageDown") {
            Some(viewport.saturating_sub(PAGE_SCROLL_OVERLAP).max(1) as i64)
        } else if kb.matches(data, "tui.altScreen.halfPageUp") {
            Some(-((viewport / 2).max(1) as i64))
        } else if kb.matches(data, "tui.altScreen.halfPageDown") {
            Some((viewport / 2).max(1) as i64)
        } else if kb.matches(data, "tui.altScreen.lineUp") {
            Some(-1)
        } else if kb.matches(data, "tui.altScreen.lineDown") {
            Some(1)
        } else {
            None
        };
        if let Some(lines) = scroll {
            if !release {
                self.scroll_by(lines);
            }
            return true;
        }
        let action: Option<fn(&mut Self)> = if kb.matches(data, "tui.altScreen.previousPrompt") {
            Some(|tui| tui.scroll_to_prompt(-1))
        } else if kb.matches(data, "tui.altScreen.nextPrompt") {
            Some(|tui| tui.scroll_to_prompt(1))
        } else if kb.matches(data, "tui.altScreen.top") {
            Some(Self::scroll_to_top)
        } else if kb.matches(data, "tui.altScreen.bottom") {
            Some(Self::scroll_to_bottom)
        } else {
            None
        };
        match action {
            Some(action) => {
                if !release {
                    action(self);
                }
                true
            }
            None => false,
        }
    }

    fn handle_mouse(&mut self, event: &SgrMouse) {
        let Some((row, column, width)) = self.indicator_rect else {
            return;
        };
        if event.release || event.button & 32 != 0 || event.button & 3 != 0 {
            return;
        }
        if event.y == row && event.x >= column && event.x < column + width {
            self.scroll_to_bottom();
        }
    }

    fn route_wheel(&mut self, x: i64, y: i64, delta: i64) {
        let mut remaining = delta;
        let mut seen: Vec<ScrollRef> = Vec::new();
        if let Some(frame) = &self.current {
            for scroll in scroll_views_at(frame, x, y) {
                remaining = scroll.borrow_mut().scroll_by(remaining);
                let contain = scroll.borrow().overscroll == Overscroll::Contain;
                seen.push(scroll);
                if remaining == 0 || contain {
                    break;
                }
            }
        }
        let primary = self.primary_scroll();
        if remaining != 0 && !seen.iter().any(|scroll| Rc::ptr_eq(scroll, &primary)) {
            primary.borrow_mut().scroll_by(remaining);
        }
        self.core.request_render(false);
    }

    fn composite_scroll_to_end_indicator(
        &mut self,
        screen: Vec<String>,
        frame: &LayoutFrame,
        width: usize,
    ) -> Vec<String> {
        self.indicator_rect = None;
        let scroll = frame
            .primary
            .clone()
            .unwrap_or_else(|| self.implicit.clone());
        let Some(indicator) = &self.scroll_to_end_indicator else {
            return screen;
        };
        if !scroll.borrow().follow_end || scroll.borrow().is_following_end() {
            return screen;
        }
        let Some(layout) = scroll_view_box(frame, &scroll) else {
            return screen;
        };
        let clip = layout.clip;
        if clip.width <= 0 || clip.height <= 0 {
            return screen;
        }
        let row = clip.y + clip.height - 1;
        if row < 0 || row as usize >= screen.len() {
            return screen;
        }
        let scrollbar = scrollbar_geometry(layout, false).map(|g| g.column);
        let label = truncate_to_width(&indicator(), clip.width as usize, "", false);
        let label_width = visible_width(&label) as i64;
        let column = clip.x + (clip.width - label_width) / 2;
        let right = scrollbar.unwrap_or(clip.x + clip.width);
        let available = (right - column).max(0) as usize;
        let text = truncate_to_width(&label, available, "", false);
        let text_width = visible_width(&text);
        if text_width == 0 {
            return screen;
        }
        let mut result = screen;
        let index = row as usize;
        result[index] = composite_tui_line(
            &result[index],
            &text,
            column.max(0) as usize,
            text_width,
            width,
        );
        self.indicator_rect = Some((row, column, text_width as i64));
        result
    }

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

    fn poll_scroll_timers(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let scrolls: Vec<ScrollRef> = self
            .current
            .as_ref()
            .map(|frame| frame.scrolls.clone())
            .unwrap_or_default();
        let mut next: Option<Instant> = None;
        for scroll in scrolls.iter().chain(std::iter::once(&self.implicit)) {
            let mut state = scroll.borrow_mut();
            state.fire_timer(now);
            if state.take_render_request() {
                self.core.render_requested = true;
            }
            if let Some(at) = state.hide_deadline() {
                next = Some(next.map_or(at, |n| n.min(at)));
            }
        }
        next
    }

    pub fn tick(&mut self, now: u64) -> Option<u64> {
        self.flush_cursor_hide();
        let timer = self.poll_scroll_timers();
        if self.core.stopped {
            return None;
        }
        let timer_due =
            timer.map(|at| now + at.saturating_duration_since(Instant::now()).as_millis() as u64);
        if !self.core.render_requested {
            return timer_due;
        }
        let due = if self.core.immediate {
            now
        } else {
            self.last_render_at
                .map_or(now, |t| t + MIN_RENDER_INTERVAL_MS)
        };
        if now < due {
            return Some(timer_due.map_or(due, |t| t.min(due)));
        }
        let force = self.core.force_requested;
        self.render_now(force, now);
        timer_due
    }

    fn do_render(&mut self) {
        if self.core.stopped || !self.active {
            return;
        }
        let width = self.terminal.columns().max(1);
        let height = self.terminal.rows().max(1);
        self.core.term_size = (width, height);
        let root = self.root();
        let frame = render_layout_frame(&root, width, height);
        let lines: Vec<String> = frame
            .lines
            .iter()
            .map(|line| strip_prompt_zones(line).to_owned())
            .collect();
        let mut screen = resolve_fake_cursors(lines, self.core.show_hardware_cursor);
        screen = self.composite_scroll_to_end_indicator(screen, &frame, width);
        if !self.core.overlays.is_empty() {
            screen = self.core.composite_overlays(screen, width, height);
        }
        if screen.len() > height {
            screen = screen.split_off(screen.len() - height);
        }
        let cursor = extract_cursor_position(&mut screen, height);
        apply_line_resets(&mut screen);
        for line in screen.iter_mut() {
            if visible_width(line) > width {
                *line = slice_by_column(line, 0, width, true);
            }
        }
        let full = self.previous_screen.is_empty()
            || self.previous_width != width
            || self.previous_height != height;
        let mut buffer = String::from(BEGIN_SYNCHRONIZED_OUTPUT);
        if full {
            self.full_redraw_count += 1;
            buffer.push_str("\x1b[2J");
        }
        for row in 0..height {
            let line = screen.get(row).map_or("", String::as_str);
            if !full && self.previous_screen.get(row).map(String::as_str) == Some(line) {
                continue;
            }
            buffer.push_str(&format!("\x1b[{};1H\x1b[2K{line}", row + 1));
        }
        match cursor {
            Some((row, col)) => {
                buffer.push_str(&format!("\x1b[{};{}H", row + 1, col.min(width) + 1));
                buffer.push_str(if self.core.show_hardware_cursor {
                    "\x1b[?25h"
                } else {
                    "\x1b[?25l"
                });
            }
            None => buffer.push_str("\x1b[?25l"),
        }
        buffer.push_str(END_SYNCHRONIZED_OUTPUT);
        self.terminal.write(&buffer);
        self.previous_screen = screen;
        self.previous_width = width;
        self.previous_height = height;
        self.current = Some(frame);
    }
}
