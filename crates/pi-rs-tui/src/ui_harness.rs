//! Stable terminal-cell snapshots and actionable UI parity diffs.
//!
//! Renderers are compared after terminal emulation rather than by raw ANSI:
//! different escape sequences may produce the same observable screen.

use std::collections::HashMap;

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


// ---------------------------------------------------------------------------
// Compact, versioned oracle encoding (PLAN A.1).
//
// The verbose one-object-per-cell format is replaced by a single shared
// style palette (foreground/background/attr-bitmask), a shared glyph table
// (distinct cell texts), and run-length-encoded cell grids. The result is
// canonical and human-reviewable (no compressed opaque blobs), round-trips
// byte-identically, and retains color, attributes, wide cells, cursor,
// geometry, and ordering.
//
// Cell token grammar (inside a frame's `cells` string):
//   grid   := run ( ';' run )*
//   run    := count 'x' style ':' glyph ':' flags
//   count  := decimal number of consecutive identical cells
//   style  := decimal index into `oracle.palette`
//   glyph  := decimal index into `oracle.glyphs`
//   flags  := decimal 0..3; bit0 = wide, bit1 = wide_continuation
//
// Attribute bitmask (`CompactStyle.a`): bit0=bold, bit1=dim, bit2=italic,
// bit3=underline, bit4=inverse.
// ---------------------------------------------------------------------------

pub const COMPACT_ORACLE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompactStyle {
    pub f: String,
    pub b: String,
    pub a: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompactFrame {
    pub name: String,
    pub w: u16,
    pub h: u16,
    /// `[cursor_row, cursor_column, cursor_visible(0|1)]`.
    pub cur: [u16; 3],
    /// RLE-encoded grid; decoded cell count equals `w * h`.
    pub cells: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompactOracle {
    pub version: u32,
    /// Distinct cell `text` values in first-appearance order.
    pub glyphs: Vec<String>,
    /// Distinct `(foreground, background, attrs)` styles in first-appearance order.
    pub palette: Vec<CompactStyle>,
    pub frames: Vec<CompactFrame>,
}

fn style_attrs(cell: &CellSnapshot) -> u8 {
    (u8::from(cell.bold))
        | (u8::from(cell.dim) << 1)
        | (u8::from(cell.italic) << 2)
        | (u8::from(cell.underline) << 3)
        | (u8::from(cell.inverse) << 4)
}

#[allow(clippy::type_complexity)]
fn style_fields(style: &CompactStyle) -> (String, String, bool, bool, bool, bool, bool) {
    (
        style.f.clone(),
        style.b.clone(),
        style.a & 1 != 0,
        style.a & 2 != 0,
        style.a & 4 != 0,
        style.a & 8 != 0,
        style.a & 16 != 0,
    )
}

impl CompactOracle {
    /// Build a compact oracle from decoded snapshots. Palette and glyph
    /// tables use first-appearance order, so re-encoding the decoded output
    /// reproduces identical bytes (byte-idempotent regeneration).
    pub fn encode(frames: &[FrameSnapshot]) -> Self {
        let mut glyph_index: HashMap<&str, usize> = HashMap::new();
        let mut glyphs: Vec<String> = Vec::new();
        let mut style_index: HashMap<(String, String, u8), usize> = HashMap::new();
        let mut palette: Vec<CompactStyle> = Vec::new();

        let compact_frames = frames
            .iter()
            .map(|frame| {
                let mut runs: Vec<String> = Vec::new();
                let mut prev: Option<(usize, usize, bool, bool)> = None;
                let mut run_len = 0usize;
                for cell in &frame.cells {
                    let attrs = style_attrs(cell);
                    let style = *style_index
                        .entry((cell.foreground.clone(), cell.background.clone(), attrs))
                        .or_insert_with(|| {
                            palette.push(CompactStyle {
                                f: cell.foreground.clone(),
                                b: cell.background.clone(),
                                a: attrs,
                            });
                            palette.len() - 1
                        });
                    let glyph = *glyph_index.entry(cell.text.as_str()).or_insert_with(|| {
                        glyphs.push(cell.text.clone());
                        glyphs.len() - 1
                    });
                    let key = (style, glyph, cell.wide, cell.wide_continuation);
                    if prev == Some(key) {
                        run_len += 1;
                    } else {
                        if let Some((s, g, wide, wide_cont)) = prev {
                            let flags = (u8::from(wide)) | (u8::from(wide_cont) << 1);
                            runs.push(format!("{run_len}x{s}:{g}:{flags}"));
                        }
                        prev = Some(key);
                        run_len = 1;
                    }
                }
                if let Some((s, g, wide, wide_cont)) = prev {
                    let flags = (u8::from(wide)) | (u8::from(wide_cont) << 1);
                    runs.push(format!("{run_len}x{s}:{g}:{flags}"));
                }
                CompactFrame {
                    name: frame.name.clone(),
                    w: frame.columns,
                    h: frame.rows,
                    cur: [
                        frame.cursor_row,
                        frame.cursor_column,
                        u16::from(frame.cursor_visible),
                    ],
                    cells: runs.join(";"),
                }
            })
            .collect();

        CompactOracle {
            version: COMPACT_ORACLE_VERSION,
            glyphs,
            palette,
            frames: compact_frames,
        }
    }

    /// Decode a compact oracle back into full snapshots, reproducing every
    /// cell field exactly.
    pub fn decode(&self) -> Vec<FrameSnapshot> {
        self.frames
            .iter()
            .map(|frame| {
                let total = usize::from(frame.w) * usize::from(frame.h);
                let mut cells = Vec::with_capacity(total);
                for run in frame.cells.split(';') {
                    if run.is_empty() {
                        continue;
                    }
                    let Some((count, rest)) = run.split_once('x') else { continue };
                    let count: usize = count.parse().unwrap_or(0);
                    let mut parts = rest.split(':');
                    let style: usize = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0)
                        .min(self.palette.len().saturating_sub(1));
                    let glyph: usize = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0)
                        .min(self.glyphs.len().saturating_sub(1));
                    let flags: u8 = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let (f, b, bold, dim, italic, underline, inverse) =
                        style_fields(&self.palette[style]);
                    let text = self.glyphs[glyph].clone();
                    let wide = flags & 1 != 0;
                    let wide_continuation = flags & 2 != 0;
                    cells.extend(std::iter::repeat_with(|| CellSnapshot {
                        text: text.clone(),
                        wide,
                        wide_continuation,
                        foreground: f.clone(),
                        background: b.clone(),
                        bold,
                        dim,
                        italic,
                        underline,
                        inverse,
                    }).take(count));
                }
                cells.resize(total, CellSnapshot {
                    text: String::new(),
                    wide: false,
                    wide_continuation: false,
                    foreground: String::from("default"),
                    background: String::from("default"),
                    bold: false,
                    dim: false,
                    italic: false,
                    underline: false,
                    inverse: false,
                });
                FrameSnapshot {
                    name: frame.name.clone(),
                    columns: frame.w,
                    rows: frame.h,
                    cursor_row: frame.cur[0],
                    cursor_column: frame.cur[1],
                    cursor_visible: frame.cur[2] != 0,
                    cells,
                }
            })
            .collect()
    }
}


