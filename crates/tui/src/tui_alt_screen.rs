use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::alt_screen_search::{
    AltScreenSearchComponent, AltScreenSearchIndex, NavigationButtonStyle, SearchMatch,
};
use crate::components::alt_screen_flash::{AltScreenFlashContainer, DEFAULT_FLASH_DURATION};
use crate::components::scroll_view::{
    Overscroll, ScrollOptions, ScrollRef, ScrollState, ScrollView,
};
use crate::components::text::StyleFn;
use crate::keybindings::get_keybindings;
use crate::keys::is_key_release;
use crate::layout::{
    LayoutBox, LayoutFrame, ScrollbarGeometry, layout_boxes_at, render_layout_frame,
    scroll_view_box, scroll_views_at, scrollbar_geometry, strip_prompt_zones,
};
use crate::osc52;
use crate::terminal::{Terminal, TerminalEvent};
use crate::tui::{
    CURSOR_MARKER, ComponentRef, Container, MouseButton, MouseEventType, OverlayAnchor, OverlayId,
    OverlayMargin, OverlayMouse, OverlayOptions, SizeValue, TuiCore, TuiMouseDispatchResult,
    TuiMouseDispatchTarget, TuiMouseEvent, apply_line_resets, composite_tui_line,
    dispatch_mouse_event, extract_cursor_position, is_image_line, resolve_fake_cursors,
    retarget_mouse_event, same,
};
use crate::tui_main_screen::MIN_RENDER_INTERVAL_MS;
use crate::unicode::word_segments;
use crate::utils::{
    extract_ansi_code, grapheme_cell_range, js_trim, js_trim_end, osc8_link_at_column,
    slice_by_column, strip_terminal_sequences, truncate_to_width, visible_width,
};
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
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const COPY_ERROR_FLASH_DURATION: Duration = Duration::from_millis(5000);
const SELECTION_AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(50);
const TERMINAL_WORD_SELECTION_JOINERS: [&str; 2] = ["/", "-"];

pub type LabelFn = Rc<dyn Fn() -> String>;
pub type OpenUrl = Rc<dyn Fn(&str)>;
pub type CopySelection = Rc<dyn Fn(String) -> Receiver<Result<(), String>>>;

pub struct AltScreenOptions {
    pub mouse: bool,
    pub wheel_scroll_lines: WheelScrollLines,
    pub search_match_style: StyleFn,
    pub search_current_match_style: StyleFn,
    pub search_navigation_button_style: NavigationButtonStyle,
    pub scroll_to_end_indicator: Option<LabelFn>,
    pub open_url: Option<OpenUrl>,
    pub copy_on_select: bool,
    pub copy_selection: Option<CopySelection>,
}

