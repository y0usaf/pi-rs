use std::rc::Rc;

use crate::layout::{LayoutNode, LayoutViewport};
use crate::tui::{Component, ComponentRef, same};

#[derive(Clone)]
pub struct StackEntry {
    pub component: ComponentRef,
    pub basis: Option<usize>,
    pub grow: usize,
    pub shrink: usize,
    pub min_size: usize,
    pub max_size: usize,
    pub visible: Option<Rc<dyn Fn(LayoutViewport) -> bool>>,
}

impl StackEntry {
    pub fn new(component: ComponentRef) -> StackEntry {
        StackEntry {
            component,
            basis: None,
            grow: 0,
            shrink: 1,
            min_size: 0,
            max_size: usize::MAX,
            visible: None,
        }
    }
    pub fn basis(mut self, basis: usize) -> StackEntry {
        self.basis = Some(basis);
        self
    }
    pub fn grow(mut self, grow: usize) -> StackEntry {
        self.grow = grow;
        self
    }
    pub fn shrink(mut self, shrink: usize) -> StackEntry {
        self.shrink = shrink;
        self
    }
    pub fn min_size(mut self, min_size: usize) -> StackEntry {
        self.min_size = min_size;
        self
    }
    pub fn max_size(mut self, max_size: usize) -> StackEntry {
        self.max_size = max_size;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackKind {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

pub struct Stack {
    kind: StackKind,
    entries: Vec<StackEntry>,
    gap: usize,
    align: Align,
}

impl Stack {
    pub fn new(kind: StackKind, entries: Vec<StackEntry>, gap: usize, align: Align) -> Stack {
        Stack {
            kind,
            entries,
            gap,
            align,
        }
    }
    pub fn vertical(entries: Vec<StackEntry>) -> Stack {
        Stack::new(StackKind::Vertical, entries, 0, Align::Stretch)
    }
    pub fn add(&mut self, entry: StackEntry) {
        self.entries.push(entry);
    }
    pub fn remove(&mut self, component: &ComponentRef) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| same(&entry.component, component))
        {
            self.entries.remove(index);
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Component for Stack {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in &self.entries {
            lines.extend(entry.component.borrow_mut().render(width));
        }
        lines
    }
    fn invalidate(&mut self) {
        for entry in &self.entries {
            entry.component.borrow_mut().invalidate();
        }
    }
    fn children(&self) -> Option<Vec<ComponentRef>> {
        Some(
            self.entries
                .iter()
                .map(|entry| entry.component.clone())
                .collect(),
        )
    }
    fn layout_node(&self) -> Option<LayoutNode> {
        Some(LayoutNode::Stack {
            kind: self.kind,
            entries: self.entries.clone(),
            gap: self.gap,
            align: self.align,
        })
    }
}

pub fn visible_entries(entries: &[StackEntry], viewport: LayoutViewport) -> Vec<StackEntry> {
    entries
        .iter()
        .filter(|entry| {
            entry
                .visible
                .as_ref()
                .is_none_or(|visible| visible(viewport))
        })
        .cloned()
        .collect()
}

fn clamp_size(size: usize, entry: &StackEntry) -> usize {
    let min = entry.min_size;
    let max = entry.max_size.max(min);
    size.clamp(min, max)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Grow,
    Shrink,
}

fn distribute(sizes: &mut [usize], entries: &[StackEntry], amount: usize, mode: Mode) {
    let mut remaining = amount;
    while remaining > 0 {
        let candidates: Vec<usize> = (0..entries.len())
            .filter(|&index| {
                let entry = &entries[index];
                match mode {
                    Mode::Grow => entry.grow > 0 && sizes[index] < entry.max_size,
                    Mode::Shrink => entry.shrink > 0 && sizes[index] > entry.min_size,
                }
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let weights: Vec<usize> = candidates
            .iter()
            .map(|&index| match mode {
                Mode::Grow => entries[index].grow,
                Mode::Shrink => entries[index].shrink * sizes[index].max(1),
            })
            .collect();
        let total: usize = weights.iter().sum();
        let mut distributed = 0;
        for (&index, weight) in candidates.iter().zip(&weights) {
            if remaining == 0 {
                break;
            }
            let proposed = (remaining * weight / total).max(1);
            let capacity = match mode {
                Mode::Grow => entries[index].max_size - sizes[index],
                Mode::Shrink => sizes[index] - entries[index].min_size,
            };
            let delta = remaining.min(proposed).min(capacity);
            if delta == 0 {
                continue;
            }
            match mode {
                Mode::Grow => sizes[index] += delta,
                Mode::Shrink => sizes[index] -= delta,
            }
            remaining -= delta;
            distributed += delta;
        }
        if distributed == 0 {
            return;
        }
    }
}

pub fn allocate_sizes(
    entries: &[StackEntry],
    intrinsic: &[usize],
    available: Option<usize>,
    gap: usize,
) -> Vec<usize> {
    let mut sizes: Vec<usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            clamp_size(
                entry
                    .basis
                    .unwrap_or_else(|| intrinsic.get(index).copied().unwrap_or(0)),
                entry,
            )
        })
        .collect();
    let Some(available) = available else {
        return sizes;
    };
    let content = available.saturating_sub(entries.len().saturating_sub(1) * gap);
    let total: usize = sizes.iter().sum();
    if total < content {
        distribute(&mut sizes, entries, content - total, Mode::Grow);
    } else if total > content {
        distribute(&mut sizes, entries, total - content, Mode::Shrink);
    }
    sizes
}
