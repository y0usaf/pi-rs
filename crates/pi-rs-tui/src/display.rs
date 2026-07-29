//! Versioned, bounded retained display trees and differential ANSI presentation.
//!
//! A caller submits one flat batch per frame. Nodes carry stable identities and
//! refer to children by ID, so decoding never requires recursive input values.
//! Validation, iterative layout, clipping, rasterization, and presentation all
//! happen inside one host call; a rejected batch leaves retained state intact.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::utils::grapheme_width;

pub const DISPLAY_SCHEMA_VERSION: u16 = 1;
const SYNC_START: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_children_per_node: usize,
    pub max_text_runs: usize,
    pub max_text_bytes: usize,
    pub max_cells: usize,
}

impl Default for DisplayLimits {
    fn default() -> Self {
        Self {
            max_nodes: 4_096,
            max_depth: 64,
            max_children_per_node: 1_024,
            max_text_runs: 16_384,
            max_text_bytes: 1_048_576,
            max_cells: 262_144,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    pub columns: u16,
    pub rows: u16,
}

impl Viewport {
    fn rect(self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.columns,
            height: self.rows,
        }
    }
}

/// A node-local rectangle. Child coordinates are relative to their parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right()?.min(other.right()?);
        let bottom = self.bottom()?.min(other.bottom()?);
        if left >= right || top >= bottom {
            return None;
        }
        Some(Self {
            x: left,
            y: top,
            width: u16::try_from(right - left).ok()?,
            height: u16::try_from(bottom - top).ok()?,
        })
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        let Some(right) = self.right() else {
            return false;
        };
        let Some(bottom) = self.bottom() else {
            return false;
        };
        x >= self.x && x < right && y >= self.y && y < bottom
    }

    fn right(self) -> Option<i32> {
        self.x.checked_add(i32::from(self.width))
    }

    fn bottom(self) -> Option<i32> {
        self.y.checked_add(i32::from(self.height))
    }

    fn translated(self, parent_x: i32, parent_y: i32) -> Option<Self> {
        Some(Self {
            x: parent_x.checked_add(self.x)?,
            y: parent_y.checked_add(self.y)?,
            ..self
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb {
        red: u8,
        green: u8,
        blue: u8,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellStyle {
    #[serde(default)]
    pub foreground: Color,
    #[serde(default)]
    pub background: Color,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub dim: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub reverse: bool,
}

impl CellStyle {
    fn write_ansi(self, output: &mut String) {
        output.push_str("\x1b[0m");
        let mut codes = Vec::with_capacity(7);
        if self.bold {
            codes.push("1".to_owned());
        }
        if self.dim {
            codes.push("2".to_owned());
        }
        if self.italic {
            codes.push("3".to_owned());
        }
        if self.underline {
            codes.push("4".to_owned());
        }
        if self.reverse {
            codes.push("7".to_owned());
        }
        match self.foreground {
            Color::Default => {}
            Color::Indexed(value) => codes.push(format!("38;5;{value}")),
            Color::Rgb { red, green, blue } => {
                codes.push(format!("38;2;{red};{green};{blue}"));
            }
        }
        match self.background {
            Color::Default => {}
            Color::Indexed(value) => codes.push(format!("48;5;{value}")),
            Color::Rgb { red, green, blue } => {
                codes.push(format!("48;2;{red};{green};{blue}"));
            }
        }
        if !codes.is_empty() {
            output.push_str("\x1b[");
            output.push_str(&codes.join(";"));
            output.push('m');
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    #[serde(default)]
    pub style: CellStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapMode {
    #[default]
    Grapheme,
    Clip,
}

fn default_tab_width() -> u8 {
    4
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayNodeContent {
    Group,
    Text {
        runs: Vec<TextRun>,
        #[serde(default)]
        wrap: WrapMode,
        #[serde(default = "default_tab_width")]
        tab_width: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayNode {
    pub id: NodeId,
    pub rect: Rect,
    #[serde(default)]
    pub clip_children: bool,
    #[serde(default)]
    pub focusable: bool,
    pub content: DisplayNodeContent,
    #[serde(default)]
    pub children: Vec<NodeId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorMetadata {
    pub node: NodeId,
    pub row: u16,
    pub column: u16,
    #[serde(default)]
    pub shape: CursorShape,
    #[serde(default = "default_true")]
    pub visible: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayBatch {
    pub version: u16,
    pub viewport: Viewport,
    pub root: NodeId,
    pub nodes: Vec<DisplayNode>,
    #[serde(default)]
    pub focused: Option<NodeId>,
    #[serde(default)]
    pub cursor: Option<CursorMetadata>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// `None` is a blank cell. A wide grapheme is stored only in its lead cell.
    pub grapheme: Option<Arc<str>>,
    pub style: CellStyle,
    pub continuation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCursor {
    pub node: NodeId,
    pub row: u16,
    pub column: u16,
    pub shape: CursorShape,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameFocus {
    pub node: NodeId,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    viewport: Viewport,
    cells: Vec<Cell>,
    pub cursor: Option<FrameCursor>,
    pub focus: Option<FrameFocus>,
}

impl Frame {
    fn blank(viewport: Viewport) -> Self {
        let count = usize::from(viewport.columns) * usize::from(viewport.rows);
        Self {
            viewport,
            cells: vec![Cell::default(); count],
            cursor: None,
            focus: None,
        }
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn cell(&self, row: u16, column: u16) -> Option<&Cell> {
        self.index(row, column)
            .and_then(|index| self.cells.get(index))
    }

    pub fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        self.cells.chunks(usize::from(self.viewport.columns))
    }

    fn index(&self, row: u16, column: u16) -> Option<usize> {
        if row >= self.viewport.rows || column >= self.viewport.columns {
            return None;
        }
        Some(usize::from(row) * usize::from(self.viewport.columns) + usize::from(column))
    }

    fn clear_footprint(&mut self, row: u16, column: u16) {
        let Some(index) = self.index(row, column) else {
            return;
        };
        if self.cells[index].continuation && column > 0 {
            if let Some(previous) = self.index(row, column - 1) {
                self.cells[previous] = Cell::default();
            }
        } else if column + 1 < self.viewport.columns {
            if let Some(next) = self.index(row, column + 1) {
                if self.cells[next].continuation {
                    self.cells[next] = Cell::default();
                }
            }
        }
        self.cells[index] = Cell::default();
    }

    fn paint(&mut self, row: u16, column: u16, grapheme: &str, width: u16, style: CellStyle) {
        for offset in 0..width {
            self.clear_footprint(row, column + offset);
        }
        let Some(lead) = self.index(row, column) else {
            return;
        };
        self.cells[lead] = Cell {
            grapheme: Some(Arc::from(grapheme)),
            style,
            continuation: false,
        };
        for offset in 1..width {
            if let Some(index) = self.index(row, column + offset) {
                self.cells[index] = Cell {
                    grapheme: None,
                    style,
                    continuation: true,
                };
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityDelta {
    pub added: Vec<NodeId>,
    pub changed: Vec<NodeId>,
    pub retained: Vec<NodeId>,
    pub removed: Vec<NodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitResult {
    pub revision: u64,
    pub ansi: String,
    pub identities: IdentityDelta,
    pub visited_nodes: usize,
    pub painted_cells: usize,
    pub changed_cells: usize,
    pub full_redraw: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DisplayError {
    #[error("unsupported display schema version {actual}; expected {expected}")]
    Version { expected: u16, actual: u16 },
    #[error("viewport dimensions must be non-zero")]
    EmptyViewport,
    #[error("viewport has {actual} cells; limit is {limit}")]
    CellLimit { actual: usize, limit: usize },
    #[error("display batch has {actual} nodes; limit is {limit}")]
    NodeLimit { actual: usize, limit: usize },
    #[error("node ID zero is reserved")]
    ZeroNodeId,
    #[error("duplicate node ID {0:?}")]
    DuplicateNode(NodeId),
    #[error("root node {0:?} is missing")]
    MissingRoot(NodeId),
    #[error("root rectangle must exactly match the viewport")]
    RootLayout,
    #[error("node {0:?} has an empty or overflowing rectangle")]
    InvalidRect(NodeId),
    #[error("node {node:?} has {actual} children; limit is {limit}")]
    ChildLimit {
        node: NodeId,
        actual: usize,
        limit: usize,
    },
    #[error("node {parent:?} references missing child {child:?}")]
    MissingChild { parent: NodeId, child: NodeId },
    #[error("node {0:?} appears more than once in the tree")]
    MultipleParents(NodeId),
    #[error("root node must not be a child")]
    RootHasParent,
    #[error("node {0:?} is not reachable from the root")]
    Unreachable(NodeId),
    #[error("display tree exceeds depth limit {limit}")]
    DepthLimit { limit: usize },
    #[error("display tree coordinate overflow at node {0:?}")]
    LayoutOverflow(NodeId),
    #[error("text run count {actual} exceeds limit {limit}")]
    TextRunLimit { actual: usize, limit: usize },
    #[error("text byte count {actual} exceeds limit {limit}")]
    TextByteLimit { actual: usize, limit: usize },
    #[error("node {0:?} contains terminal control data")]
    TerminalControl(NodeId),
    #[error("node {node:?} has invalid tab width {width}")]
    InvalidTabWidth { node: NodeId, width: u8 },
    #[error("focused node {0:?} is missing or not focusable")]
    InvalidFocus(NodeId),
    #[error("cursor node {0:?} is missing")]
    MissingCursorNode(NodeId),
    #[error("cursor is outside node {0:?}")]
    CursorOutsideNode(NodeId),
    #[error("display revision exhausted")]
    RevisionExhausted,
}

#[derive(Default)]
struct AnsiPresenter {
    previous: Option<Frame>,
}

struct PresentResult {
    ansi: String,
    changed_cells: usize,
    full_redraw: bool,
}

impl AnsiPresenter {
    fn reset(&mut self) {
        self.previous = None;
    }

    fn present(&mut self, frame: &Frame, damage: Option<&[Rect]>) -> PresentResult {
        let full_redraw = self
            .previous
            .as_ref()
            .is_none_or(|previous| previous.viewport != frame.viewport);
        let mut output = String::new();
        let mut changed_cells = 0;
        let columns = usize::from(frame.viewport.columns);

        for row in 0..usize::from(frame.viewport.rows) {
            if !full_redraw
                && damage.is_some_and(|rects| {
                    rects.iter().all(|rect| {
                        let Ok(row) = i32::try_from(row) else {
                            return true;
                        };
                        row < rect.y || row >= rect.y + i32::from(rect.height)
                    })
                })
            {
                continue;
            }
            let current = &frame.cells[row * columns..(row + 1) * columns];
            let previous = self.previous.as_ref().and_then(|previous| {
                (!full_redraw).then(|| &previous.cells[row * columns..(row + 1) * columns])
            });
            let span = if full_redraw {
                changed_cells += columns;
                (columns > 0).then_some((0, columns - 1))
            } else {
                changed_span(previous.unwrap_or(&[]), current, &mut changed_cells)
            };
            let Some((start, end)) = span else {
                continue;
            };
            if output.is_empty() {
                output.push_str(SYNC_START);
                if full_redraw {
                    output.push_str("\x1b[2J");
                }
            }
            output.push_str(&format!("\x1b[{};{}H", row + 1, start + 1));
            write_cells(&mut output, current, start, end);
            output.push_str("\x1b[0m");
        }

        let cursor_changed = self
            .previous
            .as_ref()
            .is_none_or(|previous| previous.cursor != frame.cursor);
        // Cell writes move the hardware cursor, so restore unchanged cursor
        // metadata after every non-empty differential update as well.
        if cursor_changed || full_redraw || !output.is_empty() {
            if output.is_empty() {
                output.push_str(SYNC_START);
            }
            match frame.cursor.filter(|cursor| cursor.visible) {
                Some(cursor) => {
                    let shape = match cursor.shape {
                        CursorShape::Block => 2,
                        CursorShape::Bar => 6,
                        CursorShape::Underline => 4,
                    };
                    output.push_str(&format!(
                        "\x1b[{};{}H\x1b[{shape} q\x1b[?25h",
                        usize::from(cursor.row) + 1,
                        usize::from(cursor.column) + 1
                    ));
                }
                None => output.push_str("\x1b[?25l"),
            }
        }
        if !output.is_empty() {
            output.push_str(SYNC_END);
        }
        self.previous = Some(frame.clone());
        PresentResult {
            ansi: output,
            changed_cells,
            full_redraw,
        }
    }
}

fn changed_span(
    previous: &[Cell],
    current: &[Cell],
    changed_cells: &mut usize,
) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = 0;
    for (column, (old, new)) in previous.iter().zip(current).enumerate() {
        if old != new {
            *changed_cells += 1;
            first.get_or_insert(column);
            last = column;
        }
    }
    let mut first = first?;
    if current[first].continuation || previous[first].continuation {
        first = first.saturating_sub(1);
    }
    if last + 1 < current.len()
        && (current[last + 1].continuation || previous[last + 1].continuation)
    {
        last += 1;
    }
    Some((first, last))
}

fn write_cells(output: &mut String, cells: &[Cell], start: usize, end: usize) {
    let mut active_style = None;
    let mut column = start;
    while column <= end {
        let cell = &cells[column];
        if cell.continuation {
            column += 1;
            continue;
        }
        if active_style != Some(cell.style) {
            cell.style.write_ansi(output);
            active_style = Some(cell.style);
        }
        output.push_str(cell.grapheme.as_deref().unwrap_or(" "));
        if column + 1 < cells.len() && cells[column + 1].continuation {
            column += 2;
        } else {
            column += 1;
        }
    }
}

pub struct RetainedDisplay {
    limits: DisplayLimits,
    revision: u64,
    nodes: BTreeMap<NodeId, DisplayNode>,
    root: Option<NodeId>,
    focused: Option<NodeId>,
    cursor: Option<CursorMetadata>,
    frame: Option<Frame>,
    presenter: AnsiPresenter,
}

impl Default for RetainedDisplay {
    fn default() -> Self {
        Self::new(DisplayLimits::default())
    }
}

impl RetainedDisplay {
    pub fn new(limits: DisplayLimits) -> Self {
        Self {
            limits,
            revision: 0,
            nodes: BTreeMap::new(),
            root: None,
            focused: None,
            cursor: None,
            frame: None,
            presenter: AnsiPresenter::default(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
    }

    /// Forget presentation state while retaining the accepted tree and frame.
    /// The next submission is a full redraw, for example after another process
    /// temporarily owned the terminal.
    pub fn reset_presentation(&mut self) {
        self.presenter.reset();
    }

    /// Validate and submit one complete retained-tree batch transactionally.
    pub fn submit(&mut self, batch: DisplayBatch) -> Result<SubmitResult, DisplayError> {
        let validated = validate_batch(&batch, self.limits)?;
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(DisplayError::RevisionExhausted)?;
        let next_nodes: BTreeMap<_, _> = batch
            .nodes
            .iter()
            .cloned()
            .map(|node| (node.id, node))
            .collect();
        let identities = identity_delta(&self.nodes, &next_nodes);
        let metadata_unchanged = self.root == Some(batch.root)
            && self.focused == batch.focused
            && self.cursor == batch.cursor
            && self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.viewport == batch.viewport);
        if metadata_unchanged && self.nodes == next_nodes && self.presenter.previous.is_some() {
            self.revision = next_revision;
            return Ok(SubmitResult {
                revision: next_revision,
                ansi: String::new(),
                identities,
                visited_nodes: batch.nodes.len(),
                painted_cells: 0,
                changed_cells: 0,
                full_redraw: false,
            });
        }

        let incremental = if self.presenter.previous.is_some() {
            match self
                .frame
                .as_ref()
                .filter(|frame| frame.viewport == batch.viewport)
            {
                Some(frame) => incremental_rasterize(&batch, &validated, &self.nodes, frame)?,
                None => None,
            }
        } else {
            None
        };
        let raster = match incremental {
            Some(raster) => raster,
            None => rasterize(&batch, &validated)?,
        };
        let presented = self
            .presenter
            .present(&raster.frame, raster.damage.as_deref());

        self.nodes = next_nodes;
        self.root = Some(batch.root);
        self.focused = batch.focused;
        self.cursor = batch.cursor;
        self.frame = Some(raster.frame);
        self.revision = next_revision;
        Ok(SubmitResult {
            revision: next_revision,
            ansi: presented.ansi,
            identities,
            visited_nodes: raster.visited_nodes,
            painted_cells: raster.painted_cells,
            changed_cells: presented.changed_cells,
            full_redraw: presented.full_redraw,
        })
    }
}

struct ValidatedBatch {
    index: BTreeMap<NodeId, usize>,
}

fn validate_batch(
    batch: &DisplayBatch,
    limits: DisplayLimits,
) -> Result<ValidatedBatch, DisplayError> {
    if batch.version != DISPLAY_SCHEMA_VERSION {
        return Err(DisplayError::Version {
            expected: DISPLAY_SCHEMA_VERSION,
            actual: batch.version,
        });
    }
    if batch.viewport.columns == 0 || batch.viewport.rows == 0 {
        return Err(DisplayError::EmptyViewport);
    }
    let cells =
        usize::from(batch.viewport.columns).saturating_mul(usize::from(batch.viewport.rows));
    if cells > limits.max_cells {
        return Err(DisplayError::CellLimit {
            actual: cells,
            limit: limits.max_cells,
        });
    }
    if batch.nodes.len() > limits.max_nodes {
        return Err(DisplayError::NodeLimit {
            actual: batch.nodes.len(),
            limit: limits.max_nodes,
        });
    }

    let mut index = BTreeMap::new();
    let mut text_runs = 0usize;
    let mut text_bytes = 0usize;
    for (position, node) in batch.nodes.iter().enumerate() {
        if node.id.0 == 0 {
            return Err(DisplayError::ZeroNodeId);
        }
        if index.insert(node.id, position).is_some() {
            return Err(DisplayError::DuplicateNode(node.id));
        }
        if node.rect.width == 0
            || node.rect.height == 0
            || node.rect.right().is_none()
            || node.rect.bottom().is_none()
        {
            return Err(DisplayError::InvalidRect(node.id));
        }
        if node.children.len() > limits.max_children_per_node {
            return Err(DisplayError::ChildLimit {
                node: node.id,
                actual: node.children.len(),
                limit: limits.max_children_per_node,
            });
        }
        if let DisplayNodeContent::Text {
            runs, tab_width, ..
        } = &node.content
        {
            if !(1..=16).contains(tab_width) {
                return Err(DisplayError::InvalidTabWidth {
                    node: node.id,
                    width: *tab_width,
                });
            }
            text_runs = text_runs.saturating_add(runs.len());
            for run in runs {
                text_bytes = text_bytes.saturating_add(run.text.len());
                if run.text.chars().any(|character| {
                    character == '\x1b'
                        || (character.is_control() && character != '\n' && character != '\t')
                }) {
                    return Err(DisplayError::TerminalControl(node.id));
                }
            }
        }
    }
    if text_runs > limits.max_text_runs {
        return Err(DisplayError::TextRunLimit {
            actual: text_runs,
            limit: limits.max_text_runs,
        });
    }
    if text_bytes > limits.max_text_bytes {
        return Err(DisplayError::TextByteLimit {
            actual: text_bytes,
            limit: limits.max_text_bytes,
        });
    }

    let Some(&root_index) = index.get(&batch.root) else {
        return Err(DisplayError::MissingRoot(batch.root));
    };
    if batch.nodes[root_index].rect != batch.viewport.rect() {
        return Err(DisplayError::RootLayout);
    }

    let mut parents = BTreeMap::<NodeId, NodeId>::new();
    for node in &batch.nodes {
        let mut local = BTreeSet::new();
        for child in &node.children {
            if !index.contains_key(child) {
                return Err(DisplayError::MissingChild {
                    parent: node.id,
                    child: *child,
                });
            }
            if !local.insert(*child) || parents.insert(*child, node.id).is_some() {
                return Err(DisplayError::MultipleParents(*child));
            }
        }
    }
    if parents.contains_key(&batch.root) {
        return Err(DisplayError::RootHasParent);
    }

    let mut stack = vec![(batch.root, 1usize)];
    let mut visited = BTreeSet::new();
    while let Some((id, depth)) = stack.pop() {
        if depth > limits.max_depth {
            return Err(DisplayError::DepthLimit {
                limit: limits.max_depth,
            });
        }
        if !visited.insert(id) {
            return Err(DisplayError::MultipleParents(id));
        }
        let node = &batch.nodes[index[&id]];
        for child in node.children.iter().rev() {
            stack.push((*child, depth + 1));
        }
    }
    if let Some(node) = batch.nodes.iter().find(|node| !visited.contains(&node.id)) {
        return Err(DisplayError::Unreachable(node.id));
    }

    if let Some(focused) = batch.focused {
        let Some(&position) = index.get(&focused) else {
            return Err(DisplayError::InvalidFocus(focused));
        };
        if !batch.nodes[position].focusable {
            return Err(DisplayError::InvalidFocus(focused));
        }
    }
    if let Some(cursor) = batch.cursor {
        let Some(&position) = index.get(&cursor.node) else {
            return Err(DisplayError::MissingCursorNode(cursor.node));
        };
        let node = &batch.nodes[position];
        if cursor.row >= node.rect.height || cursor.column >= node.rect.width {
            return Err(DisplayError::CursorOutsideNode(cursor.node));
        }
    }
    Ok(ValidatedBatch { index })
}

struct Rasterized {
    frame: Frame,
    visited_nodes: usize,
    painted_cells: usize,
    damage: Option<Vec<Rect>>,
}

#[derive(Clone, Copy)]
struct LayoutEntry {
    rect: Rect,
    clip: Option<Rect>,
}

fn layout_entries(
    batch: &DisplayBatch,
    validated: &ValidatedBatch,
) -> Result<BTreeMap<NodeId, LayoutEntry>, DisplayError> {
    let viewport_rect = batch.viewport.rect();
    let mut layouts = BTreeMap::new();
    let mut stack = vec![(batch.root, 0i32, 0i32, Some(viewport_rect))];
    while let Some((id, parent_x, parent_y, inherited_clip)) = stack.pop() {
        let node = &batch.nodes[validated.index[&id]];
        let rect = node
            .rect
            .translated(parent_x, parent_y)
            .ok_or(DisplayError::LayoutOverflow(id))?;
        let content_clip = inherited_clip.and_then(|clip| clip.intersection(rect));
        layouts.insert(
            id,
            LayoutEntry {
                rect,
                clip: content_clip,
            },
        );
        let child_clip = if node.clip_children {
            content_clip
        } else {
            inherited_clip
        };
        for child in node.children.iter().rev() {
            stack.push((*child, rect.x, rect.y, child_clip));
        }
    }
    Ok(layouts)
}

fn apply_metadata(
    frame: &mut Frame,
    batch: &DisplayBatch,
    layouts: &BTreeMap<NodeId, LayoutEntry>,
) -> Result<(), DisplayError> {
    frame.focus = batch.focused.and_then(|focused| {
        layouts[&focused].clip.map(|rect| FrameFocus {
            node: focused,
            rect,
        })
    });
    frame.cursor = None;
    if let Some(cursor) = batch.cursor {
        let layout = &layouts[&cursor.node];
        let column = layout
            .rect
            .x
            .checked_add(i32::from(cursor.column))
            .ok_or(DisplayError::LayoutOverflow(cursor.node))?;
        let row = layout
            .rect
            .y
            .checked_add(i32::from(cursor.row))
            .ok_or(DisplayError::LayoutOverflow(cursor.node))?;
        let visible = cursor.visible
            && layout.clip.is_some_and(|clip| clip.contains(column, row))
            && batch.viewport.rect().contains(column, row);
        if let (Ok(row), Ok(column)) = (u16::try_from(row), u16::try_from(column)) {
            frame.cursor = Some(FrameCursor {
                node: cursor.node,
                row,
                column,
                shape: cursor.shape,
                visible,
            });
        }
    }
    Ok(())
}

fn incremental_rasterize(
    batch: &DisplayBatch,
    validated: &ValidatedBatch,
    previous_nodes: &BTreeMap<NodeId, DisplayNode>,
    previous_frame: &Frame,
) -> Result<Option<Rasterized>, DisplayError> {
    if previous_nodes.len() != batch.nodes.len() {
        return Ok(None);
    }
    let mut changed = Vec::new();
    for node in &batch.nodes {
        let Some(previous) = previous_nodes.get(&node.id) else {
            return Ok(None);
        };
        if previous == node {
            continue;
        }
        let structure_unchanged = previous.rect == node.rect
            && previous.clip_children == node.clip_children
            && previous.focusable == node.focusable
            && previous.children == node.children;
        if !structure_unchanged
            || !previous.children.is_empty()
            || !matches!(previous.content, DisplayNodeContent::Text { .. })
            || !matches!(node.content, DisplayNodeContent::Text { .. })
        {
            return Ok(None);
        }
        changed.push(node.id);
    }

    let layouts = layout_entries(batch, validated)?;
    for id in &changed {
        let Some(dirty) = layouts[id].clip else {
            continue;
        };
        for other in &batch.nodes {
            if other.id == *id || !matches!(other.content, DisplayNodeContent::Text { .. }) {
                continue;
            }
            if layouts[&other.id]
                .clip
                .is_some_and(|clip| clip.intersection(dirty).is_some())
            {
                return Ok(None);
            }
        }
    }

    let mut frame = previous_frame.clone();
    let mut painted_cells = 0;
    let mut damage = Vec::new();
    for id in changed {
        let Some(clip) = layouts[&id].clip else {
            continue;
        };
        damage.push(clip);
        for row in clip.y..clip.y + i32::from(clip.height) {
            for column in clip.x..clip.x + i32::from(clip.width) {
                if let (Ok(row), Ok(column)) = (u16::try_from(row), u16::try_from(column)) {
                    frame.clear_footprint(row, column);
                }
            }
        }
        let node = &batch.nodes[validated.index[&id]];
        if let DisplayNodeContent::Text {
            runs,
            wrap,
            tab_width,
        } = &node.content
        {
            painted_cells +=
                paint_text(&mut frame, layouts[&id].rect, clip, runs, *wrap, *tab_width);
        }
    }
    apply_metadata(&mut frame, batch, &layouts)?;
    Ok(Some(Rasterized {
        frame,
        visited_nodes: batch.nodes.len(),
        painted_cells,
        damage: Some(damage),
    }))
}

fn rasterize(batch: &DisplayBatch, validated: &ValidatedBatch) -> Result<Rasterized, DisplayError> {
    let viewport_rect = batch.viewport.rect();
    let mut frame = Frame::blank(batch.viewport);
    let mut layouts = BTreeMap::new();
    let mut stack = vec![(batch.root, 0i32, 0i32, Some(viewport_rect))];
    let mut visited_nodes = 0;
    let mut painted_cells = 0;

    while let Some((id, parent_x, parent_y, inherited_clip)) = stack.pop() {
        let node = &batch.nodes[validated.index[&id]];
        let rect = node
            .rect
            .translated(parent_x, parent_y)
            .ok_or(DisplayError::LayoutOverflow(id))?;
        let content_clip = inherited_clip.and_then(|clip| clip.intersection(rect));
        layouts.insert(
            id,
            LayoutEntry {
                rect,
                clip: content_clip,
            },
        );
        if let (
            Some(clip),
            DisplayNodeContent::Text {
                runs,
                wrap,
                tab_width,
            },
        ) = (content_clip, &node.content)
        {
            painted_cells += paint_text(&mut frame, rect, clip, runs, *wrap, *tab_width);
        }
        let child_clip = if node.clip_children {
            content_clip
        } else {
            inherited_clip
        };
        for child in node.children.iter().rev() {
            stack.push((*child, rect.x, rect.y, child_clip));
        }
        visited_nodes += 1;
    }

    if let Some(focused) = batch.focused {
        let layout = &layouts[&focused];
        if let Some(rect) = layout.clip {
            frame.focus = Some(FrameFocus {
                node: focused,
                rect,
            });
        }
    }
    if let Some(cursor) = batch.cursor {
        let layout = &layouts[&cursor.node];
        let column = layout
            .rect
            .x
            .checked_add(i32::from(cursor.column))
            .ok_or(DisplayError::LayoutOverflow(cursor.node))?;
        let row = layout
            .rect
            .y
            .checked_add(i32::from(cursor.row))
            .ok_or(DisplayError::LayoutOverflow(cursor.node))?;
        let visible = cursor.visible
            && layout.clip.is_some_and(|clip| clip.contains(column, row))
            && viewport_rect.contains(column, row);
        if let (Ok(row), Ok(column)) = (u16::try_from(row), u16::try_from(column)) {
            frame.cursor = Some(FrameCursor {
                node: cursor.node,
                row,
                column,
                shape: cursor.shape,
                visible,
            });
        }
    }

    Ok(Rasterized {
        frame,
        visited_nodes,
        painted_cells,
        damage: None,
    })
}

/// One grapheme placement produced by the shared text walker.
struct TextPlacement<'a> {
    row: u16,
    column: u16,
    grapheme: &'a str,
    width: u16,
    style: CellStyle,
}

/// Walk a text node's graphemes exactly the way the rasterizer paints them and
/// return the cursor left after the last one.
///
/// Painting and measurement share this traversal, so a caller that measures a
/// string can never disagree with the cells that string later occupies.
fn walk_text<'a, I>(
    segments: I,
    width: u16,
    wrap: WrapMode,
    tab_width: u8,
    visit: &mut dyn FnMut(TextPlacement<'a>),
) -> (u16, u16)
where
    I: IntoIterator<Item = (&'a str, CellStyle)>,
{
    let tab_width = u16::from(tab_width.max(1));
    let mut row = 0u16;
    let mut column = 0u16;
    for (text, style) in segments {
        for grapheme in text.graphemes(true) {
            if grapheme == "\n" {
                row = row.saturating_add(1);
                column = 0;
                continue;
            }
            if grapheme == "\t" {
                let spaces = tab_width - column % tab_width;
                for _ in 0..spaces {
                    visit(TextPlacement {
                        row,
                        column,
                        grapheme: " ",
                        width: 1,
                        style,
                    });
                    column = column.saturating_add(1);
                }
                continue;
            }
            let cells = u16::try_from(grapheme_width(grapheme)).unwrap_or(u16::MAX);
            if cells == 0 {
                continue;
            }
            if column.saturating_add(cells) > width {
                if wrap == WrapMode::Grapheme {
                    row = row.saturating_add(1);
                    column = 0;
                } else {
                    column = column.saturating_add(cells);
                    continue;
                }
            }
            visit(TextPlacement {
                row,
                column,
                grapheme,
                width: cells,
                style,
            });
            column = column.saturating_add(cells);
        }
    }
    (row, column)
}

fn paint_text(
    frame: &mut Frame,
    rect: Rect,
    clip: Rect,
    runs: &[TextRun],
    wrap: WrapMode,
    tab_width: u8,
) -> usize {
    let mut painted = 0;
    walk_text(
        runs.iter().map(|run| (run.text.as_str(), run.style)),
        rect.width,
        wrap,
        tab_width,
        &mut |placement| {
            if placement.row >= rect.height {
                return;
            }
            painted += paint_one(
                frame,
                rect,
                clip,
                placement.grapheme,
                placement.width,
                placement.style,
                placement.row,
                placement.column,
            );
        },
    );
    painted
}

#[allow(clippy::too_many_arguments)]
fn paint_one(
    frame: &mut Frame,
    rect: Rect,
    clip: Rect,
    grapheme: &str,
    width: u16,
    style: CellStyle,
    row: u16,
    column: u16,
) -> usize {
    if width > rect.width || column.saturating_add(width) > rect.width {
        return 0;
    }
    let Some(x) = rect.x.checked_add(i32::from(column)) else {
        return 0;
    };
    let Some(y) = rect.y.checked_add(i32::from(row)) else {
        return 0;
    };
    let Some(last_x) = x.checked_add(i32::from(width.saturating_sub(1))) else {
        return 0;
    };
    if !clip.contains(x, y) || !clip.contains(last_x, y) {
        return 0;
    }
    let (Ok(row), Ok(column)) = (u16::try_from(y), u16::try_from(x)) else {
        return 0;
    };
    frame.paint(row, column, grapheme, width, style);
    usize::from(width)
}

/// Cells a string occupies when laid out in a text node of a given width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextMetrics {
    /// Rows occupied, counting the empty row a trailing newline leaves. Always
    /// at least one.
    pub rows: u32,
    /// Widest row, in cells.
    pub max_width: u16,
    /// Cursor column left after the last grapheme.
    pub last_width: u16,
    /// Cells the same text would paint.
    pub cells: usize,
}

/// One extended grapheme cluster with its terminal cell width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphemeCell {
    pub text: String,
    pub width: u16,
    /// Zero-based byte offset of this cluster in the source string.
    pub offset: usize,
}

/// A single-line string shortened to a cell budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruncatedText {
    pub text: String,
    pub width: u16,
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TextError {
    #[error("text contains terminal control data")]
    TerminalControl,
    #[error("single-line text must not contain a newline or tab")]
    MultilineText,
    #[error("layout width must be non-zero")]
    EmptyWidth,
    #[error("invalid tab width {0}")]
    InvalidTabWidth(u8),
}

/// Reject exactly the control data a display batch rejects, so text these
/// primitives accept is text `RetainedDisplay::submit` also accepts.
fn check_control(text: &str) -> Result<(), TextError> {
    if text.chars().any(|character| {
        character == '\x1b' || (character.is_control() && character != '\n' && character != '\t')
    }) {
        return Err(TextError::TerminalControl);
    }
    Ok(())
}

fn check_single_line(text: &str) -> Result<(), TextError> {
    check_control(text)?;
    if text.contains('\n') || text.contains('\t') {
        return Err(TextError::MultilineText);
    }
    Ok(())
}

fn check_layout(width: u16, tab_width: u8) -> Result<(), TextError> {
    if width == 0 {
        return Err(TextError::EmptyWidth);
    }
    if !(1..=16).contains(&tab_width) {
        return Err(TextError::InvalidTabWidth(tab_width));
    }
    Ok(())
}

fn cell_width(grapheme: &str) -> u16 {
    u16::try_from(grapheme_width(grapheme)).unwrap_or(u16::MAX)
}

/// Terminal cell width of one single-line string.
pub fn text_width(text: &str) -> Result<u16, TextError> {
    check_single_line(text)?;
    Ok(text.graphemes(true).fold(0u16, |width, grapheme| {
        width.saturating_add(cell_width(grapheme))
    }))
}

/// Rows and columns a string occupies in a text node `width` cells wide.
pub fn measure_text(
    text: &str,
    width: u16,
    wrap: WrapMode,
    tab_width: u8,
) -> Result<TextMetrics, TextError> {
    check_control(text)?;
    check_layout(width, tab_width)?;
    let mut max_width = 0u16;
    let mut cells = 0usize;
    let (row, column) = walk_text(
        std::iter::once((text, CellStyle::default())),
        width,
        wrap,
        tab_width,
        &mut |placement| {
            max_width = max_width.max(placement.column.saturating_add(placement.width));
            cells = cells.saturating_add(usize::from(placement.width));
        },
    );
    Ok(TextMetrics {
        rows: u32::from(row).saturating_add(1),
        max_width,
        last_width: column,
        cells,
    })
}

/// Break a string into the rows a text node `width` cells wide would paint.
///
/// Rows past `max_rows` are dropped; the returned flag reports that the text
/// did not fit, so the caller can decide what to do about the remainder.
pub fn wrap_text(
    text: &str,
    width: u16,
    tab_width: u8,
    max_rows: usize,
) -> Result<(Vec<String>, bool), TextError> {
    check_control(text)?;
    check_layout(width, tab_width)?;
    let mut rows: Vec<String> = Vec::new();
    let mut overflow = false;
    let (last_row, _) = walk_text(
        std::iter::once((text, CellStyle::default())),
        width,
        WrapMode::Grapheme,
        tab_width,
        &mut |placement| {
            let index = usize::from(placement.row);
            if index >= max_rows {
                overflow = true;
                return;
            }
            while rows.len() <= index {
                rows.push(String::new());
            }
            rows[index].push_str(placement.grapheme);
        },
    );
    let occupied = usize::from(last_row).saturating_add(1);
    if occupied > max_rows {
        overflow = true;
    }
    while rows.len() < occupied.min(max_rows) {
        rows.push(String::new());
    }
    Ok((rows, overflow))
}

/// Shorten a single-line string to `max_width` cells without splitting a
/// grapheme or leaving half of a wide one, appending `ellipsis` when anything
/// was dropped. An ellipsis wider than the budget is omitted.
pub fn truncate_text(
    text: &str,
    max_width: u16,
    ellipsis: &str,
) -> Result<TruncatedText, TextError> {
    let width = text_width(text)?;
    let ellipsis_width = text_width(ellipsis)?;
    if width <= max_width {
        return Ok(TruncatedText {
            text: text.to_owned(),
            width,
            truncated: false,
        });
    }
    let keep_ellipsis = ellipsis_width <= max_width;
    let budget = if keep_ellipsis {
        max_width - ellipsis_width
    } else {
        max_width
    };
    let mut kept = String::new();
    let mut kept_width = 0u16;
    for grapheme in text.graphemes(true) {
        let cells = cell_width(grapheme);
        if kept_width.saturating_add(cells) > budget {
            break;
        }
        kept.push_str(grapheme);
        kept_width = kept_width.saturating_add(cells);
    }
    if keep_ellipsis {
        kept.push_str(ellipsis);
        kept_width = kept_width.saturating_add(ellipsis_width);
    }
    Ok(TruncatedText {
        text: kept,
        width: kept_width,
        truncated: true,
    })
}

/// Bounded grapheme-cluster window plus the total cluster count.
pub fn text_graphemes(
    text: &str,
    offset: usize,
    limit: usize,
) -> Result<(Vec<GraphemeCell>, usize), TextError> {
    check_control(text)?;
    let mut total = 0usize;
    let mut window = Vec::new();
    for (byte, grapheme) in text.grapheme_indices(true) {
        if total >= offset && window.len() < limit {
            window.push(GraphemeCell {
                text: grapheme.to_owned(),
                width: cell_width(grapheme),
                offset: byte,
            });
        }
        total = total.saturating_add(1);
    }
    Ok((window, total))
}
fn identity_delta(
    previous: &BTreeMap<NodeId, DisplayNode>,
    current: &BTreeMap<NodeId, DisplayNode>,
) -> IdentityDelta {
    let mut delta = IdentityDelta::default();
    for (id, node) in current {
        match previous.get(id) {
            None => delta.added.push(*id),
            Some(old) if old == node => delta.retained.push(*id),
            Some(_) => delta.changed.push(*id),
        }
    }
    for id in previous.keys() {
        if !current.contains_key(id) {
            delta.removed.push(*id);
        }
    }
    delta
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn group(id: u64, rect: Rect, children: Vec<NodeId>) -> DisplayNode {
        DisplayNode {
            id: NodeId(id),
            rect,
            clip_children: true,
            focusable: false,
            content: DisplayNodeContent::Group,
            children,
        }
    }

    fn text(id: u64, rect: Rect, value: &str) -> DisplayNode {
        DisplayNode {
            id: NodeId(id),
            rect,
            clip_children: true,
            focusable: false,
            content: DisplayNodeContent::Text {
                runs: vec![TextRun {
                    text: value.to_owned(),
                    style: CellStyle::default(),
                }],
                wrap: WrapMode::Grapheme,
                tab_width: 4,
            },
            children: Vec::new(),
        }
    }

    fn batch(columns: u16, rows: u16, nodes: Vec<DisplayNode>) -> DisplayBatch {
        DisplayBatch {
            version: DISPLAY_SCHEMA_VERSION,
            viewport: Viewport { columns, rows },
            root: NodeId(1),
            nodes,
            focused: None,
            cursor: None,
        }
    }

    fn cell_text(frame: &Frame, row: u16, column: u16) -> Option<&str> {
        frame.cell(row, column).unwrap().grapheme.as_deref()
    }

    #[test]
    fn unicode_wide_and_combining_graphemes_occupy_cells() {
        let mut display = RetainedDisplay::default();
        let value = "A界e\u{301}";
        display
            .submit(batch(
                8,
                2,
                vec![
                    group(
                        1,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 8,
                            height: 2,
                        },
                        vec![NodeId(2)],
                    ),
                    text(
                        2,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 8,
                            height: 1,
                        },
                        value,
                    ),
                ],
            ))
            .unwrap();
        let frame = display.frame().unwrap();
        assert_eq!(cell_text(frame, 0, 0), Some("A"));
        assert_eq!(cell_text(frame, 0, 1), Some("界"));
        assert!(frame.cell(0, 2).unwrap().continuation);
        assert_eq!(cell_text(frame, 0, 3), Some("e\u{301}"));
    }

    #[test]
    fn relative_layout_wrap_and_ancestor_clip_are_deterministic() {
        let mut display = RetainedDisplay::default();
        display
            .submit(batch(
                6,
                3,
                vec![
                    group(
                        1,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 6,
                            height: 3,
                        },
                        vec![NodeId(2)],
                    ),
                    group(
                        2,
                        Rect {
                            x: 1,
                            y: 1,
                            width: 3,
                            height: 1,
                        },
                        vec![NodeId(3)],
                    ),
                    text(
                        3,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 2,
                            height: 2,
                        },
                        "abcd",
                    ),
                ],
            ))
            .unwrap();
        let frame = display.frame().unwrap();
        assert_eq!(cell_text(frame, 1, 1), Some("a"));
        assert_eq!(cell_text(frame, 1, 2), Some("b"));
        assert_eq!(cell_text(frame, 2, 1), None, "parent clips wrapped row");
    }

    #[test]
    fn resize_forces_redraw_and_updates_dimensions() {
        let mut display = RetainedDisplay::default();
        let initial = batch(
            4,
            2,
            vec![group(
                1,
                Rect {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 2,
                },
                Vec::new(),
            )],
        );
        assert!(display.submit(initial).unwrap().full_redraw);
        let resized = batch(
            6,
            3,
            vec![group(
                1,
                Rect {
                    x: 0,
                    y: 0,
                    width: 6,
                    height: 3,
                },
                Vec::new(),
            )],
        );
        let result = display.submit(resized).unwrap();
        assert!(result.full_redraw);
        assert!(result.ansi.contains("\x1b[2J"));
        assert_eq!(
            display.frame().unwrap().viewport(),
            Viewport {
                columns: 6,
                rows: 3
            }
        );
    }

    #[test]
    fn focus_and_cursor_metadata_use_clipped_absolute_layout() {
        let mut display = RetainedDisplay::default();
        let mut child = text(
            2,
            Rect {
                x: 2,
                y: 1,
                width: 3,
                height: 1,
            },
            "abc",
        );
        child.focusable = true;
        let mut value = batch(
            8,
            3,
            vec![
                group(
                    1,
                    Rect {
                        x: 0,
                        y: 0,
                        width: 8,
                        height: 3,
                    },
                    vec![NodeId(2)],
                ),
                child,
            ],
        );
        value.focused = Some(NodeId(2));
        value.cursor = Some(CursorMetadata {
            node: NodeId(2),
            row: 0,
            column: 2,
            shape: CursorShape::Bar,
            visible: true,
        });
        let result = display.submit(value).unwrap();
        let frame = display.frame().unwrap();
        assert_eq!(frame.focus.unwrap().node, NodeId(2));
        assert_eq!(frame.cursor.unwrap().column, 4);
        assert_eq!(frame.cursor.unwrap().row, 1);
        assert!(result.ansi.contains("\x1b[2;5H\x1b[6 q\x1b[?25h"));

        let mut hidden = batch(
            8,
            3,
            vec![
                group(
                    1,
                    Rect {
                        x: 0,
                        y: 0,
                        width: 8,
                        height: 3,
                    },
                    vec![NodeId(2)],
                ),
                text(
                    2,
                    Rect {
                        x: 2,
                        y: 1,
                        width: 3,
                        height: 1,
                    },
                    "abc",
                ),
            ],
        );
        hidden.cursor = Some(CursorMetadata {
            node: NodeId(2),
            row: 0,
            column: 1,
            shape: CursorShape::Underline,
            visible: false,
        });
        let hidden = display.submit(hidden).unwrap();
        assert!(hidden.ansi.contains("\x1b[?25l"));
        assert!(!hidden.ansi.contains("\x1b[4 q"));
    }

    #[test]
    fn malformed_batch_is_rejected_without_mutating_retained_state() {
        let mut display = RetainedDisplay::default();
        let valid = batch(
            4,
            1,
            vec![group(
                1,
                Rect {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 1,
                },
                Vec::new(),
            )],
        );
        display.submit(valid.clone()).unwrap();
        let prior = display.frame().unwrap().clone();
        let mut malformed = valid;
        malformed.nodes.push(text(
            2,
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            "\x1b[2J",
        ));
        malformed.nodes[0].children.push(NodeId(2));
        assert_eq!(
            display.submit(malformed),
            Err(DisplayError::TerminalControl(NodeId(2)))
        );
        assert_eq!(display.revision(), 1);
        assert_eq!(display.frame(), Some(&prior));
    }

    #[test]
    fn stable_identity_and_minimal_cell_diff_survive_whole_batch_submission() {
        let mut display = RetainedDisplay::default();
        let make = |value: &str| {
            batch(
                8,
                2,
                vec![
                    group(
                        1,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 8,
                            height: 2,
                        },
                        vec![NodeId(2)],
                    ),
                    text(
                        2,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 8,
                            height: 1,
                        },
                        value,
                    ),
                ],
            )
        };
        let first = display.submit(make("abcdef")).unwrap();
        assert_eq!(first.identities.added, [NodeId(1), NodeId(2)]);
        let unchanged = display.submit(make("abcdef")).unwrap();
        assert_eq!(unchanged.identities.retained, [NodeId(1), NodeId(2)]);
        assert!(unchanged.ansi.is_empty());
        let changed = display.submit(make("abcXef")).unwrap();
        assert_eq!(changed.identities.retained, [NodeId(1)]);
        assert_eq!(changed.identities.changed, [NodeId(2)]);
        assert_eq!(changed.changed_cells, 1);
        assert!(changed.ansi.contains('X'));
        assert!(!changed.ansi.contains("abc"));
        assert!(!changed.full_redraw);

        display.reset_presentation();
        let redrawn = display.submit(make("abcXef")).unwrap();
        assert!(redrawn.full_redraw);
        assert_eq!(redrawn.identities.retained, [NodeId(1), NodeId(2)]);
    }

    #[test]
    fn large_batches_are_iterative_and_fail_closed_at_the_limit() {
        let limits = DisplayLimits {
            max_nodes: 2_048,
            max_depth: 2_048,
            max_children_per_node: 2_048,
            ..DisplayLimits::default()
        };
        let mut nodes = Vec::with_capacity(2_048);
        for id in 1..=2_048u64 {
            nodes.push(group(
                id,
                Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                (id < 2_048).then(|| NodeId(id + 1)).into_iter().collect(),
            ));
        }
        nodes[0].rect = Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut display = RetainedDisplay::new(limits);
        let result = display.submit(batch(2, 1, nodes.clone())).unwrap();
        assert_eq!(result.visited_nodes, 2_048);

        nodes.push(group(
            2_049,
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            Vec::new(),
        ));
        assert_eq!(
            display.submit(batch(2, 1, nodes)),
            Err(DisplayError::NodeLimit {
                actual: 2_049,
                limit: 2_048
            })
        );
        assert_eq!(display.revision(), 1);
    }

    #[test]
    fn measurement_predicts_the_cells_the_same_text_paints() {
        // Wide, combining, tab, and newline graphemes in one string, laid out
        // at a width that forces a wrap mid-line.
        let value = "ab\t界e\u{301}xyz\ntail";
        let width = 6;
        let metrics = measure_text(value, width, WrapMode::Grapheme, 4).unwrap();
        let rows = u16::try_from(metrics.rows).unwrap();

        let mut display = RetainedDisplay::default();
        let result = display
            .submit(batch(
                width,
                rows,
                vec![
                    group(
                        1,
                        Rect {
                            x: 0,
                            y: 0,
                            width,
                            height: rows,
                        },
                        vec![NodeId(2)],
                    ),
                    text(
                        2,
                        Rect {
                            x: 0,
                            y: 0,
                            width,
                            height: rows,
                        },
                        value,
                    ),
                ],
            ))
            .unwrap();
        // A node sized from the measurement paints exactly the measured cells:
        // nothing overflows its rows and nothing is clipped away.
        assert_eq!(result.painted_cells, metrics.cells);

        // The wrapped rows reproduce the painted frame row for row.
        let (wrapped, overflow) = wrap_text(value, width, 4, 64).unwrap();
        assert!(!overflow);
        assert_eq!(wrapped.len(), usize::try_from(metrics.rows).unwrap());
        let frame = display.frame().unwrap();
        for (row, expected) in wrapped.iter().enumerate() {
            let painted: String = (0..width)
                .filter_map(|column| cell_text(frame, u16::try_from(row).unwrap(), column))
                .collect();
            assert_eq!(&painted, expected);
        }
    }

    #[test]
    fn text_primitives_refuse_control_data_and_respect_row_budgets() {
        assert_eq!(text_width("a\tb"), Err(TextError::MultilineText));
        assert_eq!(text_width("a\nb"), Err(TextError::MultilineText));
        assert_eq!(text_width("a\x1b[0m"), Err(TextError::TerminalControl));
        assert_eq!(text_width("a\r"), Err(TextError::TerminalControl));
        assert_eq!(
            measure_text("a", 0, WrapMode::Grapheme, 4),
            Err(TextError::EmptyWidth)
        );
        assert_eq!(
            measure_text("a", 4, WrapMode::Grapheme, 0),
            Err(TextError::InvalidTabWidth(0))
        );

        // A trailing newline occupies the empty row it opens.
        let metrics = measure_text("ab\n", 4, WrapMode::Grapheme, 4).unwrap();
        assert_eq!(metrics.rows, 2);
        assert_eq!(metrics.last_width, 0);
        assert_eq!(metrics.cells, 2);

        // Clip mode reports the columns it dropped instead of wrapping them.
        let clipped = measure_text("abcdef", 4, WrapMode::Clip, 4).unwrap();
        assert_eq!(clipped.rows, 1);
        assert_eq!(clipped.cells, 4);
        assert_eq!(clipped.last_width, 6);

        let (rows, overflow) = wrap_text("abcdef", 4, 4, 1).unwrap();
        assert_eq!(rows, vec!["abcd".to_owned()]);
        assert!(overflow);

        // Truncation fills the budget with whole clusters: the second wide
        // cluster does not fit beside the ellipsis, so it is dropped entire.
        assert_eq!(
            truncate_text("a界界", 4, "…").unwrap(),
            TruncatedText {
                text: "a界…".to_owned(),
                width: 4,
                truncated: true,
            }
        );
        // A budget too small for the first cluster keeps only the ellipsis.
        assert_eq!(
            truncate_text("界界", 2, "…").unwrap(),
            TruncatedText {
                text: "…".to_owned(),
                width: 1,
                truncated: true,
            }
        );
        assert_eq!(
            truncate_text("abc", 8, "…").unwrap(),
            TruncatedText {
                text: "abc".to_owned(),
                width: 3,
                truncated: false,
            }
        );

        let (window, total) = text_graphemes("e\u{301}界x", 1, 1).unwrap();
        assert_eq!(total, 3);
        assert_eq!(
            window,
            vec![GraphemeCell {
                text: "界".to_owned(),
                width: 2,
                offset: 3,
            }]
        );
    }
}
