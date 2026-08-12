//! Compact terminal-frame evidence format (v1).
//!
//! Reduces 140+ MiB of per-cell JSON to ~5 MiB by using a shared style palette
//! and same-style text runs per row, skipping default/empty cells.
//!
//! ```json
//! {"v":1,"p":[{style...}], "f":[
//!   {"n":"name","g":[72,20],"c":[4,0,1],"r":[[row,col,text,style,single?],...]},
//! ]}
//! ```

use serde::{Deserialize, Serialize};
use super::ui_harness::{CellSnapshot, FrameSnapshot};

// --- types ------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactEvidence {
    pub v: u8,
    pub p: Vec<Style>,
    pub f: Vec<Frame>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Style {
    #[serde(rename = "f")] pub fg: String,
    #[serde(rename = "b")] pub bg: String,
    #[serde(rename = "l")] pub bold: u8,
    #[serde(rename = "d")] pub dim: u8,
    #[serde(rename = "i")] pub italic: u8,
    #[serde(rename = "u")] pub underline: u8,
    #[serde(rename = "v")] pub inverse: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Run {
    pub row: u16,
    pub col: u16,
    pub text: String,
    pub style: u16,
    /// 1 = place all text in ONE cell (multi-char cell like emoji + VS16)
    #[serde(default)]
    pub single: u8,
    /// 0-based character offsets in `text` that are double-width (CJK):
    /// each occupies its own cell plus one wide-continuation cell after it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub w: Vec<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Frame {
    pub n: String,
    pub g: [u16; 2],
    pub c: [u16; 3],
    pub r: Vec<Run>,
}

// --- style helpers ----------------------------------------------------------

fn style_from_cell(c: &CellSnapshot) -> Style {
    Style {
        fg: c.foreground.clone(), bg: c.background.clone(),
        bold: c.bold as u8, dim: c.dim as u8,
        italic: c.italic as u8, underline: c.underline as u8,
        inverse: c.inverse as u8,
    }
}

fn style_to_cell(s: &Style) -> CellSnapshot {
    CellSnapshot {
        text: String::new(), wide: false, wide_continuation: false,
        foreground: s.fg.clone(), background: s.bg.clone(),
        bold: s.bold != 0, dim: s.dim != 0,
        italic: s.italic != 0, underline: s.underline != 0,
        inverse: s.inverse != 0,
    }
}

fn style_is_default(s: &Style) -> bool {
    s.fg == "default" && s.bg == "default"
        && s.bold == 0 && s.dim == 0 && s.italic == 0
        && s.underline == 0 && s.inverse == 0
}

fn cell_is_boring(c: &CellSnapshot) -> bool {
    c.text.is_empty() && c.foreground == "default"
        && c.background == "default" && !c.bold && !c.dim
        && !c.italic && !c.underline && !c.inverse
}

// --- compression ------------------------------------------------------------

fn build_palette(frames: &[FrameSnapshot]) -> Vec<Style> {
    let mut pal: Vec<Style> = vec![];
    for frame in frames {
        for cell in &frame.cells {
            let s = style_from_cell(cell);
            if !pal.contains(&s) { pal.push(s); }
        }
    }
    let def = Style {
        fg: "default".into(), bg: "default".into(),
        bold: 0, dim: 0, italic: 0, underline: 0, inverse: 0,
    };
    if !pal.contains(&def) { pal.insert(0, def); }
    pal
}

fn build_runs(frame: &FrameSnapshot, palette: &[Style]) -> Vec<Run> {
    let cols = frame.columns as usize;
    let mut runs = vec![];

    for row in 0..frame.rows as usize {
        let mut col = 0;
        while col < cols {
            let cell = &frame.cells[row * cols + col];
            if cell_is_boring(cell) { col += 1; continue; }
            let cs = style_from_cell(cell);
            let si = palette.iter().position(|s| *s == cs).unwrap_or(0);
            let start = col;
            let mut text = String::new();
            let mut single_run = false;
            let mut wide_offsets: Vec<u16> = vec![];

            while col < cols {
                let b = &frame.cells[row * cols + col];
                let bs = style_from_cell(b);
                let bi = palette.iter().position(|s| *s == bs).unwrap_or(0);
                if bi != si { break; }

                // Multi-char cell (emoji + VS16) — store text whole, emit single-cell run
                if !b.text.is_empty() && b.text.chars().count() > 1 {
                    text.push_str(&b.text);
                    col += 1;
                    single_run = true;
                    break;
                }

                if b.wide_continuation {
                    // Covered by the preceding wide cell; skip silently.
                    col += 1; continue;
                }
                if b.wide {
                    let idx = text.chars().count() as u16;
                    text.push_str(&b.text);
                    col += 1;
                    if col < cols {
                        text.push_str(&frame.cells[row * cols + col].text);
                        col += 1;
                    }
                    wide_offsets.push(idx);
                    continue;
                }
                if !b.text.is_empty() {
                    text.push_str(&b.text);
                }
                col += 1;
            }

            if !text.is_empty() || !style_is_default(&cs) {
                runs.push(Run {
                    row: row as u16,
                    col: start as u16,
                    text,
                    style: si as u16,
                    single: single_run as u8,
                    w: wide_offsets,
                });
            }
        }
    }
    runs
}

