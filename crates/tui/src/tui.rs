//! The TUI core (upstream `tui.ts`): components, containers, overlays, focus and input routing.
//! [`crate::tui_main_screen::TuiMainScreen`] does the differential rendering.
//!
//! Components are shared as [`ComponentRef`] (`Rc<RefCell<dyn Component>>`) and compared by
//! pointer, as upstream compares objects. There is no event loop: input is fed with
//! [`TuiCore::handle_terminal_input`] and frames are drawn by the screen's `tick`/`render_now`.

use crate::keys::{is_key_release, matches_key};
use crate::utils::{extract_segments, normalize_terminal_output, slice_by_column, slice_with_width, visible_width};
use std::cell::RefCell;
use std::rc::Rc;

/// A piece of UI that renders to lines.
pub trait Component {
    /// Render to lines no wider than `width` columns.
    fn render(&mut self, width: usize) -> Vec<String>;
    /// Handle keyboard input while focused.
    fn handle_input(&mut self, _data: &str) {}
    /// Whether this component takes keyboard input (upstream: has `handleInput`).
    fn accepts_input(&self) -> bool {
        false
    }
    /// Receive Kitty key release events too.
    fn wants_key_release(&self) -> bool {
        false
    }
    /// Drop cached rendering state.
    fn invalidate(&mut self) {}
    /// Called when focus changes, for components that show a cursor (upstream `Focusable`).
    fn set_focused(&mut self, _focused: bool) {}
    /// Whether this component tracks focus (upstream: has a `focused` field).
    fn is_focusable(&self) -> bool {
        false
    }
    /// Children, for containers (used to find mounted components).
    fn children(&self) -> Option<Vec<ComponentRef>> {
        None
    }
}

pub type ComponentRef = Rc<RefCell<dyn Component>>;

/// Wrap a component for sharing.
pub fn component<C: Component + 'static>(c: C) -> Rc<RefCell<C>> {
    Rc::new(RefCell::new(c))
}

/// Pointer identity of two components.
pub fn same(a: &ComponentRef, b: &ComponentRef) -> bool {
    std::ptr::addr_eq(Rc::as_ptr(a), Rc::as_ptr(b))
}

fn same_opt(a: &Option<ComponentRef>, b: &Option<ComponentRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// Zero-width marker a focused component emits at its cursor; the TUI moves the hardware cursor there.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";
pub const FAKE_CURSOR_START: &str = "\x1b_pi:fc\x07";
pub const FAKE_CURSOR_END: &str = "\x1b_pi:/fc\x07";

/// Wrap text in fake cursor markers.
pub fn render_fake_cursor(text: &str) -> String {
    format!("{FAKE_CURSOR_START}{text}{FAKE_CURSOR_END}")
}

/// Whether a line carries a Kitty or iTerm2 inline image.
pub fn is_image_line(line: &str) -> bool {
    line.contains("\x1b_G") || line.contains("\x1b]1337;File=")
}

/// A component holding other components, rendered top to bottom.
#[derive(Default)]
pub struct Container {
    pub children: Vec<ComponentRef>,
}

impl Container {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_child(&mut self, c: ComponentRef) {
        self.children.push(c);
    }
    pub fn remove_child(&mut self, c: &ComponentRef) {
        if let Some(i) = self.children.iter().position(|x| same(x, c)) {
            self.children.remove(i);
        }
    }
    pub fn clear(&mut self) {
        self.children.clear();
    }
}

impl Component for Container {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for c in &self.children {
            lines.extend(c.borrow_mut().render(width));
        }
        lines
    }
    fn invalidate(&mut self) {
        for c in &self.children {
            c.borrow_mut().invalidate();
        }
    }
    fn children(&self) -> Option<Vec<ComponentRef>> {
        Some(self.children.clone())
    }
}

const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

