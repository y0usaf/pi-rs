//! Stable terminal-cell snapshots and actionable UI parity diffs.
//!
//! Renderers are compared after terminal emulation rather than by raw ANSI:
//! different escape sequences may produce the same observable screen.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CellSnapshot {
    pub text: String,
    pub wide: bool,
    pub wide_continuation: bool,
    pub foreground: String,
    pub background: String,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FrameSnapshot {
    pub name: String,
    pub columns: u16,
    pub rows: u16,
    pub cursor_row: u16,
    pub cursor_column: u16,
    pub cursor_visible: bool,
    pub cells: Vec<CellSnapshot>,
}

fn color(value: vt100::Color) -> String {
    match value {
        vt100::Color::Default => "default".to_owned(),
        vt100::Color::Idx(index) => format!("index:{index}"),
        vt100::Color::Rgb(red, green, blue) => format!("rgb:{red:02x}{green:02x}{blue:02x}"),
    }
}

/// Stateful terminal emulator used by both oracle and candidate adapters.
pub struct FrameRecorder {
    parser: vt100::Parser,
    columns: u16,
    rows: u16,
}

impl FrameRecorder {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, columns, 10_000),
            columns,
            rows,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, columns: u16, rows: u16) {
        self.columns = columns;
        self.rows = rows;
        self.parser.screen_mut().set_size(rows, columns);
    }

    pub fn snapshot(&self, name: impl Into<String>) -> FrameSnapshot {
        let screen = self.parser.screen();
        let (cursor_row, cursor_column) = screen.cursor_position();
        let mut cells = Vec::with_capacity(usize::from(self.columns) * usize::from(self.rows));
        for row in 0..self.rows {
            for column in 0..self.columns {
                let cell = screen.cell(row, column);
                cells.push(match cell {
                    Some(cell) => CellSnapshot {
                        text: cell.contents().to_owned(),
                        wide: cell.is_wide(),
                        wide_continuation: cell.is_wide_continuation(),
                        foreground: color(cell.fgcolor()),
                        background: color(cell.bgcolor()),
                        bold: cell.bold(),
                        dim: cell.dim(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        inverse: cell.inverse(),
                    },
                    None => CellSnapshot {
                        text: String::new(),
                        wide: false,
                        wide_continuation: false,
                        foreground: "default".to_owned(),
                        background: "default".to_owned(),
                        bold: false,
                        dim: false,
                        italic: false,
                        underline: false,
                        inverse: false,
                    },
                });
            }
        }
        FrameSnapshot {
            name: name.into(),
            columns: self.columns,
            rows: self.rows,
            cursor_row,
            cursor_column,
            cursor_visible: !screen.hide_cursor(),
            cells,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameDiff {
    pub checkpoint: String,
    pub message: String,
}

fn visible(text: &str) -> String {
    if text.is_empty() {
        "·".to_owned()
    } else {
        text.replace(' ', "␠")
    }
}

fn row_text(frame: &FrameSnapshot, row: u16) -> String {
    let start = usize::from(row) * usize::from(frame.columns);
    let end = start + usize::from(frame.columns);
    frame.cells[start..end]
        .iter()
        .map(|cell| {
            if cell.wide_continuation {
                String::new()
            } else if cell.text.is_empty() {
                " ".to_owned()
            } else {
                cell.text.clone()
            }
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Return the first sequence, geometry, cursor, glyph, or style mismatch.
pub fn first_diff(expected: &[FrameSnapshot], actual: &[FrameSnapshot]) -> Option<FrameDiff> {
    if expected.len() != actual.len() {
        return Some(FrameDiff {
            checkpoint: "sequence".to_owned(),
            message: format!(
                "expected {} checkpoints, got {}",
                expected.len(),
                actual.len()
            ),
        });
    }
    for (expected, actual) in expected.iter().zip(actual) {
        if expected.name != actual.name {
            return Some(FrameDiff {
                checkpoint: expected.name.clone(),
                message: format!(
                    "checkpoint name differs: expected {:?}, got {:?}",
                    expected.name, actual.name
                ),
            });
        }
        if (expected.columns, expected.rows) != (actual.columns, actual.rows) {
            return Some(FrameDiff {
                checkpoint: expected.name.clone(),
                message: format!(
                    "geometry differs: expected {}x{}, got {}x{}",
                    expected.columns, expected.rows, actual.columns, actual.rows
                ),
            });
        }
        let expected_cursor = (
            expected.cursor_row,
            expected.cursor_column,
            expected.cursor_visible,
        );
        let actual_cursor = (
            actual.cursor_row,
            actual.cursor_column,
            actual.cursor_visible,
        );
        if expected_cursor != actual_cursor {
            return Some(FrameDiff {
                checkpoint: expected.name.clone(),
                message: format!(
                    "cursor differs: expected row={} column={} visible={}, got row={} column={} visible={}",
                    expected_cursor.0,
                    expected_cursor.1,
                    expected_cursor.2,
                    actual_cursor.0,
                    actual_cursor.1,
                    actual_cursor.2
                ),
            });
        }
        for (index, (expected_cell, actual_cell)) in
            expected.cells.iter().zip(&actual.cells).enumerate()
        {
            if expected_cell != actual_cell {
                let row = (index / usize::from(expected.columns)) as u16;
                let column = (index % usize::from(expected.columns)) as u16;
                return Some(FrameDiff {
                    checkpoint: expected.name.clone(),
                    message: format!(
                        "cell ({row},{column}) differs: expected glyph={} fg={} bg={} attrs=[b:{} d:{} i:{} u:{} inv:{}], got glyph={} fg={} bg={} attrs=[b:{} d:{} i:{} u:{} inv:{}]\nexpected row: {:?}\nactual row:   {:?}",
                        visible(&expected_cell.text),
                        expected_cell.foreground,
                        expected_cell.background,
                        expected_cell.bold,
                        expected_cell.dim,
                        expected_cell.italic,
                        expected_cell.underline,
                        expected_cell.inverse,
                        visible(&actual_cell.text),
                        actual_cell.foreground,
                        actual_cell.background,
                        actual_cell.bold,
                        actual_cell.dim,
                        actual_cell.italic,
                        actual_cell.underline,
                        actual_cell.inverse,
                        row_text(expected, row),
                        row_text(actual, row)
                    ),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{FrameRecorder, first_diff};

    #[test]
    fn records_cells_styles_cursor_and_resize() {
        let mut recorder = FrameRecorder::new(5, 2);
        recorder.process(b"\x1b[31;1mA\x1b[0m \xe7\x95\x8c\x1b[?25l");
        let frame = recorder.snapshot("startup");
        assert_eq!(frame.cells[0].text, "A");
        assert_eq!(frame.cells[0].foreground, "index:1");
        assert!(frame.cells[0].bold);
        assert!(frame.cells[2].wide);
        assert!(frame.cells[3].wide_continuation);
        assert!(!frame.cursor_visible);
        recorder.resize(7, 3);
        assert_eq!(recorder.snapshot("resize").cells.len(), 21);
    }

    #[test]
    fn diff_reports_prefix_glyph_and_location() {
        let mut oracle = FrameRecorder::new(20, 2);
        oracle.process(b"hello");
        let expected = vec![oracle.snapshot("submitted")];
        let mut candidate = FrameRecorder::new(20, 2);
        candidate.process(b"you: hello\x1b[5D");
        let actual = vec![candidate.snapshot("submitted")];
        let diff = first_diff(&expected, &actual).expect("prefix must differ");
        assert!(diff.message.contains("cell (0,0)"));
        assert!(diff.message.contains("expected row: \"hello\""));
        assert!(diff.message.contains("actual row:   \"you: hello\""));
    }
}