impl Default for AltScreenOptions {
    fn default() -> Self {
        AltScreenOptions {
            mouse: true,
            wheel_scroll_lines: WheelScrollLines::Lines(1.0),
            search_match_style: Rc::new(|text: &str| format!("\x1b[4m{text}\x1b[24m")),
            search_current_match_style: Rc::new(|text: &str| format!("\x1b[1;7m{text}\x1b[22;27m")),
            search_navigation_button_style: Rc::new(|text: &str, _| text.to_owned()),
            scroll_to_end_indicator: None,
            open_url: None,
            copy_on_select: true,
            copy_selection: None,
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

fn decode_mouse_button(button: u32) -> MouseButton {
    match button & 3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    }
}

fn same_scroll(a: &Option<ScrollRef>, b: &Option<ScrollRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[derive(Clone)]
struct SelectionPoint {
    row: i64,
    col: i64,
    scroll: Option<ScrollRef>,
    boundary: bool,
}

#[derive(Clone)]
struct SelectionRange {
    start: SelectionPoint,
    end: SelectionPoint,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Character,
    Word,
    Line,
}

struct ClickTarget {
    at: Instant,
    count: u32,
    row: i64,
    scroll: Option<ScrollRef>,
    word_start: i64,
    word_end: i64,
}

struct ComponentClick {
    at: Instant,
    count: u32,
    component: ComponentRef,
    x: i64,
    y: i64,
}

struct ScrollbarDrag {
    scroll: ScrollRef,
    grab_offset: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchSelection {
    Query,
    Retain,
    Next,
    Previous,
}

struct ActiveSearch {
    component: Rc<RefCell<AltScreenSearchComponent>>,
    index: AltScreenSearchIndex,
    overlay: OverlayId,
    query: String,
    matches: Rc<Vec<SearchMatch>>,
    selected_index: i64,
    selected_key: Option<String>,
    anchor_row: i64,
    mode: SearchSelection,
}

struct SearchHighlight {
    start: i64,
    end: i64,
    current: bool,
}

struct WordSegmentSpan {
    start: i64,
    end: i64,
    selectable: bool,
    joiner: bool,
}

fn selection_columns(
    line: &str,
    row: i64,
    (start, end): (&SelectionPoint, &SelectionPoint),
    min_column: i64,
    max_column: i64,
) -> (i64, i64) {
    let line_width = visible_width(line) as i64;
    let mut first = min_column.max(0);
    let mut last = line_width.min(max_column);
    if row == start.row {
        first = grapheme_cell_range(line, start.col.max(0) as usize)
            .map_or(start.col.min(line_width), |(cell, _)| cell as i64);
    }
    if row == end.row {
        last = if end.boundary {
            end.col.min(line_width)
        } else {
            grapheme_cell_range(line, end.col.max(0) as usize)
                .map_or((end.col + 1).min(line_width), |(_, cell)| cell as i64)
        };
    }
    (first.max(min_column), last.min(max_column))
}

fn selection_highlight(text: &str) -> String {
    let mut result = String::from("\x1b[7m");
    let mut index = 0;
    while index < text.len() {
        if let Some(code) = extract_ansi_code(text, index) {
            result.push_str(code);
            if code.ends_with('m') {
                result.push_str("\x1b[7m");
            }
            index += code.len();
            continue;
        }
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        result.push(ch);
        index += ch.len_utf8();
    }
    result.push_str("\x1b[27m");
    result
}

fn search_text_highlight(text: &str, style: &StyleFn) -> String {
    let mut result = String::new();
    let mut plain_start = 0;
    let mut index = 0;
    while index < text.len() {
        let Some(code) = extract_ansi_code(text, index) else {
            index += text[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        if index > plain_start {
            result.push_str(&style(&text[plain_start..index]));
        }
        result.push_str(code);
        index += code.len();
        plain_start = index;
    }
    if plain_start < text.len() {
        result.push_str(&style(&text[plain_start..]));
    }
    result
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
    search_match_style: StyleFn,
    search_current_match_style: StyleFn,
    search_navigation_button_style: NavigationButtonStyle,
    scroll_to_end_indicator: Option<LabelFn>,
    open_url: Option<OpenUrl>,
    copy_on_select: bool,
    copy_selection: Option<CopySelection>,
    pending_copies: Vec<Receiver<Result<(), String>>>,
    flashes: AltScreenFlashContainer,
    indicator_rect: Option<(i64, i64, i64)>,
    selection_anchor: Option<SelectionPoint>,
    selection_focus: Option<SelectionPoint>,
    selection_granularity: Granularity,
    selection_initial_range: Option<SelectionRange>,
    last_click: Option<ClickTarget>,
    selection_drag_pointer: Option<(i64, i64)>,
    selection_auto_scroll_direction: i64,
    selection_auto_scroll_at: Option<Instant>,
    selection_press_active: bool,
    selection_dragged: bool,
    pressed_url: Option<String>,
    scrollbar_drag: Option<ScrollbarDrag>,
    scrollbar_hover: Option<ScrollRef>,
    search: Option<ActiveSearch>,
    mouse_capture: Option<TuiMouseDispatchTarget>,
    mouse_press_target: Option<TuiMouseDispatchTarget>,
    mouse_press_point: Option<(i64, i64)>,
    mouse_press_moved: bool,
    last_component_click: Option<ComponentClick>,
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
            search_match_style: options.search_match_style,
            search_current_match_style: options.search_current_match_style,
            search_navigation_button_style: options.search_navigation_button_style,
            scroll_to_end_indicator: options.scroll_to_end_indicator,
            open_url: options.open_url,
            copy_on_select: options.copy_on_select,
            copy_selection: options.copy_selection,
            pending_copies: Vec::new(),
            flashes: AltScreenFlashContainer::default(),
            indicator_rect: None,
            selection_anchor: None,
            selection_focus: None,
            selection_granularity: Granularity::Character,
            selection_initial_range: None,
            last_click: None,
            selection_drag_pointer: None,
            selection_auto_scroll_direction: 0,
            selection_auto_scroll_at: None,
            selection_press_active: false,
            selection_dragged: false,
            pressed_url: None,
            scrollbar_drag: None,
            scrollbar_hover: None,
            search: None,
            mouse_capture: None,
            mouse_press_target: None,
            mouse_press_point: None,
            mouse_press_moved: false,
            last_component_click: None,
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

    pub fn copy_on_select(&self) -> bool {
        self.copy_on_select
    }

    pub fn set_copy_on_select(&mut self, enabled: bool) {
        self.copy_on_select = enabled;
    }

    pub fn has_active_selection(&self) -> bool {
        self.active_selection_text().is_some()
    }

    pub fn copy_active_selection_to_clipboard(&mut self) -> bool {
        match self.active_selection_text() {
            Some(text) => self.copy_text_to_clipboard(text),
            None => false,
        }
    }

    pub fn reset_text_selection(&mut self) {
        self.clear_text_selection();
        self.last_click = None;
    }

    pub fn flash(&mut self, message: &str, duration: Option<Duration>) {
        self.flashes
            .flash(message, duration.unwrap_or(DEFAULT_FLASH_DURATION));
        self.core.request_render(false);
    }

    pub fn screen_lines(&self) -> Vec<String> {
        self.previous_screen.clone()
    }

    pub fn set_layout_root(&mut self, root: Option<ComponentRef>) {
        let same = match (&self.layout_root, &root) {
            (Some(a), Some(b)) => same(a, b),
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

    fn reset_pointer_state(&mut self) {
        self.stop_selection_auto_scroll();
        self.selection_press_active = false;
        self.stop_scrollbar_hover();
        self.stop_scrollbar_drag();
        self.flashes.dispose();
    }

    pub fn start(&mut self) {
        self.reset_pointer_state();
        self.core.stopped = false;
        self.active = true;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.selection_granularity = Granularity::Character;
        self.selection_initial_range = None;
        self.last_click = None;
        self.pressed_url = None;
        self.selection_dragged = false;
        self.clear_component_mouse_gesture();
        self.last_component_click = None;
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
        self.close_search();
        self.reset_pointer_state();
        self.clear_component_mouse_gesture();
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

    fn toggle_search(&mut self) {
        if self.search.is_some() {
            self.close_search();
            return;
        }
        let component = Rc::new(RefCell::new(AltScreenSearchComponent::new(
            self.search_navigation_button_style.clone(),
        )));
        let anchor_row = self.primary_scroll().borrow().scroll_top() as i64;
        let overlay = self.core.show_overlay(
            component.clone(),
            Some(OverlayOptions {
                anchor: Some(OverlayAnchor::TopRight),
                width: Some(SizeValue::Percent(40.0)),
                min_width: Some(32),
                margin: Some(OverlayMargin::all(1)),
                ..OverlayOptions::default()
            }),
        );
        self.search = Some(ActiveSearch {
            component,
            index: AltScreenSearchIndex::default(),
            overlay,
            query: String::new(),
            matches: Rc::new(Vec::new()),
            selected_index: -1,
            selected_key: None,
            anchor_row,
            mode: SearchSelection::Query,
        });
    }

    fn close_search(&mut self) {
        let Some(search) = self.search.take() else {
            return;
        };
        self.core.hide_overlay_id(search.overlay);
        self.core.request_render(false);
    }

    fn update_search_query(&mut self, query: String) {
        let scroll_top = self.primary_scroll().borrow().scroll_top() as i64;
        let Some(search) = &mut self.search else {
            return;
        };
        if query == search.query {
            return;
        }
        let selected = usize::try_from(search.selected_index)
            .ok()
            .and_then(|index| search.matches.get(index));
        search.anchor_row = selected
            .and_then(|found| found.segments.first())
            .map_or(scroll_top, |segment| segment.row);
        search.query = query;
        search.mode = SearchSelection::Query;
        search.component.borrow_mut().set_result(-1, 0);
        self.core.request_render(false);
    }

    fn navigate_search(&mut self, direction: i64) {
        let Some(search) = &mut self.search else {
            return;
        };
        if search.query.is_empty() {
            return;
        }
        search.mode = if direction < 0 {
            SearchSelection::Previous
        } else {
            SearchSelection::Next
        };
        self.core.request_render(false);
    }

    fn search_navigation_direction_at(&self, x: i64, y: i64) -> Option<i64> {
        let search = self.search.as_ref()?;
        let bounds = self.core.overlay_bounds(search.overlay)?;
        if x < bounds.col
            || x >= bounds.col + bounds.width
            || y < bounds.row
            || y >= bounds.row + bounds.height
        {
            return None;
        }
        search
            .component
            .borrow()
            .navigation_direction_at(y - bounds.row, x - bounds.col)
    }

    fn handle_search_mouse_event(&mut self, event: &SgrMouse) -> bool {
        let Some(search) = &self.search else {
            return false;
        };
        let direction = self.search_navigation_direction_at(event.x, event.y);
        if search
            .component
            .borrow_mut()
            .set_hovered_navigation_direction(direction)
        {
            self.core.request_render(false);
        }
        let Some(direction) = direction else {
            return false;
        };
        if event.release || event.button & 32 != 0 || event.button & 3 != 0 {
            return false;
        }
        self.navigate_search(direction);
        true
    }

    fn refresh_search(&mut self, frame: &LayoutFrame) -> bool {
        let scroll = frame
            .primary
            .clone()
            .unwrap_or_else(|| self.implicit.clone());
        let Some(search) = &mut self.search else {
            return false;
        };
        let layout = scroll_view_box(frame, &scroll);
        let lines = match layout.and_then(|b| b.scroll_content_lines.clone()) {
            Some(lines) if !js_trim(&search.query).is_empty() => lines,
            _ => {
                search.matches = Rc::new(Vec::new());
                search.selected_index = -1;
                search.selected_key = None;
                search.mode = SearchSelection::Retain;
                search.component.borrow_mut().set_result(-1, 0);
                return false;
            }
        };
        let reveal = search.mode != SearchSelection::Retain;
        let (matches, changed) = search.index.search(&lines, &search.query);
        search.matches = matches.clone();
        if !changed && search.mode == SearchSelection::Retain {
            return false;
        }
        let exact = if changed {
            search
                .selected_key
                .as_ref()
                .and_then(|key| matches.iter().position(|found| found.key() == *key))
                .map_or(-1, |index| index as i64)
        } else {
            search.selected_index
        };
        let count = matches.len() as i64;
        let mut selected = -1;
        if count > 0 {
            let base = if exact >= 0 {
                exact
            } else {
                search.selected_index.min(count - 1)
            };
            selected = match search.mode {
                SearchSelection::Query => {
                    let low = matches.partition_point(|found| {
                        found.segments.first().map_or(0, |segment| segment.row) < search.anchor_row
                    }) as i64;
                    if low < count { low } else { 0 }
                }
                SearchSelection::Next if base < 0 => 0,
                SearchSelection::Next => (base + 1) % count,
                SearchSelection::Previous if base < 0 => count - 1,
                SearchSelection::Previous => (base - 1 + count) % count,
                SearchSelection::Retain if exact >= 0 => exact,
                SearchSelection::Retain => search.selected_index.max(0).min(count - 1),
            };
        }
        let chosen = usize::try_from(selected)
            .ok()
            .and_then(|index| matches.get(index));
        search.selected_index = selected;
        search.selected_key = chosen.map(SearchMatch::key);
        search.mode = SearchSelection::Retain;
        search
            .component
            .borrow_mut()
            .set_result(selected, matches.len());
        if !reveal {
            return false;
        }
        let (Some(first), Some(last)) = (
            chosen.and_then(|found| found.segments.first()),
            chosen.and_then(|found| found.segments.last()),
        ) else {
            return false;
        };
        let viewport = scroll.borrow().viewport_height() as i64;
        if layout.is_none() || viewport <= 0 {
            return false;
        }
        let before = scroll.borrow().scroll_top() as i64;
        let visible_bottom = before + viewport - 1;
        let mut target = before;
        if first.row < before || last.row > visible_bottom {
            target = first.row - viewport / 3;
        }
        scroll.borrow_mut().scroll_to(target, true);
        scroll.borrow().scroll_top() as i64 != before
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
                        let query = self
                            .search
                            .as_ref()
                            .map(|search| search.component.borrow().query().to_owned());
                        if let Some(query) = query {
                            self.update_search_query(query);
                        }
                    }
                }
                TerminalEvent::Resize => self.core.request_render(false),
            }
        }
        self.flush_cursor_hide();
    }

    fn search_overlay_focused(&self) -> bool {
        self.search
            .as_ref()
            .is_some_and(|search| self.core.is_overlay_focused(search.overlay))
    }

    fn handle_focus_out(&mut self) {
        let had_active_selection = self.selection_press_active;
        let had_visible_selection = had_active_selection && self.selection_bounds().is_some();
        self.selection_press_active = false;
        self.stop_selection_auto_scroll();
        self.stop_scrollbar_hover();
        if self.search.as_ref().is_some_and(|search| {
            search
                .component
                .borrow_mut()
                .set_hovered_navigation_direction(None)
        }) {
            self.core.request_render(false);
        }
        self.stop_scrollbar_drag();
        self.pressed_url = None;
        self.selection_dragged = false;
        self.clear_component_mouse_gesture();
        self.last_component_click = None;
        if had_active_selection {
            self.selection_anchor = None;
            self.selection_focus = None;
            self.selection_granularity = Granularity::Character;
            self.selection_initial_range = None;
            if had_visible_selection {
                self.core.request_render(false);
            }
        }
        self.last_click = None;
    }

    fn handle_viewport_input(&mut self, data: &str) -> bool {
        if data == FOCUS_OUT {
            self.handle_focus_out();
            return true;
        }
        if data == FOCUS_IN {
            return true;
        }
        if let Some((direction, raw)) = wheel_event(data) {
            let now = self.epoch.elapsed().as_secs_f64() * 1000.0;
            let lines = self.wheel.next(direction, now);
            let delta = direction
                * if raw.button & 8 != 0 {
                    lines * ALT_WHEEL_SCROLL_MULTIPLIER
                } else {
                    lines
                };
            let event = self.mouse_event(MouseEventType::Wheel, &raw, Some(delta), None);
            let result = match self.core.dispatch_mouse_to_overlay(&event) {
                OverlayMouse::Hit(result) => result,
                OverlayMouse::Missed => self.dispatch_mouse_to_layout(&event),
            };
            if let Some(result) = result {
                if self.apply_mouse_dispatch_result(&event, &result) {
                    self.core.request_render(false);
                }
                return true;
            }
            self.route_wheel(raw.x, raw.y, delta);
            return true;
        }
        if let Some(event) = parse_sgr_mouse(data) {
            self.handle_mouse_event(&event);
            return true;
        }
        if is_mouse_sequence(data) {
            return true;
        }
        let kb = get_keybindings();
        let release = is_key_release(data);
        if kb.matches(data, "tui.altScreen.search") {
            if !release {
                self.toggle_search();
            }
            return true;
        }
        if self.search_overlay_focused() {
            let action: Option<fn(&mut Self)> = if kb.matches(data, "tui.altScreen.searchNext") {
                Some(|tui| tui.navigate_search(1))
            } else if kb.matches(data, "tui.altScreen.searchPrevious") {
                Some(|tui| tui.navigate_search(-1))
            } else if kb.matches(data, "tui.altScreen.searchClose") {
                Some(Self::close_search)
            } else {
                None
            };
            if let Some(action) = action {
                if !release {
                    action(self);
                }
                return true;
            }
        }
        if self.core.is_overlay_focused_any() && !self.search_overlay_focused() {
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

    fn mouse_event(
        &self,
        kind: MouseEventType,
        raw: &SgrMouse,
        wheel_delta: Option<i64>,
        click_count: Option<u32>,
    ) -> TuiMouseEvent {
        TuiMouseEvent {
            kind,
            button: if kind == MouseEventType::Wheel {
                MouseButton::None
            } else {
                decode_mouse_button(raw.button)
            },
            x: raw.x,
            y: raw.y,
            screen_x: raw.x,
            screen_y: raw.y,
            width: self.terminal.columns().max(1) as i64,
            height: self.terminal.rows().max(1) as i64,
            shift: raw.button & 4 != 0,
            alt: raw.button & 8 != 0,
            ctrl: raw.button & 16 != 0,
            wheel_delta,
            click_count,
        }
    }

    fn dispatch_mouse_to_layout(&self, event: &TuiMouseEvent) -> Option<TuiMouseDispatchResult> {
        let frame = self.current.as_ref()?;
        let mut visited: Vec<ComponentRef> = Vec::new();
        for layout in layout_boxes_at(frame, event.screen_x, event.screen_y) {
            if visited.iter().any(|seen| same(seen, &layout.component)) {
                continue;
            }
            visited.push(layout.component.clone());
            let result = dispatch_mouse_event(
                &layout.component,
                &TuiMouseEvent {
                    x: event.screen_x - layout.rect.x,
                    y: event.screen_y - layout.rect.y,
                    width: layout.rect.width,
                    height: layout.rect.height,
                    ..*event
                },
            );
            if result.is_some() {
                return result;
            }
        }
        None
    }

    fn dispatch_mouse_to_screen(&self, event: &TuiMouseEvent) -> Option<TuiMouseDispatchResult> {
        match self.core.dispatch_mouse_to_overlay(event) {
            OverlayMouse::Hit(result) => result,
            OverlayMouse::Missed => self.dispatch_mouse_to_layout(event),
        }
    }

    fn apply_mouse_dispatch_result(
        &mut self,
        event: &TuiMouseEvent,
        result: &TuiMouseDispatchResult,
    ) -> bool {
        let component = result
            .focus_target
            .clone()
            .unwrap_or_else(|| result.target.component.clone());
        let focus_target = self.core.resolve_mouse_focus_target(&component);
        let focus_changed = result.focus
            && !self
                .core
                .focused_component()
                .is_some_and(|focused| same(&focused, &focus_target));
        if result.focus {
            self.core.set_focus(Some(focus_target));
        }
        if result.capture {
            self.mouse_capture = Some(result.target.clone());
        }
        result.render.unwrap_or(
            focus_changed
                || matches!(
                    event.kind,
                    MouseEventType::Press
                        | MouseEventType::Click
                        | MouseEventType::Drag
                        | MouseEventType::Wheel
                ),
        )
    }

    fn component_click_count(&mut self, target: &TuiMouseDispatchTarget, x: i64, y: i64) -> u32 {
        let now = Instant::now();
        let count = match &self.last_component_click {
            Some(previous)
                if now.duration_since(previous.at) <= DOUBLE_CLICK_INTERVAL
                    && same(&previous.component, &target.component)
                    && previous.x == x
                    && previous.y == y =>
            {
                previous.count % 3 + 1
            }
            _ => 1,
        };
        self.last_component_click = Some(ComponentClick {
            at: now,
            count,
            component: target.component.clone(),
            x,
            y,
        });
        count
    }

    fn clear_component_mouse_gesture(&mut self) {
        self.mouse_capture = None;
        self.mouse_press_target = None;
        self.mouse_press_point = None;
        self.mouse_press_moved = false;
    }

    fn clear_text_selection(&mut self) {
        self.stop_selection_auto_scroll();
        self.selection_press_active = false;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.selection_granularity = Granularity::Character;
        self.selection_initial_range = None;
        self.pressed_url = None;
        self.selection_dragged = false;
    }

    fn handle_mouse_event(&mut self, raw: &SgrMouse) {
        let kind = if raw.release {
            MouseEventType::Release
        } else if raw.button & 32 != 0 {
            if decode_mouse_button(raw.button) == MouseButton::None {
                MouseEventType::Move
            } else {
                MouseEventType::Drag
            }
        } else {
            MouseEventType::Press
        };
        let event = self.mouse_event(kind, raw, None, None);

        if let Some(target) = self
            .mouse_capture
            .clone()
            .or_else(|| self.mouse_press_target.clone())
        {
            if self
                .mouse_press_point
                .is_some_and(|point| point != (raw.x, raw.y))
            {
                self.mouse_press_moved = true;
                self.last_component_click = None;
            }
            let mut render = false;
            if let Some(result) =
                dispatch_mouse_event(&target.component, &retarget_mouse_event(&event, &target))
            {
                render = self.apply_mouse_dispatch_result(&event, &result);
            }
            if raw.release {
                if !self.mouse_press_moved && self.mouse_press_point == Some((raw.x, raw.y)) {
                    let count = self.component_click_count(&target, raw.x, raw.y);
                    let click = self.mouse_event(MouseEventType::Click, raw, None, Some(count));
                    if let Some(result) = dispatch_mouse_event(
                        &target.component,
                        &retarget_mouse_event(&click, &target),
                    ) {
                        render = self.apply_mouse_dispatch_result(&click, &result) || render;
                    }
                }
                self.clear_component_mouse_gesture();
            }
            if render {
                self.core.request_render(false);
            }
            return;
        }

        if self.handle_search_mouse_event(raw) {
            return;
        }

        let overlay = self.core.dispatch_mouse_to_overlay(&event);
        let result = match overlay {
            OverlayMouse::Missed => {
                if self.handle_scroll_to_end_indicator_mouse_event(raw) {
                    return;
                }
                let scrollbar_handled = self.handle_scrollbar_mouse_event(raw);
                if self.scrollbar_drag.is_none() {
                    self.update_scrollbar_hover(raw.x, raw.y);
                }
                if scrollbar_handled {
                    return;
                }
                self.dispatch_mouse_to_layout(&event)
            }
            OverlayMouse::Hit(result) => {
                self.stop_scrollbar_hover();
                result
            }
        };
        if let Some(result) = result {
            let render = self.apply_mouse_dispatch_result(&event, &result);
            if kind == MouseEventType::Press {
                self.clear_text_selection();
                self.mouse_press_target = Some(result.target.clone());
                self.mouse_press_point = Some((raw.x, raw.y));
                self.mouse_press_moved = false;
            }
            if render {
                self.core.request_render(false);
            }
            return;
        }
        self.handle_selection_mouse_event(raw);
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
        self.update_scrollbar_hover(x, y);
        self.core.request_render(false);
    }

    fn handle_scroll_to_end_indicator_mouse_event(&mut self, event: &SgrMouse) -> bool {
        let Some((row, column, width)) = self.indicator_rect else {
            return false;
        };
        if event.release || event.button & 32 != 0 || event.button & 3 != 0 {
            return false;
        }
        if event.y != row || event.x < column || event.x >= column + width {
            return false;
        }
        self.scroll_to_bottom();
        true
    }

    fn scrollbar_target_at(
        &self,
        x: i64,
        y: i64,
        include_hidden_auto: bool,
    ) -> Option<(ScrollRef, ScrollbarGeometry)> {
        if self.core.has_overlay() {
            return None;
        }
        let frame = self.current.as_ref()?;
        scroll_views_at(frame, x, y).into_iter().find_map(|scroll| {
            let geometry =
                scrollbar_geometry(scroll_view_box(frame, &scroll)?, include_hidden_auto)?;
            (x == geometry.column
                && y >= geometry.track_top
                && y < geometry.track_top + geometry.track_height)
                .then_some((scroll, geometry))
        })
    }

    fn set_scrollbar_hover(&mut self, scroll: Option<ScrollRef>) {
        if same_scroll(&scroll, &self.scrollbar_hover) {
            return;
        }
        if let Some(previous) = &self.scrollbar_hover {
            previous.borrow_mut().set_scrollbar_active(false);
        }
        self.scrollbar_hover = scroll;
        if let Some(next) = &self.scrollbar_hover {
            next.borrow_mut().set_scrollbar_active(true);
        }
    }

    fn update_scrollbar_hover(&mut self, x: i64, y: i64) {
        let target = self
            .scrollbar_target_at(x, y, true)
            .map(|(scroll, _)| scroll);
        self.set_scrollbar_hover(target);
    }

    fn stop_scrollbar_hover(&mut self) {
        self.set_scrollbar_hover(None);
    }

    fn stop_scrollbar_drag(&mut self) {
        self.scrollbar_drag = None;
    }

    fn scroll_scrollbar_to_pointer(
        scroll: &ScrollRef,
        geometry: &ScrollbarGeometry,
        pointer_y: i64,
        grab_offset: i64,
    ) {
        let max_thumb_offset = geometry.track_height - geometry.thumb_height;
        let thumb_offset = (pointer_y - geometry.track_top - grab_offset)
            .min(max_thumb_offset)
            .max(0);
        let scroll_top = if max_thumb_offset == 0 {
            0
        } else {
            (thumb_offset as f64 / max_thumb_offset as f64 * geometry.max_scroll_top as f64).round()
                as i64
        };
        scroll.borrow_mut().scroll_to(scroll_top, false);
    }

    fn handle_scrollbar_mouse_event(&mut self, event: &SgrMouse) -> bool {
        if let Some(drag) = &self.scrollbar_drag {
            if event.release {
                self.stop_scrollbar_drag();
                return true;
            }
            let geometry = self
                .current
                .as_ref()
                .and_then(|frame| scroll_view_box(frame, &drag.scroll))
                .and_then(|layout| scrollbar_geometry(layout, false));
            if let Some(geometry) = geometry {
                Self::scroll_scrollbar_to_pointer(
                    &drag.scroll,
                    &geometry,
                    event.y,
                    drag.grab_offset,
                );
            }
            return true;
        }
        if event.release || event.button & 32 != 0 || event.button & 3 != 0 {
            return false;
        }
        let Some((scroll, geometry)) = self.scrollbar_target_at(event.x, event.y, false) else {
            return false;
        };
        self.stop_selection_auto_scroll();
        self.selection_press_active = false;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.selection_granularity = Granularity::Character;
        self.selection_initial_range = None;
        self.last_click = None;
        self.pressed_url = None;
        self.selection_dragged = false;
        self.set_scrollbar_hover(Some(scroll.clone()));
        let on_thumb =
            event.y >= geometry.thumb_top && event.y < geometry.thumb_top + geometry.thumb_height;
        let grab_offset = if on_thumb {
            event.y - geometry.thumb_top
        } else {
            geometry.thumb_height / 2
        };
        if !on_thumb {
            Self::scroll_scrollbar_to_pointer(&scroll, &geometry, event.y, grab_offset);
        }
        self.scrollbar_drag = Some(ScrollbarDrag {
            scroll,
            grab_offset,
        });
        true
    }

    fn visible_rows(&self, scroll: &ScrollRef) -> Option<(i64, i64, &LayoutBox)> {
        let layout = scroll_view_box(self.current.as_ref()?, scroll)?;
        if layout.rect.height <= 0 || layout.clip.height <= 0 {
            return None;
        }
        let top = layout.rect.y.max(layout.clip.y).max(0);
        let bottom = (self.terminal.rows() as i64 - 1)
            .min(layout.rect.y + layout.rect.height - 1)
            .min(layout.clip.y + layout.clip.height - 1);
        Some((top, bottom, layout))
    }

    fn scroll_selection_point(&self, scroll: &ScrollRef, x: i64, y: i64) -> Option<SelectionPoint> {
        let (top, bottom, layout) = self.visible_rows(scroll)?;
        if bottom < top {
            return None;
        }
        let pointer_row = y.min(bottom).max(top);
        let content_rows = layout
            .scroll_content_lines
            .as_ref()
            .map_or(1, |lines| lines.len() as i64);
        let max_content_row = (content_rows - 1).max(0);
        Some(SelectionPoint {
            row: (scroll.borrow().scroll_top() as i64 + pointer_row - layout.rect.y)
                .min(max_content_row)
                .max(0),
            col: (x - layout.rect.x).min(layout.rect.width - 1).max(0),
            scroll: Some(scroll.clone()),
            boundary: false,
        })
    }

    fn selection_point(&self, x: i64, y: i64, scroll: Option<&ScrollRef>) -> SelectionPoint {
        if let Some(point) = scroll.and_then(|scroll| self.scroll_selection_point(scroll, x, y)) {
            return point;
        }
        SelectionPoint {
            row: y.min(self.terminal.rows() as i64 - 1).max(0),
            col: x.min(self.terminal.columns() as i64 - 1).max(0),
            scroll: None,
            boundary: false,
        }
    }

    fn selection_source_line(&self, point: &SelectionPoint) -> String {
        if let (Some(scroll), Some(frame)) = (&point.scroll, &self.current)
            && let Some(lines) = scroll_view_box(frame, scroll)
                .and_then(|layout| layout.scroll_content_lines.as_ref())
        {
            return usize::try_from(point.row)
                .ok()
                .and_then(|row| lines.get(row))
                .cloned()
                .unwrap_or_default();
        }
        usize::try_from(point.row)
            .ok()
            .and_then(|row| self.previous_screen.get(row))
            .cloned()
            .unwrap_or_default()
    }

    fn word_selection(&self, point: &SelectionPoint) -> Option<SelectionRange> {
        let line = strip_terminal_sequences(&self.selection_source_line(point));
        let mut segments: Vec<WordSegmentSpan> = Vec::new();
        let mut start = 0;
        for segment in word_segments(&line) {
            let end = start + visible_width(segment.segment) as i64;
            let joiner = TERMINAL_WORD_SELECTION_JOINERS.contains(&segment.segment);
            segments.push(WordSegmentSpan {
                start,
                end,
                selectable: segment.is_word_like || joiner,
                joiner,
            });
            start = end;
        }
        let clicked = segments
            .iter()
            .position(|segment| point.col >= segment.start && point.col < segment.end)?;
        let can_join = |left: &WordSegmentSpan, right: &WordSegmentSpan| {
            left.selectable && right.selectable && (left.joiner || right.joiner)
        };
        let mut selection_start = segments[clicked].start;
        let mut selection_end = segments[clicked].end;
        let mut index = clicked;
        while index > 0 && can_join(&segments[index - 1], &segments[index]) {
            selection_start = segments[index - 1].start;
            index -= 1;
        }
        let mut index = clicked;
        while index + 1 < segments.len() && can_join(&segments[index], &segments[index + 1]) {
            selection_end = segments[index + 1].end;
            index += 1;
        }
        Some(SelectionRange {
            start: SelectionPoint {
                col: selection_start,
                ..point.clone()
            },
            end: SelectionPoint {
                col: selection_end,
                boundary: true,
                ..point.clone()
            },
        })
    }

    fn line_selection(&self, point: &SelectionPoint) -> SelectionRange {
        SelectionRange {
            start: SelectionPoint {
                col: 0,
                ..point.clone()
            },
            end: SelectionPoint {
                col: visible_width(&self.selection_source_line(point)) as i64,
                boundary: true,
                ..point.clone()
            },
        }
    }

    fn update_selection_focus(&mut self, point: SelectionPoint) {
        let Some(initial) = self.selection_initial_range.clone() else {
            self.selection_focus = Some(point);
            return;
        };
        let range = match self.selection_granularity {
            Granularity::Character => {
                self.selection_focus = Some(point);
                return;
            }
            Granularity::Word => self.word_selection(&point),
            Granularity::Line => Some(self.line_selection(&point)),
        };
        let Some(range) = range else {
            return;
        };
        let before_initial = range.start.row < initial.start.row
            || (range.start.row == initial.start.row && range.start.col < initial.start.col);
        if before_initial {
            self.selection_anchor = Some(initial.end);
            self.selection_focus = Some(range.start);
        } else {
            self.selection_anchor = Some(initial.start);
            self.selection_focus = Some(range.end);
        }
    }

    fn click_count(&mut self, point: &SelectionPoint, word: Option<&SelectionRange>) -> u32 {
        let now = Instant::now();
        let count = match (word, &self.last_click) {
            (Some(word), Some(previous))
                if now.duration_since(previous.at) <= DOUBLE_CLICK_INTERVAL
                    && previous.row == point.row
                    && same_scroll(&previous.scroll, &point.scroll)
                    && previous.word_start == word.start.col
                    && previous.word_end == word.end.col =>
            {
                previous.count % 3 + 1
            }
            _ => 1,
        };
        self.last_click = word.map(|word| ClickTarget {
            at: now,
            count,
            row: point.row,
            scroll: point.scroll.clone(),
            word_start: word.start.col,
            word_end: word.end.col,
        });
        count
    }

    fn update_selection_auto_scroll(&mut self, event: &SgrMouse) {
        let Some(scroll) = self
            .selection_anchor
            .as_ref()
            .and_then(|anchor| anchor.scroll.clone())
        else {
            self.stop_selection_auto_scroll();
            return;
        };
        let Some((top, bottom, _)) = self.visible_rows(&scroll) else {
            self.stop_selection_auto_scroll();
            return;
        };
        self.selection_drag_pointer = Some((event.x, event.y));
        self.selection_auto_scroll_direction = if event.y <= top {
            -1
        } else if event.y >= bottom {
            1
        } else {
            0
        };
        if self.selection_auto_scroll_direction == 0 {
            self.stop_selection_auto_scroll();
            return;
        }
        if self.selection_auto_scroll_at.is_none() {
            self.selection_auto_scroll_at = Some(Instant::now() + SELECTION_AUTO_SCROLL_INTERVAL);
        }
    }

    fn auto_scroll_selection(&mut self) {
        let scroll = self
            .selection_anchor
            .as_ref()
            .and_then(|anchor| anchor.scroll.clone());
        let direction = self.selection_auto_scroll_direction;
        let (Some(scroll), Some((x, y))) = (scroll, self.selection_drag_pointer) else {
            self.stop_selection_auto_scroll();
            return;
        };
        if direction == 0 {
            self.stop_selection_auto_scroll();
            return;
        }
        let remaining = scroll.borrow_mut().scroll_by(direction);
        if remaining == direction {
            self.stop_selection_auto_scroll();
            return;
        }
        if let Some(point) = self.scroll_selection_point(&scroll, x, y) {
            self.update_selection_focus(point);
        }
        self.core.request_render(false);
    }

    fn stop_selection_auto_scroll(&mut self) {
        self.selection_auto_scroll_at = None;
        self.selection_auto_scroll_direction = 0;
        self.selection_drag_pointer = None;
    }

    fn handle_selection_mouse_event(&mut self, event: &SgrMouse) {
        let button = event.button & 3;
        if button != 0 && !(event.release && button == 3) {
            return;
        }
        let anchor_scroll = self
            .selection_anchor
            .as_ref()
            .and_then(|anchor| anchor.scroll.clone());
        let point = self.selection_point(event.x, event.y, anchor_scroll.as_ref());
        if event.release {
            if !self.selection_press_active {
                return;
            }
            self.selection_press_active = false;
            self.stop_selection_auto_scroll();
            if self.selection_anchor.is_none() {
                return;
            }
            self.update_selection_focus(point.clone());
            let is_click = !self.selection_dragged
                && self.selection_anchor.as_ref().is_some_and(|anchor| {
                    same_scroll(&anchor.scroll, &point.scroll)
                        && anchor.row == point.row
                        && anchor.col == point.col
                });
            let pressed_url = self.pressed_url.take();
            if let (true, Some(url), Some(open_url)) =
                (is_click, pressed_url, self.open_url.clone())
            {
                self.selection_anchor = None;
                self.selection_focus = None;
                open_url(&url);
                self.core.request_render(false);
                return;
            }
            if is_click {
                let count = self.last_click.as_ref().map_or(1, |click| click.count);
                let click = self.mouse_event(MouseEventType::Click, event, None, Some(count));
                if let Some(result) = self.dispatch_mouse_to_screen(&click) {
                    let render = self.apply_mouse_dispatch_result(&click, &result);
                    self.clear_text_selection();
                    if render {
                        self.core.request_render(false);
                    }
                    return;
                }
            }
            if self.copy_on_select
                && let Some(text) = self.active_selection_text()
            {
                self.copy_text_to_clipboard(text);
            }
            self.core.request_render(false);
            return;
        }
        if event.button & 32 != 0 {
            if !self.selection_press_active || self.selection_anchor.is_none() {
                return;
            }
            self.selection_dragged = true;
            self.last_click = None;
            self.pressed_url = None;
            self.update_selection_focus(point);
            self.update_selection_auto_scroll(event);
            self.core.request_render(false);
            return;
        }
        self.stop_selection_auto_scroll();
        self.selection_press_active = true;
        let scroll = if self.core.has_overlay() {
            None
        } else {
            self.current
                .as_ref()
                .and_then(|frame| scroll_views_at(frame, event.x, event.y).into_iter().next())
        };
        let anchor = self.selection_point(event.x, event.y, scroll.as_ref());
        let word = self.word_selection(&anchor);
        let count = self.click_count(&anchor, word.as_ref());
        let range = match count {
            2 => word,
            3 => Some(self.line_selection(&anchor)),
            _ => None,
        };
        self.selection_granularity = match (&range, count) {
            (None, _) => Granularity::Character,
            (Some(_), 2) => Granularity::Word,
            (Some(_), _) => Granularity::Line,
        };
        self.selection_anchor = Some(
            range
                .as_ref()
                .map_or_else(|| anchor.clone(), |r| r.start.clone()),
        );
        self.selection_focus = Some(
            range
                .as_ref()
                .map_or_else(|| anchor.clone(), |r| r.end.clone()),
        );
        self.pressed_url = if range.is_some() {
            None
        } else {
            let row = event.y.min(self.terminal.rows() as i64 - 1).max(0) as usize;
            let column = event.x.min(self.terminal.columns() as i64 - 1).max(0) as usize;
            self.previous_screen
                .get(row)
                .and_then(|line| osc8_link_at_column(line, column))
        };
        self.selection_initial_range = range;
        self.selection_dragged = false;
        self.core.request_render(false);
    }

    fn selection_bounds(&self) -> Option<(SelectionPoint, SelectionPoint)> {
        let anchor = self.selection_anchor.as_ref()?;
        let focus = self.selection_focus.as_ref()?;
        if !same_scroll(&anchor.scroll, &focus.scroll) {
            return None;
        }
        if anchor.row == focus.row && anchor.col == focus.col {
            return None;
        }
        let anchor_first =
            anchor.row < focus.row || (anchor.row == focus.row && anchor.col < focus.col);
        Some(if anchor_first {
            (anchor.clone(), focus.clone())
        } else {
            (focus.clone(), anchor.clone())
        })
    }

    fn active_selection_text(&self) -> Option<String> {
        let (start, end) = self.selection_bounds()?;
        let source: Rc<Vec<String>> = match &start.scroll {
            Some(scroll) => scroll_view_box(self.current.as_ref()?, scroll)?
                .scroll_content_lines
                .clone()?,
            None => Rc::new(self.previous_screen.clone()),
        };
        let mut lines = Vec::new();
        for row in start.row..=end.row {
            let line = usize::try_from(row)
                .ok()
                .and_then(|row| source.get(row))
                .map_or("", String::as_str);
            let (first, last) =
                selection_columns(line, row, (&start, &end), 0, visible_width(line) as i64);
            let sliced =
                slice_by_column(line, first as usize, (last - first).max(0) as usize, true);
            lines.push(js_trim_end(&strip_terminal_sequences(&sliced)).to_owned());
        }
        let text = lines.join("\n");
        (!text.is_empty()).then_some(text)
    }

    fn copy_text_to_clipboard(&mut self, text: String) -> bool {
        match &self.copy_selection {
            Some(copy) => {
                self.pending_copies.push(copy(text));
            }
            None => {
                self.terminal
                    .write(&osc52::sequence(&osc52::base64(text.as_bytes())));
                self.flash("Copied!", None);
            }
        }
        true
    }

    fn poll_copies(&mut self) {
        let mut outcomes = Vec::new();
        self.pending_copies
            .retain(|pending| match pending.try_recv() {
                Ok(outcome) => {
                    outcomes.push(outcome);
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => {
                    outcomes.push(Err("Copy failed".to_owned()));
                    false
                }
            });
        for outcome in outcomes {
            match outcome {
                Ok(()) => self.flash("Copied!", None),
                Err(message) => self.flash(&message, Some(COPY_ERROR_FLASH_DURATION)),
            }
        }
    }

    fn apply_search_highlights(&self, screen: Vec<String>, frame: &LayoutFrame) -> Vec<String> {
        let Some(search) = &self.search else {
            return screen;
        };
        if search.selected_index < 0 || search.matches.is_empty() {
            return screen;
        }
        let scroll = frame
            .primary
            .clone()
            .unwrap_or_else(|| self.implicit.clone());
        let Some(layout) = scroll_view_box(frame, &scroll) else {
            return screen;
        };
        let scrollbar_column = scrollbar_geometry(layout, false).map(|geometry| geometry.column);
        let (rect, clip) = (layout.rect, layout.clip);
        let min_row = rect.y.max(clip.y).max(0);
        let max_row = (screen.len() as i64)
            .min(rect.y + rect.height)
            .min(clip.y + clip.height);
        let min_column = rect.x.max(clip.x).max(0);
        let max_column = (self.terminal.columns() as i64)
            .min(rect.x + rect.width)
            .min(clip.x + clip.width)
            .min(scrollbar_column.unwrap_or(i64::MAX));
        let scroll_top = scroll.borrow().scroll_top() as i64;
        let min_content_row = scroll_top + min_row - rect.y;
        let max_content_row = scroll_top + max_row - rect.y - 1;
        let low = search.matches.partition_point(|found| {
            found.segments.last().map_or(-1, |segment| segment.row) < min_content_row
        });
        let mut ranges_by_row: BTreeMap<usize, Vec<SearchHighlight>> = BTreeMap::new();
        for (index, found) in search.matches.iter().enumerate().skip(low) {
            if found.segments.first().map_or(0, |segment| segment.row) > max_content_row {
                break;
            }
            for segment in &found.segments {
                let row = rect.y + segment.row - scroll_top;
                if row < min_row || row >= max_row {
                    continue;
                }
                let start = min_column.max(rect.x + segment.start_col);
                let end = max_column.min(rect.x + segment.end_col);
                if end <= start {
                    continue;
                }
                ranges_by_row
                    .entry(row as usize)
                    .or_default()
                    .push(SearchHighlight {
                        start,
                        end,
                        current: index as i64 == search.selected_index,
                    });
            }
        }
        let mut result = screen;
        for (row, mut ranges) in ranges_by_row {
            let Some(line) = result.get(row) else {
                continue;
            };
            if is_image_line(line) {
                continue;
            }
            let mut line = line.clone();
            let line_width = visible_width(&line) as i64;
            ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
            for range in ranges {
                let start = range.start.min(line_width);
                let end = range.end.min(line_width);
                if end <= start {
                    continue;
                }
                let style = if range.current {
                    &self.search_current_match_style
                } else {
                    &self.search_match_style
                };
                let before = slice_by_column(&line, 0, start as usize, true);
                let highlighted =
                    slice_by_column(&line, start as usize, (end - start) as usize, true);
                let after = slice_by_column(
                    &line,
                    end as usize,
                    (line_width - end).max(0) as usize,
                    true,
                );
                line = format!(
                    "{before}{}{after}",
                    search_text_highlight(&highlighted, style)
                );
            }
            result[row] = line;
        }
        result
    }

    fn apply_selection(&self, screen: Vec<String>, frame: &LayoutFrame) -> Vec<String> {
        let Some((start, end)) = self.selection_bounds() else {
            return screen;
        };
        let mut min_row = 0;
        let mut max_row = screen.len() as i64 - 1;
        let mut min_column = 0;
        let mut max_column = self.terminal.columns() as i64;
        let (mut first, mut last) = (start.clone(), end.clone());
        if let Some(scroll) = &start.scroll {
            let Some(layout) = scroll_view_box(frame, scroll) else {
                return screen;
            };
            let (rect, clip) = (layout.rect, layout.clip);
            min_row = rect.y.max(clip.y).max(0);
            max_row = (screen.len() as i64 - 1)
                .min(rect.y + rect.height - 1)
                .min(clip.y + clip.height - 1);
            min_column = rect.x.max(clip.x).max(0);
            max_column = (self.terminal.columns() as i64)
                .min(rect.x + rect.width)
                .min(clip.x + clip.width);
            let scroll_top = scroll.borrow().scroll_top() as i64;
            first.row = rect.y + start.row - scroll_top;
            first.col = rect.x + start.col;
            last.row = rect.y + end.row - scroll_top;
            last.col = rect.x + end.col;
        }
        screen
            .into_iter()
            .enumerate()
            .map(|(row, line)| {
                let row = row as i64;
                if row < min_row
                    || row > max_row
                    || row < first.row
                    || row > last.row
                    || is_image_line(&line)
                {
                    return line;
                }
                let line_width = visible_width(&line) as i64;
                let (from, to) =
                    selection_columns(&line, row, (&first, &last), min_column, max_column);
                if to <= from {
                    return line;
                }
                let before = slice_by_column(&line, 0, from as usize, true);
                let selected = slice_by_column(&line, from as usize, (to - from) as usize, true);
                let after =
                    slice_by_column(&line, to as usize, (line_width - to).max(0) as usize, true);
                format!("{before}{}{after}", selection_highlight(&selected))
            })
            .collect()
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
        if row < 0 || row as usize >= screen.len() || is_image_line(&screen[row as usize]) {
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

    fn composite_flashes(&self, screen: Vec<String>, width: usize, height: usize) -> Vec<String> {
        let rendered = self.flashes.render(width);
        let lines = &rendered[rendered.len().saturating_sub(height)..];
        if lines.is_empty() {
            return screen;
        }
        let mut result = screen;
        while result.len() < height {
            result.push(String::new());
        }
        for (row, line) in lines.iter().enumerate() {
            let flash_width = visible_width(line);
            if flash_width == 0 {
                continue;
            }
            result[row] = composite_tui_line(
                &result[row],
                line,
                width.saturating_sub(flash_width),
                flash_width,
                width,
            );
        }
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
            next = earliest(next, state.hide_deadline());
        }
        next
    }

    fn poll_timers(&mut self) -> Option<Instant> {
        let now = Instant::now();
        self.poll_copies();
        if self.selection_auto_scroll_at.is_some_and(|at| at <= now) {
            self.selection_auto_scroll_at = Some(now + SELECTION_AUTO_SCROLL_INTERVAL);
            self.auto_scroll_selection();
        }
        if self.flashes.expire(now) {
            self.core.request_render(false);
        }
        let copies = (!self.pending_copies.is_empty())
            .then(|| now + Duration::from_millis(MIN_RENDER_INTERVAL_MS));
        [
            self.poll_scroll_timers(),
            self.selection_auto_scroll_at,
            self.flashes.deadline(),
            copies,
        ]
        .into_iter()
        .fold(None, earliest)
    }

    pub fn tick(&mut self, now: u64) -> Option<u64> {
        self.flush_cursor_hide();
        let timer = self.poll_timers();
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
        let mut frame = render_layout_frame(&root, width, height);
        if self.refresh_search(&frame) {
            frame = render_layout_frame(&root, width, height);
        }
        let lines: Vec<String> = frame
            .lines
            .iter()
            .map(|line| strip_prompt_zones(line).to_owned())
            .collect();
        let mut screen = resolve_fake_cursors(lines, self.core.show_hardware_cursor);
        screen = self.apply_search_highlights(screen, &frame);
        screen = self.composite_scroll_to_end_indicator(screen, &frame, width);
        if !self.core.overlays.is_empty() {
            screen = self.core.composite_overlays(screen, width, height);
        }
        if screen.len() > height {
            screen = screen.split_off(screen.len() - height);
        }
        screen = self.apply_selection(screen, &frame);
        screen = self.composite_flashes(screen, width, height);
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
