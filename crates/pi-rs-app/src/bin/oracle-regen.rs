#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regenerate `tests/<parity>/oracle.json` from the hash-pinned vendored Pi
//! (`ref/pi`) — the Rust owner of the opt-in oracle workflow (PLAN A.3).
//!
//! Each driver (see `drivers.rs`, ported from the deleted `gen-oracle.ts`
//! files) runs Pi's real stream/agent/extension code against scripted inputs
//! and prints the canonical oracle JSON. The driver is materialized into the
//! parity directory (same relative depth, so `../../ref/pi` imports resolve
//! exactly as before), executed with the pinned node/bun runtime, and removed.
//!
//! Usage:
//!   cargo run -p pi-rs-app --bin oracle-regen <parity-name>
//!   cargo run -p pi-rs-app --bin oracle-regen --list
//!
//! Requires: `ref/pi/node_modules` (npm ci) and `nix` on PATH (pinned
//! node/bun via `nix shell nixpkgs#nodejs_22` / `nixpkgs#bun`).

#[path = "oracle-regen/drivers.rs"]
mod drivers;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn usage() -> ! {
    eprintln!(
        "usage: oracle-regen <parity-name>   regenerate tests/<parity>/oracle.json\n       \
         oracle-regen --list                 list supported parities\n\n\
         Opt-in Pi differential regeneration; the checked oracle.json fixtures remain\n\
         canonical for normal verification. Requires ref/pi/node_modules (npm ci) and\n\
         nix on PATH for the pinned node/bun runtime."
    );
    std::process::exit(2);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first() else {
        usage();
    };
    if name == "--list" {
        for (parity, _, _, runner, _) in drivers::PARITIES {
            println!("{parity} ({runner})");
        }
        return ExitCode::SUCCESS;
    }
    let Some(spec) = drivers::PARITIES.iter().find(|(parity, _, _, _, _)| parity == name) else {
        eprintln!("oracle-regen: unknown parity {name}");
        eprintln!("known parities:");
        for (parity, _, _, _, _) in drivers::PARITIES {
            eprintln!("  {parity}");
        }
        return ExitCode::FAILURE;
    };
    match regen(&repo_root(), spec) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oracle-regen: {error}");
            ExitCode::FAILURE
        }
    }
}

fn regen(root: &Path, spec: &(&str, &str, Option<&str>, &str, &[&str])) -> Result<(), String> {
    let (name, driver, input, runner, unset) = *spec;
    let parity_dir = root.join("tests").join(format!("{name}-parity"));
    if !parity_dir.is_dir() {
        return Err(format!("parity dir not found: {}", parity_dir.display()));
    }
    let tsx = root.join("ref/pi/node_modules/.bin/tsx");
    if !tsx.exists() {
        return Err(format!(
            "ref/pi node_modules missing ({}) — bootstrap it with:\n  \
             nix shell nixpkgs#nodejs_22 --command sh -c 'cd ref/pi && corepack npm ci --ignore-scripts'",
            tsx.display()
        ));
    }

    let driver_path = parity_dir.join(".oracle-regen-driver.ts");
    fs::write(&driver_path, driver)
        .map_err(|error| format!("cannot write driver {}: {error}", driver_path.display()))?;
    let stdout = match run_driver(root, runner, &parity_dir, &driver_path, input, unset) {
        Ok(stdout) => stdout,
        Err(error) => {
            let _ = fs::remove_file(&driver_path);
            return Err(error);
        }
    };
    let _ = fs::remove_file(&driver_path);

    let oracle_path = parity_dir.join("oracle.json");
    fs::write(&oracle_path, stdout)
        .map_err(|error| format!("cannot write oracle {}: {error}", oracle_path.display()))?;
    println!("wrote {}", oracle_path.display());
    Ok(())
}

fn run_driver(
    root: &Path,
    runner: &str,
    parity_dir: &Path,
    driver: &Path,
    input: Option<&str>,
    unset: &[&str],
) -> Result<String, String> {
    let mut command = Command::new("nix");
    match runner {
        "tsx" => {
            command.args(["shell", "nixpkgs#nodejs_22", "--command"]);
            let tsx = root.join("ref/pi/node_modules/.bin/tsx");
            let tsconfig = root.join("ref/pi/tsconfig.json");
            command.arg(&tsx);
            command.arg("--tsconfig");
            command.arg(&tsconfig);
        }
        "bun" => {
            command.args(["shell", "nixpkgs#bun", "--command"]);
            command.arg("bun");
        }
        other => return Err(format!("unknown runner {other}")),
    }
    command.arg(driver);
    if let Some(input) = input {
        command.arg(parity_dir.join(input));
    }
    // Match the deleted scripts/*-oracle wrappers: the driver must not resolve
    // themes/assets through an ambient PI_PACKAGE_DIR (a nix-store pi install);
    // the hash-pinned ref/pi source tree is the only source of truth, and
    // FORCE_COLOR pins the output regardless of the ambient tty.
    command.env_remove("PI_PACKAGE_DIR");
    command.env("FORCE_COLOR", "3");
    for key in unset {
        command.env_remove(key);
    }
    command.current_dir(root);
    let output = command
        .output()
        .map_err(|error| format!("failed to spawn nix shell ({error}); nix must be on PATH"))?;
    if !output.status.success() {
        return Err(format!(
            "driver run failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("driver output is not UTF-8: {error}"))
}