pub fn frames_to_compact(frames: &[FrameSnapshot]) -> CompactEvidence {
    let palette = build_palette(frames);
    let frames: Vec<Frame> = frames.iter().map(|fr| Frame {
        n: fr.name.clone(),
        g: [fr.columns, fr.rows],
        c: [fr.cursor_row, fr.cursor_column, u16::from(fr.cursor_visible)],
        r: build_runs(fr, &palette),
    }).collect();
    CompactEvidence { v: 1, p: palette, f: frames }
}

// --- decompression ----------------------------------------------------------

pub fn compact_to_frames(ev: &CompactEvidence) -> Vec<FrameSnapshot> {
    let temps: Vec<CellSnapshot> = ev.p.iter().map(style_to_cell).collect();
    let def = CellSnapshot {
        text: String::new(), wide: false, wide_continuation: false,
        foreground: "default".into(), background: "default".into(),
        bold: false, dim: false, italic: false, underline: false, inverse: false,
    };

    ev.f.iter().map(|cf| {
        let cols = cf.g[0] as usize;
        let rows = cf.g[1] as usize;
        let total = cols * rows;

        let mut cells: Vec<CellSnapshot> = (0..total).map(|_| {
            let mut c = def.clone();
            c.text = String::new();
            c
        }).collect();

        for run in &cf.r {
            let style = temps.get(run.style as usize).unwrap_or(&def);
            let base = style.clone();
            if run.single == 1 {
                // Entire text placed in ONE cell (multi-char emoji + VS16)
                let idx = run.row as usize * cols + run.col as usize;
                if idx < total {
                    cells[idx] = base;
                    cells[idx].text = run.text.clone();
                }
            } else {
                let wide: std::collections::HashSet<u16> = run.w.iter().copied().collect();
                let mut col = run.col as usize;
                for (i, ch) in run.text.chars().enumerate() {
                    let offset = i as u16;
                    if wide.contains(&offset) {
                        let base_idx = run.row as usize * cols + col;
                        if base_idx < total {
                            let mut c = base.clone();
                            c.text = ch.to_string();
                            c.wide = true;
                            cells[base_idx] = c;
                        }
                        // Following cell is the wide continuation half.
                        let cont_idx = base_idx + 1;
                        if cont_idx < total {
                            let mut cc = base.clone();
                            cc.text = String::new();
                            cc.wide_continuation = true;
                            cells[cont_idx] = cc;
                        }
                        col += 2;
                    } else {
                        let idx = run.row as usize * cols + col;
                        if idx >= total { break; }
                        let mut c = base.clone();
                        c.text = ch.to_string();
                        cells[idx] = c;
                        col += 1;
                    }
                }
            }
        }

        FrameSnapshot {
            name: cf.n.clone(),
            columns: cf.g[0],
            rows: cf.g[1],
            cursor_row: cf.c[0],
            cursor_column: cf.c[1],
            cursor_visible: cf.c[2] != 0,
            cells,
        }
    }).collect()
}
/// Load frame snapshots from a .pci.json (compact) or .pi.json (legacy) file.
/// Tries .pci.json first, falls back to .pi.json.
///
/// Used by ui-diff.rs to transparently transition.
pub fn load_frames(path: &std::path::Path) -> std::io::Result<Vec<FrameSnapshot>> {
    let data = std::fs::read_to_string(path)?;
    // Try compact format first (detect by "v" key)
    if let Ok(ev) = serde_json::from_str::<CompactEvidence>(&data) {
        return Ok(compact_to_frames(&ev));
    }
    // Fall back: parse as legacy Vec<FrameSnapshot>
    serde_json::from_str::<Vec<FrameSnapshot>>(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::ui_harness::{CellSnapshot, FrameSnapshot};

    fn cell(text: &str, fg: &str, bold: bool) -> CellSnapshot {
        CellSnapshot {
            text: text.into(), wide: false, wide_continuation: false,
            foreground: fg.into(), background: "default".into(),
            bold, dim: false, italic: false, underline: false, inverse: false,
        }
    }

    fn blank() -> CellSnapshot {
        cell("", "default", false)
    }

    fn sample_frames() -> Vec<FrameSnapshot> {
        let cols = 4u16; let rows = 3u16;
        // Row 0: default blanks then a styled run "ab".
        // Row 1: a wide char (glyph in col 0, continuation in col 1), then style.
        // Row 2: a multi-char emoji+VS16 cell in col 0 (single run).
        let mut cells = vec![blank(); (cols * rows) as usize];
        cells[2] = cell("a", "rgb:ff0000", true);
        cells[3] = cell("b", "rgb:ff0000", true);
        cells[4] = cell("界", "default", false);
        cells[5] = cell("", "default", false);
        cells[4].wide = true;
        cells[5].wide_continuation = true;
        let mut emoji = cell("👍\u{fe0f}", "default", false);
        emoji.text = "👍\u{fe0f}".into();
        cells[8] = emoji;
        vec![FrameSnapshot {
            name: "f0".into(), columns: cols, rows,
            cursor_row: 2, cursor_column: 1, cursor_visible: true,
            cells,
        }]
    }

    #[test]
    fn encode_decode_round_trips_grid() {
        let frames = sample_frames();
        let compact = frames_to_compact(&frames);
        let decoded = compact_to_frames(&compact);
        for (i, (a, b)) in frames.iter().zip(decoded.iter()).enumerate() {
            assert_eq!(a.name, b.name, "frame {i} name");
            assert_eq!(a.columns, b.columns);
            assert_eq!(a.rows, b.rows);
            assert_eq!(a.cursor_row, b.cursor_row);
            assert_eq!(a.cursor_column, b.cursor_column);
            assert_eq!(a.cursor_visible, b.cursor_visible);
            for (j, (x, y)) in a.cells.iter().zip(b.cells.iter()).enumerate() {
                assert_eq!(x, y, "frame {i} cell {j}");
            }
        }
    }

    #[test]
    fn decode_then_encode_is_byte_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let frames = sample_frames();
        let a = frames_to_compact(&frames);
        // Decoding then re-encoding yields the identical compact document.
        let decoded = compact_to_frames(&a);
        let b = frames_to_compact(&decoded);
        let json_a = serde_json::to_string(&a)?;
        let json_b = serde_json::to_string(&b)?;
        assert_eq!(json_a, json_b);
        Ok(())
    }

    #[test]
    fn default_only_grid_is_all_blank() {
        let frames = vec![FrameSnapshot {
            name: "empty".into(), columns: 2, rows: 2,
            cursor_row: 0, cursor_column: 0, cursor_visible: false,
            cells: vec![blank(); 4],
        }];
        let compact = frames_to_compact(&frames);
        assert!(compact.f[0].r.is_empty(), "all-default frame must emit no runs");
        let decoded = compact_to_frames(&compact);
        for c in &decoded[0].cells { assert!(cell_is_boring(c)); }
    }

    #[test]
    fn compact_negative_control_identifies_first_mismatched_cell() {
        // Mutate a non-default run glyph and confirm `first_diff` reports the
        // exact cell through the decoded (compact) frames, satisfying A.1's
        // negative-control criterion at the compact-evidence boundary.
        let frames = sample_frames();
        let compact = frames_to_compact(&frames);
        let mut decoded = compact_to_frames(&compact);
        // cells[2] (row 0, col 2, 4 columns wide) holds the styled "a" glyph
        // from the "ab" run; overwrite it and require the exact cell mismatch.
        decoded[0].cells[2].text = "X".into();
        let diff = match crate::ui_harness::first_diff(&frames, &decoded) {
            Some(diff) => diff,
            None => panic!("mutation must be detected"),
        };
        assert_eq!(diff.checkpoint, "f0");
        assert!(
            diff.message.contains("cell (0,2) differs"),
            "expected first mismatch at (0,2), got: {}",
            diff.message
        );
    }
}
