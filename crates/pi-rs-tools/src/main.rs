//! A.3 source-language gate binary.
//!
//! Subcommands:
//! - `scan --root DIR`  — list all tracked foreign-language files (diagnostic).
//! - `update-manifests --root DIR` — rewrite tests/source-language/{allowlist,legacy}.json
//!   from the current tracked footprint (explicit opt-in; reviewed).
//! - `check --root DIR` — fail if any tracked foreign-language file is not
//!   allowed (grandfathered legacy or allowlisted JS). This is the flake check.

use std::path::PathBuf;
use std::process::ExitCode;

use pi_rs_tools::gate::{Flag, Gate};
use pi_rs_tools::manifest;

const USAGE: &str = "\
pi-rs-tools source-language gate (A.3)

usage: pi-rs-tools gate <command> [options]

commands:
  scan --root DIR             list every tracked foreign-language file
  update-manifests --root DIR rewrite allowlist.json + legacy.json from the
                              current tracked footprint (explicit, reviewed opt-in)
  check --root DIR            fail if tracked foreign-language files are not
                              allowed; print violations (default to repo root)
";

fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!(
                "cannot derive workspace root from CARGO_MANIFEST_DIR {}",
                manifest_dir.display()
            )
        })
}

enum Command {
    Scan(PathBuf),
    UpdateManifests(PathBuf),
    Check(PathBuf),
}

fn parse_args() -> Result<Command, String> {
    let mut args = std::env::args_os().skip(2);
    let Some(cmd) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let cmd = cmd.to_string_lossy().into_owned();
    let mut root = None;
    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy().into_owned();
        if arg == "--root" {
            if let Some(next) = args.next() {
                root = Some(PathBuf::from(next.to_string_lossy().into_owned()));
            } else {
                return Err("--root requires a path".to_owned());
            }
        } else {
            return Err(format!("unexpected argument {arg:?}\n{USAGE}"));
        }
    }
    let root = root
        .map(Ok)
        .unwrap_or_else(repo_root)?;
    match cmd.as_str() {
        "scan" => Ok(Command::Scan(root)),
        "update-manifests" => Ok(Command::UpdateManifests(root)),
        "check" => Ok(Command::Check(root)),
        other => Err(format!("unknown command {other:?}\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    let command = match parse_args() {
        Ok(c) => c,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("source-language gate: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let root = match &command {
        Command::Scan(root) | Command::UpdateManifests(root) | Command::Check(root) => root.clone(),
    };
    let files = manifest::tracked_files(&root)?;
    // Build the list of (rel, first_line) so the gate can sniff shebangs.
    let scanned: Vec<(String, Option<String>)> = files
        .into_iter()
        .map(|rel| {
            let line = manifest_first_line(&root, &rel);
            (rel, line)
        })
        .collect();

    match &command {
        Command::Scan(_) => {
            // Diagnostic: print foreign-language files with their language.
            for (rel, line) in &scanned {
                let lang = pi_rs_tools::gate::detect(std::path::Path::new(rel), line.as_deref());
                if let Some(lang) = lang {
                    println!("{}\t{}", lang.label(), rel);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::UpdateManifests(_) => {
            let (allow, legacy) = manifest::write_manifests(&root)?;
            println!("wrote {}", allow.display());
            println!("wrote {}", legacy.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Check(_) => {
            let (js_allowlist, legacy) = manifest::load_manifests(&root)?;
            let gate = Gate::new(js_allowlist, legacy);
            let files_ref: Vec<(&str, Option<&str>)> = scanned
                .iter()
                .map(|(rel, line)| (rel.as_str(), line.as_deref()))
                .collect();
            let flags = gate.violations(files_ref);
            if flags.is_empty() {
                println!("source-language gate passed: no unallowed foreign-language files");
                Ok(ExitCode::SUCCESS)
            } else {
                report_flags(&flags);
                eprintln!(
                    "source-language gate FAILED: {} unallowed foreign-language file(s).",
                    flags.len()
                );
                Ok(ExitCode::FAILURE)
            }
        }
    }
}

fn manifest_first_line(root: &std::path::Path, rel: &str) -> Option<String> {
    let path = root.join(rel);
    let bytes = std::fs::read(&path).ok()?;
    let end = bytes.iter().position(|&b| b == b'\n').unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn report_flags(flags: &[Flag]) {
    for flag in flags {
        println!(
            "  {}  {}  ({})",
            flag.path,
            flag.language.label(),
            flag.reason
        );
    }
}
