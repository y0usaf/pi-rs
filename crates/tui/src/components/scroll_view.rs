use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::components::text::StyleFn;
use crate::layout::LayoutNode;
use crate::tui::{Component, ComponentRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scrollbar {
    Hidden,
    Auto,
    Always,
}

impl Scrollbar {
    pub fn parse(value: &str) -> Scrollbar {
        match value {
            "always" => Scrollbar::Always,
            "hidden" => Scrollbar::Hidden,
            _ => Scrollbar::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overscroll {
    Chain,
    Contain,
}

pub struct ScrollOptions {
    pub follow_end: bool,
    pub primary: bool,
    pub overscroll: Overscroll,
    pub scrollbar: Scrollbar,
    pub track_style: StyleFn,
    pub thumb_style: StyleFn,
    pub hide_delay: Duration,
}

impl Default for ScrollOptions {
    fn default() -> Self {
        ScrollOptions {
            follow_end: false,
            primary: false,
            overscroll: Overscroll::Chain,
            scrollbar: Scrollbar::Hidden,
            track_style: Rc::new(|text: &str| format!("\x1b[90m{text}\x1b[39m")),
            thumb_style: Rc::new(|text: &str| format!("\x1b[37m{text}\x1b[39m")),
            hide_delay: Duration::from_millis(1000),
        }
    }
}

pub struct ScrollState {
    pub follow_end: bool,
    pub primary: bool,
    pub overscroll: Overscroll,
    pub track_style: StyleFn,
    pub thumb_style: StyleFn,
    scrollbar: Scrollbar,
    hide_delay: Duration,
    scroll_top: usize,
    content_height: usize,
    viewport_height: usize,
    following_end: bool,
    follow_suppressed_at_end: bool,
    laid_out: bool,
    render_requested: bool,
    transient_visible: bool,
    active: bool,
    hide_at: Option<Instant>,
}

pub type ScrollRef = Rc<RefCell<ScrollState>>;

impl ScrollState {
    pub fn new(options: ScrollOptions) -> ScrollState {
        ScrollState {
            follow_end: options.follow_end,
            primary: options.primary,
            overscroll: options.overscroll,
            track_style: options.track_style,
            thumb_style: options.thumb_style,
            scrollbar: options.scrollbar,
            hide_delay: options.hide_delay,
            scroll_top: 0,
            content_height: 0,
            viewport_height: 0,
            following_end: options.follow_end,
            follow_suppressed_at_end: false,
            laid_out: false,
            render_requested: false,
            transient_visible: false,
            active: false,
            hide_at: None,
        }
    }

    pub fn scroll_top(&self) -> usize {
        self.scroll_top
    }

    pub fn is_following_end(&self) -> bool {
        self.following_end
    }

    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    pub fn content_height(&self) -> usize {
        self.content_height
    }

    pub fn scrollbar(&self) -> Scrollbar {
        self.scrollbar
    }

    pub fn is_scrollbar_visible(&self) -> bool {
        match self.scrollbar {
            Scrollbar::Always => self.viewport_height > 0,
            Scrollbar::Auto => self.content_height > self.viewport_height && self.transient_visible,
            Scrollbar::Hidden => false,
        }
    }

    pub fn is_scrollbar_active(&self) -> bool {
        self.active
    }

    fn request_render(&mut self) {
        if self.laid_out {
            self.render_requested = true;
        }
    }

    pub fn take_render_request(&mut self) -> bool {
        std::mem::take(&mut self.render_requested)
    }

    pub fn set_scrollbar(&mut self, scrollbar: Scrollbar) {
        if scrollbar == self.scrollbar {
            return;
        }
        self.scrollbar = scrollbar;
        if scrollbar != Scrollbar::Auto {
            self.hide_transient();
        } else if self.active {
            self.mark_activity();
        }
        self.request_render();
    }

    pub fn content_width(&self, width: usize) -> usize {
        if self.scrollbar == Scrollbar::Always && width > 1 {
            width - 1
        } else {
            width
        }
    }

    fn mark_activity(&mut self) {
        if self.scrollbar != Scrollbar::Auto || self.content_height <= self.viewport_height {
            return;
        }
        self.transient_visible = true;
        self.hide_at = None;
        if self.active {
            return;
        }
        self.hide_at = Some(Instant::now() + self.hide_delay);
    }

    fn hide_transient(&mut self) {
        self.transient_visible = false;
        self.hide_at = None;
    }

    pub fn hide_deadline(&self) -> Option<Instant> {
        self.hide_at
    }

    pub fn fire_timer(&mut self, now: Instant) {
        if self.hide_at.is_some_and(|at| at <= now) {
            self.hide_at = None;
            self.transient_visible = false;
            self.request_render();
        }
    }

    pub fn set_scrollbar_active(&mut self, active: bool) {
        if active == self.active {
            return;
        }
        self.active = active;
        self.mark_activity();
        self.request_render();
    }

    fn max_scroll_top(&self) -> usize {
        self.content_height.saturating_sub(self.viewport_height)
    }

    pub fn scroll_to(&mut self, scroll_top: i64, disable_follow: bool) {
        let max = self.max_scroll_top();
        let next = scroll_top.clamp(0, max as i64) as usize;
        let suppressed = disable_follow && next == max;
        let following = !suppressed && self.follow_end && next == max;
        if next == self.scroll_top
            && following == self.following_end
            && suppressed == self.follow_suppressed_at_end
        {
            return;
        }
        let moved = next != self.scroll_top;
        self.scroll_top = next;
        self.following_end = following;
        self.follow_suppressed_at_end = suppressed;
        if moved {
            self.mark_activity();
        }
        self.request_render();
    }

    pub fn scroll_by(&mut self, lines: i64) -> i64 {
        if lines == 0 {
            return 0;
        }
        let max = self.max_scroll_top() as i64;
        let start = if self.following_end {
            max
        } else {
            self.scroll_top as i64
        };
        let next = (start + lines).clamp(0, max);
        let moved = next - start;
        let was_following = self.following_end;
        self.scroll_top = next as usize;
        self.following_end = self.follow_end && next == max;
        self.follow_suppressed_at_end = false;
        if moved != 0 {
            self.mark_activity();
        }
        if moved != 0 || self.following_end != was_following {
            self.request_render();
        }
        lines - moved
    }

    pub fn scroll_to_start(&mut self) {
        let following = self.follow_end && self.content_height <= self.viewport_height;
        let changed = self.scroll_top != 0 || self.following_end != following;
        self.scroll_top = 0;
        self.following_end = following;
        self.follow_suppressed_at_end = false;
        if changed {
            self.mark_activity();
            self.request_render();
        }
    }

    pub fn scroll_to_end(&mut self) {
        let next = self.max_scroll_top();
        let changed = self.scroll_top != next || self.following_end != self.follow_end;
        self.scroll_top = next;
        self.following_end = self.follow_end;
        self.follow_suppressed_at_end = false;
        if changed {
            self.mark_activity();
            self.request_render();
        }
    }

    pub fn update_layout(&mut self, content_height: usize, viewport_height: usize) {
        self.content_height = content_height;
        self.viewport_height = viewport_height;
        self.laid_out = true;
        let max = self.max_scroll_top();
        if self.following_end {
            self.scroll_top = max;
        } else {
            self.scroll_top = self.scroll_top.min(max);
        }
        if self.scroll_top < max {
            self.follow_suppressed_at_end = false;
        }
        if self.follow_end && self.scroll_top == max && !self.follow_suppressed_at_end {
            self.following_end = true;
        }
        if self.content_height <= self.viewport_height {
            self.hide_transient();
        }
    }
}

pub struct ScrollView {
    child: ComponentRef,
    pub state: ScrollRef,
}

impl ScrollView {
    pub fn new(child: ComponentRef, options: ScrollOptions) -> ScrollView {
        ScrollView {
            child,
            state: Rc::new(RefCell::new(ScrollState::new(options))),
        }
    }

    pub fn with_state(child: ComponentRef, state: ScrollRef) -> ScrollView {
        ScrollView { child, state }
    }
}

impl Component for ScrollView {
    fn render(&mut self, width: usize) -> Vec<String> {
        let content = self.state.borrow().content_width(width);
        let lines = self.child.borrow_mut().render(content);
        if content == width {
            lines
        } else {
            lines.into_iter().map(|line| line + " ").collect()
        }
    }
    fn invalidate(&mut self) {
        self.child.borrow_mut().invalidate();
    }
    fn children(&self) -> Option<Vec<ComponentRef>> {
        Some(vec![self.child.clone()])
    }
    fn layout_node(&self) -> Option<LayoutNode> {
        Some(LayoutNode::Scroll {
            child: self.child.clone(),
            state: self.state.clone(),
        })
    }
}