#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{CompactOracle, FrameRecorder, first_diff};

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

    #[test]
    fn compact_encode_decode_round_trips_exactly() {
        let mut recorder = FrameRecorder::new(6, 2);
        recorder.process(
            b"\x1b[31;1mA\x1b[0m \xe7\x95\x8c\x1b[?25l\x1b[1B\x1b[32mp\x1b[0mPi",
        );
        let frame = recorder.snapshot("startup");
        let oracle = CompactOracle::encode(std::slice::from_ref(&frame));
        assert_eq!(oracle.version, super::COMPACT_ORACLE_VERSION);
        let decoded = oracle.decode();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].cells, frame.cells);
        assert_eq!(decoded, vec![frame]);
        // Re-encoding the decoded output is byte-identical (idempotent):
        // palette/glyph tables come from first-appearance order, so the
        // decoded cells encode to the same compact structure.
        let again = CompactOracle::encode(&decoded);
        assert_eq!(again, oracle);
    }

    #[test]
    fn compact_multi_frame_round_trips() {
        let mut recorder = FrameRecorder::new(4, 3);
        recorder.process(b"abcd");
        let mut frames = vec![recorder.snapshot("a")];
        recorder.process(b"\x1b[2;1Hx\x1b[2;2Hy");
        frames.push(recorder.snapshot("b"));
        let compact = CompactOracle::encode(&frames);
        assert_eq!(compact.decode(), frames);
    }

    #[test]
    fn compact_runs_decode_to_full_grid() {
        // Two identical style/glyph runs plus a wide cell flag.
        let mut recorder = FrameRecorder::new(2, 1);
        recorder.process(b"\x1b[1mab\x1b[0m");
        let frame = recorder.snapshot("row");
        let compact = CompactOracle::encode(std::slice::from_ref(&frame));
        // Grid must decode back to exactly w*h cells.
        assert_eq!(compact.decode()[0].cells.len(), 2);
        assert_eq!(compact.decode()[0], frame);
    }

}
