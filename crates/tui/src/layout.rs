use std::collections::HashMap;
use std::rc::Rc;

use crate::components::scroll_view::{ScrollRef, Scrollbar};
use crate::components::stack::{Align, StackEntry, StackKind, allocate_sizes, visible_entries};
use crate::tui::{CURSOR_MARKER, ComponentRef, composite_tui_line, is_image_line};
use crate::utils::{
    active_background_ansi, extract_ansi_code, grapheme_cell_range, slice_by_column, visible_width,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutViewport {
    pub width: usize,
    pub height: usize,
}

pub enum LayoutNode {
    Stack {
        kind: StackKind,
        entries: Vec<StackEntry>,
        gap: usize,
        align: Align,
    },
    Scroll {
        child: ComponentRef,
        state: ScrollRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutRect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl LayoutRect {
    pub fn contains(&self, x: i64, y: i64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

pub struct LayoutBox {
    pub component: ComponentRef,
    pub rect: LayoutRect,
    pub clip: LayoutRect,
    pub children: Vec<LayoutBox>,
    pub lines: Option<Rc<Vec<String>>>,
    pub line_offset: usize,
    pub scroll: Option<ScrollRef>,
    pub scroll_content_lines: Option<Rc<Vec<String>>>,
}

pub struct LayoutFrame {
    pub root: LayoutBox,
    pub width: usize,
    pub height: usize,
    pub lines: Vec<String>,
    pub primary: Option<ScrollRef>,
    pub scrolls: Vec<ScrollRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarGeometry {
    pub column: i64,
    pub track_top: i64,
    pub track_height: i64,
    pub thumb_top: i64,
    pub thumb_height: i64,
    pub max_scroll_top: i64,
}

struct Context {
    viewport: LayoutViewport,
    cache: HashMap<(usize, usize), Rc<Vec<String>>>,
    primary: Option<ScrollRef>,
    scrolls: Vec<ScrollRef>,
}

fn key(component: &ComponentRef) -> usize {
    Rc::as_ptr(component) as *const () as usize
}

fn intersect(a: LayoutRect, b: LayoutRect) -> LayoutRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    LayoutRect {
        x,
        y,
        width: (right - x).max(0),
        height: (bottom - y).max(0),
    }
}

fn render_cached(context: &mut Context, component: &ComponentRef, width: usize) -> Rc<Vec<String>> {
    let width = width.max(1);
    context
        .cache
        .entry((key(component), width))
        .or_insert_with(|| Rc::new(component.borrow_mut().render(width)))
        .clone()
}

fn measure_height(context: &mut Context, component: &ComponentRef, width: usize) -> usize {
    render_cached(context, component, width).len()
}

fn measure_width(context: &mut Context, component: &ComponentRef, width: usize) -> usize {
    render_cached(context, component, width)
        .iter()
        .map(|line| visible_width(line))
        .max()
        .unwrap_or(0)
}

fn translate(layout: &mut LayoutBox, delta: i64) {
    layout.rect.y += delta;
    for child in &mut layout.children {
        translate(child, delta);
    }
}

fn update_clips(layout: &mut LayoutBox, parent: LayoutRect) {
    layout.clip = intersect(parent, layout.rect);
    let clip = layout.clip;
    for child in &mut layout.children {
        update_clips(child, clip);
    }
}

fn leaf(component: &ComponentRef, rect: LayoutRect, clip: LayoutRect) -> LayoutBox {
    LayoutBox {
        component: component.clone(),
        rect,
        clip: intersect(clip, rect),
        children: Vec::new(),
        lines: None,
        line_offset: 0,
        scroll: None,
        scroll_content_lines: None,
    }
}

fn layout_component(
    context: &mut Context,
    component: &ComponentRef,
    x: i64,
    y: i64,
    width: usize,
    height: Option<usize>,
    clip: LayoutRect,
) -> LayoutBox {
    let width = width.max(1);
    let node = component.borrow().layout_node();
    match node {
        None => {
            let lines = render_cached(context, component, width);
            let allocated = height.unwrap_or(lines.len());
            let mut line_offset = 0;
            if lines.len() > allocated
                && allocated > 0
                && let Some(cursor) = lines.iter().position(|line| line.contains(CURSOR_MARKER))
                && cursor >= allocated
            {
                line_offset = cursor - allocated + 1;
            }
            let rect = LayoutRect {
                x,
                y,
                width: width as i64,
                height: allocated as i64,
            };
            LayoutBox {
                lines: Some(lines),
                line_offset,
                ..leaf(component, rect, clip)
            }
        }
        Some(LayoutNode::Scroll { child, state }) => {
            let previous = state.borrow().scroll_top() as i64;
            let content_width = state.borrow().content_width(width);
            let mut child_box =
                layout_component(context, &child, x, y - previous, content_width, None, clip);
            let content_height = child_box.rect.height.max(0) as usize;
            let viewport_height = height.unwrap_or(content_height);
            state
                .borrow_mut()
                .update_layout(content_height, viewport_height);
            translate(
                &mut child_box,
                previous - state.borrow().scroll_top() as i64,
            );
            if state.borrow().primary || context.primary.is_none() {
                context.primary = Some(state.clone());
            }
            context.scrolls.push(state.clone());
            let rect = LayoutRect {
                x,
                y,
                width: width as i64,
                height: viewport_height as i64,
            };
            let child_clip = intersect(clip, rect);
            update_clips(&mut child_box, child_clip);
            let content = render_cached(context, &child, content_width);
            LayoutBox {
                children: vec![child_box],
                scroll: Some(state),
                scroll_content_lines: Some(content),
                ..leaf(component, rect, clip)
            }
        }
        Some(LayoutNode::Stack {
            kind,
            entries,
            gap,
            align,
        }) => {
            let entries = visible_entries(&entries, context.viewport);
            let gap_total = entries.len().saturating_sub(1) * gap;
            match kind {
                StackKind::Vertical => {
                    let intrinsic: Vec<usize> = entries
                        .iter()
                        .map(|entry| {
                            entry
                                .basis
                                .unwrap_or_else(|| measure_height(context, &entry.component, width))
                        })
                        .collect();
                    let sizes = allocate_sizes(&entries, &intrinsic, height, gap);
                    let natural = sizes.iter().sum::<usize>() + gap_total;
                    let rect = LayoutRect {
                        x,
                        y,
                        width: width as i64,
                        height: height.unwrap_or(natural) as i64,
                    };
                    let mut layout = leaf(component, rect, clip);
                    let mut child_y = y;
                    for (entry, size) in entries.iter().zip(&sizes) {
                        let child = layout_component(
                            context,
                            &entry.component,
                            x,
                            child_y,
                            width,
                            Some(*size),
                            layout.clip,
                        );
                        layout.children.push(child);
                        child_y += (*size + gap) as i64;
                    }
                    layout
                }
                StackKind::Horizontal => {
                    let intrinsic: Vec<usize> = entries
                        .iter()
                        .map(|entry| {
                            entry
                                .basis
                                .unwrap_or_else(|| measure_width(context, &entry.component, width))
                        })
                        .collect();
                    let widths = allocate_sizes(&entries, &intrinsic, Some(width), gap);
                    let heights: Vec<usize> = entries
                        .iter()
                        .zip(&widths)
                        .map(|(entry, w)| measure_height(context, &entry.component, (*w).max(1)))
                        .collect();
                    let allocated =
                        height.unwrap_or_else(|| heights.iter().copied().max().unwrap_or(0));
                    let rect = LayoutRect {
                        x,
                        y,
                        width: width as i64,
                        height: allocated as i64,
                    };
                    let mut layout = leaf(component, rect, clip);
                    let mut child_x = x;
                    for ((entry, child_width), natural) in entries.iter().zip(&widths).zip(&heights)
                    {
                        let child_height = if align == Align::Stretch {
                            allocated
                        } else {
                            allocated.min(*natural)
                        };
                        let child_y = y + match align {
                            Align::Center => ((allocated - child_height) / 2) as i64,
                            Align::End => (allocated - child_height) as i64,
                            _ => 0,
                        };
                        if *child_width == 0 {
                            layout.children.push(LayoutBox {
                                clip: LayoutRect {
                                    x: child_x,
                                    y: child_y,
                                    width: 0,
                                    height: 0,
                                },
                                ..leaf(
                                    &entry.component,
                                    LayoutRect {
                                        x: child_x,
                                        y: child_y,
                                        width: 0,
                                        height: child_height as i64,
                                    },
                                    layout.clip,
                                )
                            });
                        } else {
                            let child = layout_component(
                                context,
                                &entry.component,
                                child_x,
                                child_y,
                                *child_width,
                                Some(child_height),
                                layout.clip,
                            );
                            layout.children.push(child);
                        }
                        child_x += (*child_width + gap) as i64;
                    }
                    layout
                }
            }
        }
    }
}

pub fn strip_prompt_zones(line: &str) -> &str {
    let mut rest = line;
    loop {
        let Some(after) = rest.strip_prefix("\x1b]133;") else {
            return rest;
        };
        let mut chars = after.chars();
        if !matches!(chars.next(), Some('A' | 'B' | 'C')) {
            return rest;
        }
        let tail = chars.as_str();
        if let Some(next) = tail.strip_prefix('\x07') {
            rest = next;
        } else if let Some(next) = tail.strip_prefix("\x1b\\") {
            rest = next;
        } else {
            return rest;
        }
    }
}

fn replace_scrollbar_cell(
    line: &str,
    column: usize,
    total_width: usize,
    replacement: &str,
    preserve_background: bool,
) -> String {
    if is_image_line(line) {
        return line.to_owned();
    }
    let (start, end) = grapheme_cell_range(line, column).unwrap_or((column, column + 1));
    let before = slice_by_column(line, 0, start, true);
    let target = slice_by_column(line, start, end - start, true);
    let after = slice_by_column(line, end, total_width.saturating_sub(end), true);
    let mut prefix = String::new();
    let mut index = 0;
    while let Some(code) = extract_ansi_code(&target, index) {
        prefix.push_str(code);
        index += code.len();
    }
    let before_padding = " ".repeat(start.saturating_sub(visible_width(&before)));
    let cell_before = " ".repeat(column.saturating_sub(start));
    let cell_after = " ".repeat((end - column).saturating_sub(1));
    let background = if preserve_background {
        active_background_ansi(&prefix)
    } else {
        String::new()
    };
    format!(
        "{before}{before_padding}\x1b[0m\x1b]8;;\x07{background}{cell_before}{replacement}{cell_after}{after}"
    )
}

pub fn scrollbar_geometry(
    layout: &LayoutBox,
    include_hidden_auto: bool,
) -> Option<ScrollbarGeometry> {
    let scroll = layout.scroll.as_ref()?;
    if layout.rect.width <= 0 || layout.rect.height <= 0 {
        return None;
    }
    let state = scroll.borrow();
    let content_height = layout
        .children
        .first()
        .map(|child| child.rect.height)
        .or_else(|| {
            layout
                .scroll_content_lines
                .as_ref()
                .map(|lines| lines.len() as i64)
        })
        .unwrap_or(0);
    let track_height = layout.rect.height;
    let reveal = include_hidden_auto
        && state.scrollbar() == Scrollbar::Auto
        && content_height > track_height;
    if !state.is_scrollbar_visible() && !reveal {
        return None;
    }
    let min_thumb = track_height.min(2);
    let thumb_height =
        min_thumb
            .max(track_height.min(
                ((track_height * track_height) as f64 / content_height as f64).round() as i64,
            ));
    let max_scroll_top = (content_height - track_height).max(0);
    let max_thumb_top = track_height - thumb_height;
    let thumb_offset = if max_scroll_top == 0 {
        0
    } else {
        ((state.scroll_top() as f64 / max_scroll_top as f64) * max_thumb_top as f64).round() as i64
    };
    let column = layout.rect.x + layout.rect.width - 1;
    if column < layout.clip.x || column >= layout.clip.x + layout.clip.width {
        return None;
    }
    Some(ScrollbarGeometry {
        column,
        track_top: layout.rect.y,
        track_height,
        thumb_top: layout.rect.y + thumb_offset,
        thumb_height,
        max_scroll_top,
    })
}

fn paint_scrollbar(layout: &LayoutBox, screen: &mut [String], total_width: usize) {
    let Some(geometry) = scrollbar_geometry(layout, false) else {
        return;
    };
    let Some(scroll) = &layout.scroll else {
        return;
    };
    let state = scroll.borrow();
    for offset in 0..geometry.track_height {
        let row = geometry.track_top + offset;
        if row < layout.clip.y
            || row >= layout.clip.y + layout.clip.height
            || row < 0
            || row >= screen.len() as i64
        {
            continue;
        }
        let thumb = row >= geometry.thumb_top && row < geometry.thumb_top + geometry.thumb_height;
        let replacement = if thumb {
            (state.thumb_style)(if state.is_scrollbar_active() {
                "█"
            } else {
                "┃"
            })
        } else {
            (state.track_style)("│")
        };
        let row = row as usize;
        screen[row] = replace_scrollbar_cell(
            &screen[row],
            geometry.column as usize,
            total_width,
            &replacement,
            state.scrollbar() != Scrollbar::Always,
        );
    }
}

fn paint(layout: &LayoutBox, screen: &mut [String], total_width: usize) {
    if let Some(lines) = &layout.lines {
        let first = layout.rect.y.max(layout.clip.y).max(0);
        let last = (layout.rect.y + layout.rect.height)
            .min(layout.clip.y + layout.clip.height)
            .min(screen.len() as i64);
        for row in first..last {
            let Some(source) = lines.get(layout.line_offset + (row - layout.rect.y) as usize)
            else {
                continue;
            };
            let line = strip_prompt_zones(source);
            let row = row as usize;
            if layout.rect.x == 0
                && layout.rect.width >= total_width as i64
                && (is_image_line(line) || screen[row].is_empty())
            {
                screen[row] = line.to_owned();
            } else {
                screen[row] = composite_tui_line(
                    &screen[row],
                    line,
                    layout.rect.x.max(0) as usize,
                    layout.rect.width.max(0) as usize,
                    total_width,
                );
            }
        }
    }
    for child in &layout.children {
        paint(child, screen, total_width);
    }
    paint_scrollbar(layout, screen, total_width);
}

pub fn render_layout_frame(root: &ComponentRef, width: usize, height: usize) -> LayoutFrame {
    let width = width.max(1);
    let height = height.max(1);
    let mut context = Context {
        viewport: LayoutViewport { width, height },
        cache: HashMap::new(),
        primary: None,
        scrolls: Vec::new(),
    };
    let bounds = LayoutRect {
        x: 0,
        y: 0,
        width: width as i64,
        height: height as i64,
    };
    let root = layout_component(&mut context, root, 0, 0, width, Some(height), bounds);
    let mut lines = vec![String::new(); height];
    paint(&root, &mut lines, width);
    LayoutFrame {
        root,
        width,
        height,
        lines,
        primary: context.primary,
        scrolls: context.scrolls,
    }
}

pub fn layout_boxes_at(frame: &LayoutFrame, x: i64, y: i64) -> Vec<&LayoutBox> {
    fn visit<'a>(
        layout: &'a LayoutBox,
        depth: usize,
        x: i64,
        y: i64,
        out: &mut Vec<(&'a LayoutBox, usize)>,
    ) {
        if !layout.clip.contains(x, y) {
            return;
        }
        out.push((layout, depth));
        for child in &layout.children {
            visit(child, depth + 1, x, y, out);
        }
    }
    let mut found = Vec::new();
    visit(&frame.root, 0, x, y, &mut found);
    found.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    found.into_iter().map(|(layout, _)| layout).collect()
}

pub fn scroll_view_box<'a>(frame: &'a LayoutFrame, scroll: &ScrollRef) -> Option<&'a LayoutBox> {
    fn visit<'a>(layout: &'a LayoutBox, scroll: &ScrollRef) -> Option<&'a LayoutBox> {
        if layout
            .scroll
            .as_ref()
            .is_some_and(|own| Rc::ptr_eq(own, scroll))
        {
            return Some(layout);
        }
        layout
            .children
            .iter()
            .find_map(|child| visit(child, scroll))
    }
    visit(&frame.root, scroll)
}

pub fn scroll_views_at(frame: &LayoutFrame, x: i64, y: i64) -> Vec<ScrollRef> {
    fn visit(layout: &LayoutBox, depth: usize, x: i64, y: i64, out: &mut Vec<(ScrollRef, usize)>) {
        if !layout.clip.contains(x, y) {
            return;
        }
        if let Some(scroll) = &layout.scroll
            && layout.rect.contains(x, y)
        {
            out.push((scroll.clone(), depth));
        }
        for child in &layout.children {
            visit(child, depth + 1, x, y, out);
        }
    }
    let mut found = Vec::new();
    visit(&frame.root, 0, x, y, &mut found);
    found.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    found.into_iter().map(|(scroll, _)| scroll).collect()
}