/// Composite overlay content into a terminal line at a fixed column.
pub fn composite_tui_line(base: &str, overlay: &str, start_col: usize, overlay_width: usize, total_width: usize) -> String {
    if is_image_line(base) {
        return base.to_string();
    }
    let after_start = start_col + overlay_width;
    let seg = extract_segments(base, start_col, after_start, total_width.saturating_sub(after_start), true);
    let (otext, owidth) = slice_with_width(overlay, 0, overlay_width, true);
    let before_pad = start_col.saturating_sub(seg.before_width);
    let overlay_pad = overlay_width.saturating_sub(owidth);
    let actual_before = start_col.max(seg.before_width);
    let actual_overlay = overlay_width.max(owidth);
    let after_target = total_width.saturating_sub(actual_before + actual_overlay);
    let after_pad = after_target.saturating_sub(seg.after_width);
    let result = format!(
        "{}{}{SEGMENT_RESET}{otext}{}{SEGMENT_RESET}{}{}",
        seg.before,
        " ".repeat(before_pad),
        " ".repeat(overlay_pad),
        seg.after,
        " ".repeat(after_pad)
    );
    if visible_width(&result) <= total_width { result } else { slice_by_column(&result, 0, total_width, true) }
}

/// A size: absolute, or a percentage of the terminal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeValue {
    Abs(i64),
    Percent(f64),
}

