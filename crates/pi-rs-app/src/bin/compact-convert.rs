//! Convert and verify compact UI evidence format.
//!
//! Usage: compact-convert convert tests/ui-parity/*.pi.json
//!        compact-convert verify tests/ui-parity/*.pci.json

use std::{env, fs, path::PathBuf, process::ExitCode};

use pi_rs_tui::compact_evidence::{self, CompactEvidence};
use pi_rs_tui::ui_harness::{FrameSnapshot, first_diff};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: compact-convert convert|verify <paths...>");
        return ExitCode::FAILURE;
    }
    match args[1].as_str() {
        "convert" => convert(&args[2..]),
        "verify" => verify(&args[2..]),
        _ => { eprintln!("subcommand: convert or verify"); ExitCode::FAILURE }
    }
}

fn convert(paths: &[String]) -> ExitCode {
    let mut ok = true;
    let mut total_orig = 0u64;
    let mut total_comp = 0u64;
    for path in paths {
        let p = PathBuf::from(path);
        let data = match fs::read_to_string(&p) {
            Ok(d) => d,
            Err(e) => { eprintln!("read {}: {e}", p.display()); ok = false; continue; }
        };
        let frames: Vec<FrameSnapshot> = match serde_json::from_str(&data) {
            Ok(f) => f,
            Err(e) => { eprintln!("parse {}: {e}", p.display()); ok = false; continue; }
        };
        let compact = compact_evidence::frames_to_compact(&frames);
        let compact_json = serde_json::to_string(&compact).unwrap_or_default();
        // strip `.pi.json` → `.pci.json`
        let pci = p.with_file_name(
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .replace(".pi.json", ".pci.json"),
        );
        if let Err(e) = fs::write(&pci, compact_json.as_bytes()) {
            eprintln!("write {}: {e}", pci.display());
            ok = false; continue;
        }
        let o = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let c = compact_json.len() as u64;
        total_orig += o; total_comp += c;
        let pct = if o > 0 { 100 - (100 * c / o) } else { 0 };
        println!("{:.45} {:>8}B -> {:>8}B ({}%)",
                 p.file_name().unwrap().to_string_lossy(), o, c, pct);
    }
    if paths.len() > 1 && total_orig > 0 {
        println!("{:.45} {:>8}B -> {:>8}B ({}%)",
                 "TOTAL", total_orig, total_comp,
                 100 - (100 * total_comp / total_orig));
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn verify(paths: &[String]) -> ExitCode {
    let mut ok = true;
    for path in paths {
        let p = PathBuf::from(path);
        let compact_data = match fs::read_to_string(&p) {
            Ok(d) => d,
            Err(e) => { eprintln!("read {}: {e}", p.display()); ok = false; continue; }
        };
        let compact: CompactEvidence = match serde_json::from_str(&compact_data) {
            Ok(c) => c,
            Err(e) => { eprintln!("parse {}: {e}", p.display()); ok = false; continue; }
        };
        let pi = p.with_file_name(
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .replace(".pci.json", ".pi.json"),
        );
        let orig_data = match fs::read_to_string(&pi) {
            Ok(d) => d,
            Err(e) => { eprintln!("read {}: {e}", pi.display()); ok = false; continue; }
        };
        let original: Vec<FrameSnapshot> = match serde_json::from_str(&orig_data) {
            Ok(f) => f,
            Err(e) => { eprintln!("parse {}: {e}", pi.display()); ok = false; continue; }
        };
        let decomp = compact_evidence::compact_to_frames(&compact);
        if let Some(diff) = first_diff(&original, &decomp) {
            eprintln!("FAIL {:.45}: {}", p.file_name().unwrap().to_string_lossy(), diff.message);
            ok = false;
        } else {
            let o = fs::metadata(&pi).map(|m| m.len()).unwrap_or(0);
            let c = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let pct = if o > 0 { 100 - (100 * c / o) } else { 0 };
            println!("OK {:.45} {:>8}B -> {:>8}B ({}%)",
                     p.file_name().unwrap().to_string_lossy(), o, c, pct);
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}