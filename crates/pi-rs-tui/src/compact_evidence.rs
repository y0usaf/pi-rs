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
                    col += 1; continue;
                }
                if b.wide {
                    text.push_str(&b.text);
                    col += 1;
                    if col < cols {
                        text.push_str(&frame.cells[row * cols + col].text);
                        col += 1;
                    }
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
                for (i, ch) in run.text.chars().enumerate() {
                    let idx = run.row as usize * cols + run.col as usize + i;
                    if idx >= total { break; }
                    let mut c = base.clone();
                    c.text = ch.to_string();
                    cells[idx] = c;
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