fn parse_size(v: Option<SizeValue>, reference: usize) -> Option<i64> {
    match v? {
        SizeValue::Abs(n) => Some(n),
        SizeValue::Percent(p) => Some((reference as f64 * p / 100.0).floor() as i64),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayAnchor {
    #[default]
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayMargin {
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
    pub left: i64,
}

impl OverlayMargin {
    pub fn all(n: i64) -> Self {
        OverlayMargin { top: n, right: n, bottom: n, left: n }
    }
}

/// Placement and sizing of an overlay.
#[derive(Clone, Default)]
pub struct OverlayOptions {
    pub width: Option<SizeValue>,
    pub min_width: Option<i64>,
    pub max_height: Option<SizeValue>,
    pub anchor: Option<OverlayAnchor>,
    pub offset_x: Option<i64>,
    pub offset_y: Option<i64>,
    pub row: Option<SizeValue>,
    pub col: Option<SizeValue>,
    pub margin: Option<OverlayMargin>,
    /// Shown only while this returns true for (columns, rows).
    pub visible: Option<Rc<dyn Fn(usize, usize) -> bool>>,
    /// Do not take keyboard focus when shown.
    pub non_capturing: bool,
}

/// Last rendered terminal-relative overlay rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayBounds {
    pub row: i64,
    pub col: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OverlayLayout {
    pub width: i64,
    pub row: i64,
    pub col: i64,
    pub max_height: Option<i64>,
}

fn anchor_row(a: OverlayAnchor, h: i64, avail: i64, top: i64) -> i64 {
    use OverlayAnchor::*;
    match a {
        TopLeft | TopCenter | TopRight => top,
        BottomLeft | BottomCenter | BottomRight => top + avail - h,
        LeftCenter | Center | RightCenter => top + (avail - h).div_euclid(2),
    }
}

fn anchor_col(a: OverlayAnchor, w: i64, avail: i64, left: i64) -> i64 {
    use OverlayAnchor::*;
    match a {
        TopLeft | LeftCenter | BottomLeft => left,
        TopRight | RightCenter | BottomRight => left + avail - w,
        TopCenter | Center | BottomCenter => left + (avail - w).div_euclid(2),
    }
}

pub(crate) fn resolve_overlay_layout(opt: &OverlayOptions, overlay_height: i64, tw: usize, th: usize) -> OverlayLayout {
    let (twi, thi) = (tw as i64, th as i64);
    let m = opt.margin.unwrap_or_default();
    let (mt, mr, mb, ml) = (m.top.max(0), m.right.max(0), m.bottom.max(0), m.left.max(0));
    let avail_w = (twi - ml - mr).max(1);
    let avail_h = (thi - mt - mb).max(1);
    let mut width = parse_size(opt.width, tw).unwrap_or(80.min(avail_w));
    if let Some(mw) = opt.min_width {
        width = width.max(mw);
    }
    width = width.min(avail_w).max(1);
    let max_height = parse_size(opt.max_height, th).map(|h| h.min(avail_h).max(1));
    let eff_h = max_height.map_or(overlay_height, |m| overlay_height.min(m));
    let anchor = opt.anchor.unwrap_or_default();
    let mut row = match opt.row {
        Some(SizeValue::Abs(n)) => n,
        Some(SizeValue::Percent(p)) => mt + ((avail_h - eff_h).max(0) as f64 * (p / 100.0)).floor() as i64,
        None => anchor_row(anchor, eff_h, avail_h, mt),
    };
    let mut col = match opt.col {
        Some(SizeValue::Abs(n)) => n,
        Some(SizeValue::Percent(p)) => ml + ((avail_w - width).max(0) as f64 * (p / 100.0)).floor() as i64,
        None => anchor_col(anchor, width, avail_w, ml),
    };
    row += opt.offset_y.unwrap_or(0);
    col += opt.offset_x.unwrap_or(0);
    row = row.min(thi - mb - eff_h).max(mt);
    col = col.min(twi - mr - width).max(ml);
    OverlayLayout { width, row, col, max_height }
}

/// Identifies a shown overlay (upstream returns an `OverlayHandle` of closures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayId(u64);

pub(crate) struct OverlayEntry {
    pub id: OverlayId,
    pub component: ComponentRef,
    pub options: Option<OverlayOptions>,
    pub pre_focus: Option<ComponentRef>,
    pub hidden: bool,
    pub focus_order: u64,
    pub bounds: Option<OverlayBounds>,
}

#[derive(Clone)]
enum Resume {
    RestoreOverlay,
    FocusTarget(Option<ComponentRef>),
}

#[derive(Clone)]
enum FocusRestore {
    Inactive,
    Eligible(OverlayId),
    Blocked { overlay: OverlayId, blocked_by: ComponentRef, resume: Resume },
}

impl FocusRestore {
    fn overlay(&self) -> Option<OverlayId> {
        match self {
            FocusRestore::Inactive => None,
            FocusRestore::Eligible(o) | FocusRestore::Blocked { overlay: o, .. } => Some(*o),
        }
    }
}

/// A listener's verdict on input: consume it, or replace it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputListenerResult {
    pub consume: bool,
    pub data: Option<String>,
}

pub type InputListener = Box<dyn FnMut(&str) -> Option<InputListenerResult>>;

/// State shared by the screen renderers: root children, focus, overlays, input routing.
pub struct TuiCore {
    pub root: Container,
    focused: Option<ComponentRef>,
    listeners: Vec<(u64, InputListener)>,
    next_listener: u64,
    pub on_debug: Option<Box<dyn FnMut()>>,
    pub(crate) render_requested: bool,
    pub(crate) force_requested: bool,
    pub(crate) immediate: bool,
    pub(crate) show_hardware_cursor: bool,
    pub(crate) clear_on_shrink: bool,
    pub(crate) stopped: bool,
    focus_counter: u64,
    next_overlay: u64,
    pub(crate) overlays: Vec<OverlayEntry>,
    focus_restore: FocusRestore,
    /// Terminal size used for overlay visibility, updated by the screen before each use.
    pub(crate) term_size: (usize, usize),
    /// Set by `hide_terminal_cursor`; the screen writes it out.
    pub(crate) pending_hide_cursor: bool,
}

impl Default for TuiCore {
    fn default() -> Self {
        TuiCore {
            root: Container::new(),
            focused: None,
            listeners: Vec::new(),
            next_listener: 0,
            on_debug: None,
            render_requested: false,
            force_requested: false,
            immediate: false,
            show_hardware_cursor: false,
            clear_on_shrink: false,
            stopped: true,
            focus_counter: 0,
            next_overlay: 0,
            overlays: Vec::new(),
            focus_restore: FocusRestore::Inactive,
            term_size: (80, 24),
            pending_hide_cursor: false,
        }
    }
}

impl TuiCore {
    pub fn add_child(&mut self, c: ComponentRef) {
        self.root.add_child(c);
    }
    pub fn remove_child(&mut self, c: &ComponentRef) {
        self.root.remove_child(c);
    }
    pub fn clear(&mut self) {
        self.root.clear();
    }

    /// Ask for a frame. `force` redraws everything.
    pub fn request_render(&mut self, force: bool) {
        self.render_requested = true;
        if force {
            self.force_requested = true;
            self.immediate = true;
        }
    }

    pub fn focused_component(&self) -> Option<ComponentRef> {
        self.focused.clone()
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        if self.show_hardware_cursor == enabled {
            return;
        }
        self.show_hardware_cursor = enabled;
        if !enabled {
            self.hide_terminal_cursor();
        }
        self.request_render(false);
    }
    pub fn show_hardware_cursor(&self) -> bool {
        self.show_hardware_cursor
    }
    /// Redraw fully when content shrinks, clearing the rows it left.
    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.clear_on_shrink = enabled;
    }
    pub fn clear_on_shrink(&self) -> bool {
        self.clear_on_shrink
    }

    fn hide_terminal_cursor(&mut self) {
        if !self.stopped {
            self.pending_hide_cursor = true;
        }
    }

    /// Add an input listener; returns an id for [`TuiCore::remove_input_listener`].
    pub fn add_input_listener(&mut self, l: InputListener) -> u64 {
        self.next_listener += 1;
        self.listeners.push((self.next_listener, l));
        self.next_listener
    }
    pub fn remove_input_listener(&mut self, id: u64) {
        self.listeners.retain(|(i, _)| *i != id);
    }

    fn entry(&self, id: OverlayId) -> Option<&OverlayEntry> {
        self.overlays.iter().find(|e| e.id == id)
    }
    fn entry_mut(&mut self, id: OverlayId) -> Option<&mut OverlayEntry> {
        self.overlays.iter_mut().find(|e| e.id == id)
    }
    fn entry_of(&self, c: &ComponentRef) -> Option<&OverlayEntry> {
        self.overlays.iter().find(|e| same(&e.component, c))
    }

    pub(crate) fn is_visible(&self, e: &OverlayEntry) -> bool {
        if e.hidden {
            return false;
        }
        match e.options.as_ref().and_then(|o| o.visible.clone()) {
            Some(f) => f(self.term_size.0, self.term_size.1),
            None => true,
        }
    }
    fn is_visible_id(&self, id: OverlayId) -> bool {
        self.entry(id).is_some_and(|e| self.is_visible(e))
    }

    fn topmost_visible(&self) -> Option<OverlayId> {
        let mut top: Option<&OverlayEntry> = None;
        for o in &self.overlays {
            if o.options.as_ref().is_some_and(|x| x.non_capturing) || !self.is_visible(o) {
                continue;
            }
            if top.is_none_or(|t| o.focus_order > t.focus_order) {
                top = Some(o);
            }
        }
        top.map(|o| o.id)
    }

    /// Whether any overlay is visible.
    pub fn has_overlay(&self) -> bool {
        self.overlays.iter().any(|o| self.is_visible(o))
    }

    fn contains(root: &ComponentRef, target: &ComponentRef) -> bool {
        if same(root, target) {
            return true;
        }
        let kids = root.borrow().children();
        kids.is_some_and(|k| k.iter().any(|c| Self::contains(c, target)))
    }

    fn is_mounted(&self, c: &ComponentRef) -> bool {
        self.root.children.iter().any(|r| Self::contains(r, c))
    }

    fn visible_restore(&self) -> FocusRestore {
        match self.focus_restore.overlay() {
            Some(o) if self.is_visible_id(o) => self.focus_restore.clone(),
            _ => FocusRestore::Inactive,
        }
    }

    fn is_focus_ancestor(&self, entry: OverlayId, c: &ComponentRef) -> bool {
        let mut visited: Vec<ComponentRef> = Vec::new();
        let mut cur = self.entry(entry).and_then(|e| e.pre_focus.clone());
        while let Some(x) = cur {
            if visited.iter().any(|v| same(v, &x)) {
                break;
            }
            if same(&x, c) {
                return true;
            }
            cur = self.entry_of(&x).and_then(|e| e.pre_focus.clone());
            visited.push(x);
        }
        false
    }

    fn retarget_pre_focus(&mut self, removed: OverlayId) {
        let Some(e) = self.entry(removed) else { return };
        let (comp, pre) = (e.component.clone(), e.pre_focus.clone());
        for o in &mut self.overlays {
            if o.id != removed && o.pre_focus.as_ref().is_some_and(|p| same(p, &comp)) {
                o.pre_focus = pre.clone();
            }
        }
    }

    fn clear_restore_for(&mut self, id: OverlayId) {
        if self.focus_restore.overlay() == Some(id) {
            self.focus_restore = FocusRestore::Inactive;
        }
    }

    fn resolve_blocked_resume(&mut self, overlay: OverlayId, resume: &Resume) -> Option<ComponentRef> {
        match resume {
            Resume::RestoreOverlay => self.entry(overlay).map(|e| e.component.clone()),
            Resume::FocusTarget(t) => {
                self.focus_restore = FocusRestore::Inactive;
                t.clone()
            }
        }
    }

    /// Give keyboard focus to a component (or none).
    pub fn set_focus(&mut self, c: Option<ComponentRef>) {
        self.set_focus_internal(c, true);
    }

    fn set_focus_internal(&mut self, component: Option<ComponentRef>, clear_restore: bool) {
        let previous = self.focused.clone();
        let mut next = component;
        let prev_overlay = previous
            .as_ref()
            .and_then(|p| self.overlays.iter().find(|e| same(&e.component, p) && self.is_visible(e)).map(|e| e.id));
        let next_is_overlay = next.as_ref().is_some_and(|n| self.entry_of(n).is_some());
        let restore = self.visible_restore();
        if let Some(n) = next.clone().filter(|_| !next_is_overlay) {
            match &restore {
                FocusRestore::Blocked { overlay, blocked_by, resume } if same_opt(&Some(blocked_by.clone()), &previous) => {
                    if matches!(resume, Resume::FocusTarget(_)) || !self.is_mounted(blocked_by) {
                        next = self.resolve_blocked_resume(*overlay, resume);
                    } else {
                        self.focus_restore = FocusRestore::Blocked { overlay: *overlay, blocked_by: n, resume: resume.clone() };
                    }
                }
                _ => {
                    if let Some(po) = prev_overlay {
                        if restore.overlay() == Some(po) && !self.is_focus_ancestor(po, &n) {
                            self.focus_restore = FocusRestore::Blocked { overlay: po, blocked_by: n, resume: Resume::RestoreOverlay };
                        }
                    }
                }
            }
        } else if next.is_none() {
            match &restore {
                FocusRestore::Blocked { overlay, blocked_by, resume } if same_opt(&Some(blocked_by.clone()), &previous) => {
                    next = self.resolve_blocked_resume(*overlay, resume);
                }
                _ => {
                    if clear_restore {
                        self.focus_restore = FocusRestore::Inactive;
                    }
                }
            }
        }
        if let Some(f) = &self.focused {
            if f.borrow().is_focusable() {
                f.borrow_mut().set_focused(false);
            }
        }
        self.focused = next.clone();
        if let Some(n) = &next {
            if n.borrow().is_focusable() {
                n.borrow_mut().set_focused(true);
            }
            if let Some(e) = self.overlays.iter().find(|e| same(&e.component, n) && self.is_visible(e)) {
                self.focus_restore = FocusRestore::Eligible(e.id);
            }
        }
    }

    /// Show an overlay above the content. It takes focus unless `non_capturing`.
    pub fn show_overlay(&mut self, component: ComponentRef, options: Option<OverlayOptions>) -> OverlayId {
        self.next_overlay += 1;
        self.focus_counter += 1;
        let id = OverlayId(self.next_overlay);
        let capturing = !options.as_ref().is_some_and(|o| o.non_capturing);
        self.overlays.push(OverlayEntry {
            id,
            component: component.clone(),
            options,
            pre_focus: self.focused.clone(),
            hidden: false,
            focus_order: self.focus_counter,
            bounds: None,
        });
        if capturing && self.is_visible_id(id) {
            self.set_focus(Some(component));
        }
        self.hide_terminal_cursor();
        self.request_render(false);
        id
    }

    /// Remove an overlay for good.
    pub fn hide_overlay_id(&mut self, id: OverlayId) {
        let Some(idx) = self.overlays.iter().position(|e| e.id == id) else { return };
        self.clear_restore_for(id);
        self.retarget_pre_focus(id);
        let e = self.overlays.remove(idx);
        if self.focused.as_ref().is_some_and(|f| same(f, &e.component)) {
            let top = self.topmost_visible().and_then(|t| self.entry(t)).map(|t| t.component.clone());
            self.set_focus(top.or(e.pre_focus));
        }
        if self.overlays.is_empty() {
            self.hide_terminal_cursor();
        }
        self.request_render(false);
    }

    /// Remove the topmost overlay and restore the previous focus.
    pub fn hide_overlay(&mut self) {
        if let Some(id) = self.overlays.last().map(|e| e.id) {
            self.hide_overlay_id(id);
        }
    }

    /// Temporarily hide or show an overlay.
    pub fn set_overlay_hidden(&mut self, id: OverlayId, hidden: bool) {
        let Some(e) = self.entry(id) else { return };
        if e.hidden == hidden {
            return;
        }
        let comp = e.component.clone();
        let pre = e.pre_focus.clone();
        let capturing = !e.options.as_ref().is_some_and(|o| o.non_capturing);
        self.entry_mut(id).unwrap().hidden = hidden;
        if hidden {
            self.clear_restore_for(id);
            if self.focused.as_ref().is_some_and(|f| same(f, &comp)) {
                let top = self.topmost_visible().and_then(|t| self.entry(t)).map(|t| t.component.clone());
                self.set_focus(top.or(pre));
            }
        } else if capturing && self.is_visible_id(id) {
            self.focus_counter += 1;
            let fc = self.focus_counter;
            self.entry_mut(id).unwrap().focus_order = fc;
            self.set_focus(Some(comp));
        }
        self.request_render(false);
    }

    pub fn is_overlay_hidden(&self, id: OverlayId) -> bool {
        self.entry(id).is_some_and(|e| e.hidden)
    }

    /// Focus an overlay and bring it to the front.
    pub fn focus_overlay(&mut self, id: OverlayId) {
        if !self.is_visible_id(id) {
            return;
        }
        self.focus_counter += 1;
        let fc = self.focus_counter;
        let e = self.entry_mut(id).unwrap();
        e.focus_order = fc;
        let c = e.component.clone();
        self.set_focus(Some(c));
        self.request_render(false);
    }

    /// Release an overlay's focus to the next overlay or its previous target, or to `target`.
    pub fn unfocus_overlay(&mut self, id: OverlayId, target: Option<Option<ComponentRef>>) {
        let Some(e) = self.entry(id) else { return };
        let comp = e.component.clone();
        let pre = e.pre_focus.clone();
        let is_focused = self.focused.as_ref().is_some_and(|f| same(f, &comp));
        let has_pending = self.focus_restore.overlay() == Some(id);
        if !is_focused && !has_pending {
            return;
        }
        if let FocusRestore::Blocked { overlay, blocked_by, .. } = self.focus_restore.clone() {
            if overlay == id && same_opt(&self.focused, &Some(blocked_by.clone())) {
                self.focus_restore = match target {
                    Some(t) => FocusRestore::Blocked { overlay: id, blocked_by, resume: Resume::FocusTarget(t) },
                    None => FocusRestore::Inactive,
                };
                self.request_render(false);
                return;
            }
        }
        self.clear_restore_for(id);
        if is_focused || target.is_some() {
            let top = self.topmost_visible().filter(|t| *t != id).and_then(|t| self.entry(t)).map(|t| t.component.clone());
            let fallback = top.or(pre);
            self.set_focus(match target {
                Some(t) => t,
                None => fallback,
            });
        }
        self.request_render(false);
    }

    pub fn is_overlay_focused(&self, id: OverlayId) -> bool {
        self.entry(id).is_some_and(|e| self.focused.as_ref().is_some_and(|f| same(f, &e.component)))
    }

    /// Most recent rendered bounds of a visible overlay.
    pub fn overlay_bounds(&self, id: OverlayId) -> Option<OverlayBounds> {
        let e = self.entry(id)?;
        if !self.is_visible(e) {
            return None;
        }
        e.bounds
    }

    pub fn invalidate(&mut self) {
        self.root.invalidate();
        for o in &self.overlays {
            o.component.borrow_mut().invalidate();
        }
    }

    /// Route one input sequence: listeners, debug key, overlay focus repair, then the focused
    /// component. Returns whether a frame should be drawn now.
    pub fn handle_terminal_input(&mut self, data: &str) -> bool {
        let mut data = data.to_string();
        if !self.listeners.is_empty() {
            let mut current = data.clone();
            for (_, l) in self.listeners.iter_mut() {
                if let Some(r) = l(&current) {
                    if r.consume {
                        return false;
                    }
                    if let Some(d) = r.data {
                        current = d;
                    }
                }
            }
            if current.is_empty() {
                return false;
            }
            data = current;
        }
        if matches_key(&data, "shift+ctrl+d") {
            if let Some(cb) = self.on_debug.as_mut() {
                cb();
                return false;
            }
        }
        if let Some(f) = self.focused.clone() {
            if let Some(e) = self.entry_of(&f) {
                if !self.is_visible(e) {
                    let pre = e.pre_focus.clone();
                    match self.topmost_visible().and_then(|t| self.entry(t)).map(|t| t.component.clone()) {
                        Some(t) => self.set_focus(Some(t)),
                        None => self.set_focus_internal(pre, false),
                    }
                }
            }
        }
        let focus_is_overlay = self.focused.as_ref().is_some_and(|f| self.entry_of(f).is_some());
        if !focus_is_overlay {
            match self.visible_restore() {
                FocusRestore::Eligible(o) => {
                    let c = self.entry(o).map(|e| e.component.clone());
                    self.set_focus(c);
                }
                FocusRestore::Blocked { overlay, blocked_by, resume } if !same_opt(&Some(blocked_by.clone()), &self.focused) => {
                    match resume {
                        Resume::RestoreOverlay => {
                            let c = self.entry(overlay).map(|e| e.component.clone());
                            self.set_focus(c);
                        }
                        Resume::FocusTarget(t) => {
                            self.focus_restore = FocusRestore::Inactive;
                            self.set_focus(t);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(f) = self.focused.clone() {
            if f.borrow().accepts_input() {
                if is_key_release(&data) && !f.borrow().wants_key_release() {
                    return false;
                }
                f.borrow_mut().handle_input(&data);
                self.render_requested = true;
                self.immediate = true;
                return true;
            }
        }
        false
    }

    /// Composite visible overlays into the content lines, highest focus order on top.
    pub(crate) fn composite_overlays(&mut self, lines: Vec<String>, tw: usize, th: usize) -> Vec<String> {
        if self.overlays.is_empty() {
            return lines;
        }
        self.term_size = (tw, th);
        let mut result = lines;
        for e in &mut self.overlays {
            e.bounds = None;
        }
        let mut visible: Vec<usize> = (0..self.overlays.len()).filter(|&i| self.is_visible(&self.overlays[i])).collect();
        visible.sort_by_key(|&i| self.overlays[i].focus_order);
        let mut rendered = Vec::new();
        let mut min_lines = result.len() as i64;
        let show_hw = self.show_hardware_cursor;
        for i in visible {
            let opts = self.overlays[i].options.clone().unwrap_or_default();
            let l0 = resolve_overlay_layout(&opts, 0, tw, th);
            let raw = self.overlays[i].component.borrow_mut().render(l0.width as usize);
            let mut ol = resolve_fake_cursors(raw, show_hw);
            if let Some(mh) = l0.max_height {
                ol.truncate(mh as usize);
            }
            let l = resolve_overlay_layout(&opts, ol.len() as i64, tw, th);
            self.overlays[i].bounds = Some(OverlayBounds { row: l.row, col: l.col, width: l0.width, height: ol.len() as i64 });
            min_lines = min_lines.max(l.row + ol.len() as i64);
            rendered.push((ol, l.row, l.col, l0.width));
        }
        let working = (result.len() as i64).max(th as i64).max(min_lines);
        while (result.len() as i64) < working {
            result.push(String::new());
        }
        let viewport_start = (working - th as i64).max(0);
        for (ol, row, col, w) in rendered {
            for (i, line) in ol.iter().enumerate() {
                let idx = viewport_start + row + i as i64;
                if idx >= 0 && (idx as usize) < result.len() {
                    let w = w as usize;
                    let t = if visible_width(line) > w { slice_by_column(line, 0, w, true) } else { line.clone() };
                    let idx = idx as usize;
                    result[idx] = composite_tui_line(&result[idx], &t, col.max(0) as usize, w, tw);
                }
            }
        }
        result
    }
}

/// Turn fake cursor markers into reverse video; with the hardware cursor shown, drop the fake
/// cursor right after the focused CURSOR_MARKER.
pub(crate) fn resolve_fake_cursors(lines: Vec<String>, show_hardware_cursor: bool) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| {
            if !line.contains(FAKE_CURSOR_START) {
                return line;
            }
            let mut resolved = line.clone();
            if show_hardware_cursor {
                let pat = format!("{CURSOR_MARKER}{FAKE_CURSOR_START}");
                if let Some(at) = line.find(&pat) {
                    let body_start = at + pat.len();
                    let body_end = line[body_start..].find(FAKE_CURSOR_END).map(|j| (body_start + j, FAKE_CURSOR_END.len()));
                    let (end, skip) = body_end.unwrap_or((line.len(), 0));
                    resolved = format!("{}{CURSOR_MARKER}{}{}", &line[..at], &line[body_start..end], &line[end + skip..]);
                }
            }
            resolved.replace(FAKE_CURSOR_START, "\x1b[7m").replace(FAKE_CURSOR_END, "\x1b[27m")
        })
        .collect()
}

/// Normalize each non-image line and append the segment reset.
pub(crate) fn apply_line_resets(lines: &mut [String]) {
    for l in lines.iter_mut() {
        if !is_image_line(l) {
            *l = normalize_terminal_output(l) + SEGMENT_RESET;
        }
    }
}

/// Find CURSOR_MARKER in the bottom `height` lines, strip it, and return its (row, col).
pub(crate) fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<(usize, usize)> {
    let top = lines.len().saturating_sub(height);
    for row in (top..lines.len()).rev() {
        if let Some(i) = lines[row].find(CURSOR_MARKER) {
            let col = visible_width(&lines[row][..i]);
            lines[row].replace_range(i..i + CURSOR_MARKER.len(), "");
            return Some((row, col));
        }
    }
    None
}
