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

top-level:
  model-catalog {selftest|update}...  Pi model-catalog owner (A.3)
  construction-inventory [--check] [--print-extracted] [--root DIR]
                              first-party construction inventory owner (A.3)
  construction-inventory selftest [--root DIR]
                              offline negative-control self-test (A.3)
  dogfood-oracle {check,selftest,generate} [--source PATH] [--root DIR]
                              pinned pi-flake dogfood fixture contract (A.3)
  final-parity-audit {check,selftest,generate} [--root DIR]
                              closed Pi v0.79.0 final surface audit (A.3)
  extension-inventory {check,generate,print-extracted,selftest} [--root DIR]
                              closed Pi v0.79.0 extension-surface inventory (A.3)
  external-extension-inventory {check,generate,print-extracted,selftest} [--root DIR]
                              pinned pi-flake external-extension inventory (A.3)
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
    ModelCatalog(Vec<String>),
    ConstructionInventory {
        root: PathBuf,
        check: bool,
        print_extracted: bool,
    },
    ConstructionInventorySelftest { root: PathBuf },
    DogfoodOracle {
        root: PathBuf,
        action: DogfoodAction,
        source: Option<PathBuf>,
    },
    FinalParityAudit {
        root: PathBuf,
        action: AuditAction,
    },
    ExtensionInventory {
        root: PathBuf,
        check: bool,
        print_extracted: bool,
    },
    ExtensionInventorySelftest { root: PathBuf },
    ExternalExtensionInventory {
        root: PathBuf,
        check: bool,
        print_extracted: bool,
    },
    ExternalExtensionInventorySelftest { root: PathBuf },
}

enum AuditAction {
    Check,
    Generate,
    Selftest,
}

enum DogfoodAction {
    Check,
    Generate,
    Selftest,
}

fn parse_args() -> Result<Command, String> {
    let mut args = std::env::args_os().skip(1);
    let Some(top) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let top = top.to_string_lossy().into_owned();
    if top == "model-catalog" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        return Ok(Command::ModelCatalog(rest));
    }
    if top == "construction-inventory" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        return parse_construction_inventory(rest);
    }
    if top == "dogfood-oracle" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        return parse_dogfood_oracle(rest);
    }
    if top == "final-parity-audit" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        return parse_final_parity_audit(rest);
    }
    if top == "extension-inventory" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        return parse_extension_inventory(rest);
    }
    if top == "external-extension-inventory" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        return parse_external_extension_inventory(rest);
    }
    if top != "gate" {
        return Err(format!("unknown top-level subcommand {top:?}\n{USAGE}"));
    }
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

fn parse_construction_inventory(rest: Vec<String>) -> Result<Command, String> {
    let mut root = None;
    let mut check = false;
    let mut print_extracted = false;
    let mut selftest = false;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--print-extracted" => print_extracted = true,
            "selftest" => selftest = true,
            "--root" => {
                root = it.next();
                if root.is_none() {
                    return Err("construction-inventory --root requires a path".to_owned());
                }
            }
            other => return Err(format!("unexpected argument {other:?} for construction-inventory")),
        }
    }
    if selftest {
        return Ok(Command::ConstructionInventorySelftest {
            root: root.map(PathBuf::from).unwrap_or(default_root()),
        });
    }
    Ok(Command::ConstructionInventory {
        root: root.map(PathBuf::from).unwrap_or(default_root()),
        check,
        print_extracted,
    })
}

fn parse_external_extension_inventory(rest: Vec<String>) -> Result<Command, String> {
    let mut check = false;
    let mut print_extracted = false;
    let mut selftest = false;
    let mut root: Option<PathBuf> = None;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "check" => check = true,
            "generate" => check = false,
            "print-extracted" => print_extracted = true,
            "selftest" => selftest = true,
            "--root" => {
                if let Some(p) = it.next() {
                    root = Some(PathBuf::from(p));
                } else {
                    return Err("external-extension-inventory --root requires a path".to_owned());
                }
            }
            other => return Err(format!("unexpected argument {other:?} for external-extension-inventory")),
        }
    }
    if selftest {
        return Ok(Command::ExternalExtensionInventorySelftest {
            root: root.unwrap_or_else(default_root),
        });
    }
    Ok(Command::ExternalExtensionInventory {
        root: root.unwrap_or_else(default_root),
        check,
        print_extracted,
    })
}

fn parse_extension_inventory(rest: Vec<String>) -> Result<Command, String> {
    let mut check = false;
    let mut print_extracted = false;
    let mut selftest = false;
    let mut root: Option<PathBuf> = None;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "check" => check = true,
            "generate" => check = false,
            "print-extracted" => print_extracted = true,
            "selftest" => selftest = true,
            "--root" => {
                if let Some(p) = it.next() {
                    root = Some(PathBuf::from(p));
                } else {
                    return Err("extension-inventory --root requires a path".to_owned());
                }
            }
            other => return Err(format!("unexpected argument {other:?} for extension-inventory")),
        }
    }
    if selftest {
        return Ok(Command::ExtensionInventorySelftest {
            root: root.unwrap_or_else(default_root),
        });
    }
    Ok(Command::ExtensionInventory {
        root: root.unwrap_or_else(default_root),
        check,
        print_extracted,
    })
}

fn parse_final_parity_audit(rest: Vec<String>) -> Result<Command, String> {
    let mut action: Option<AuditAction> = None;
    let mut root: Option<PathBuf> = None;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "check" => action = Some(AuditAction::Check),
            "generate" => action = Some(AuditAction::Generate),
            "selftest" => action = Some(AuditAction::Selftest),
            "--root" => {
                if let Some(p) = it.next() {
                    root = Some(PathBuf::from(p));
                } else {
                    return Err("final-parity-audit --root requires a path".to_owned());
                }
            }
            other => return Err(format!("unexpected argument {other:?} for final-parity-audit")),
        }
    }
    let action = action.ok_or("final-parity-audit requires one of check|generate|selftest")?;
    Ok(Command::FinalParityAudit {
        root: root.unwrap_or_else(default_root),
        action,
    })
}

fn parse_dogfood_oracle(rest: Vec<String>) -> Result<Command, String> {
    let mut action: Option<DogfoodAction> = None;
    let mut root: Option<PathBuf> = None;
    let mut source: Option<PathBuf> = None;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "check" => action = Some(DogfoodAction::Check),
            "generate" => action = Some(DogfoodAction::Generate),
            "selftest" => action = Some(DogfoodAction::Selftest),
            "--root" => {
                if let Some(p) = it.next() {
                    root = Some(PathBuf::from(p));
                } else {
                    return Err("dogfood-oracle --root requires a path".to_owned());
                }
            }
            "--source" => {
                if let Some(p) = it.next() {
                    source = Some(PathBuf::from(p));
                } else {
                    return Err("dogfood-oracle --source requires a path".to_owned());
                }
            }
            other => return Err(format!("unexpected argument {other:?} for dogfood-oracle")),
        }
    }
    let action = action.ok_or("dogfood-oracle requires one of check|generate|selftest")?;
    Ok(Command::DogfoodOracle {
        root: root.unwrap_or_else(default_root),
        action,
        source,
    })
}

fn default_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
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
    if let Command::ModelCatalog(args) = &command {
        return run_model_catalog(args);
    }
    if let Command::ConstructionInventory { root, check, print_extracted } = &command {
        pi_rs_tools::construction_inventory::run(root, *check, *print_extracted)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::ConstructionInventorySelftest { root } = &command {
        pi_rs_tools::construction_inventory_selftest::run_root(root)?;
        println!("construction-inventory selftest passed");
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::DogfoodOracle { root, action, source } = &command {
        let opts = pi_rs_tools::dogfood_oracle::Options {
            root,
            check: matches!(action, DogfoodAction::Check | DogfoodAction::Selftest),
            source: source.as_deref(),
            self_test: matches!(action, DogfoodAction::Selftest),
        };
        pi_rs_tools::dogfood_oracle::run(&opts)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::FinalParityAudit { root, action } = &command {
        let check = matches!(action, AuditAction::Check | AuditAction::Selftest);
        let self_test = matches!(action, AuditAction::Selftest);
        pi_rs_tools::final_parity_audit::run(root, check, self_test)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::ExtensionInventory { root, check, print_extracted } = &command {
        pi_rs_tools::extension_inventory::run(root, *check, *print_extracted)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::ExtensionInventorySelftest { root } = &command {
        pi_rs_tools::extension_inventory_selftest::run_root(root)?;
        println!("extension inventory fail-closed self-tests passed");
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::ExternalExtensionInventory { root, check, print_extracted } = &command {
        let base = root.join("tests/external-extension-inventory");
        pi_rs_tools::external_extension_inventory::run(root, &base, *check, *print_extracted)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::ExternalExtensionInventorySelftest { root } = &command {
        pi_rs_tools::external_extension_inventory_selftest::run_root(root)?;
        println!("external extension inventory fail-closed self-tests passed");
        return Ok(ExitCode::SUCCESS);
    }
    let root = match &command {
        Command::Scan(root) | Command::UpdateManifests(root) | Command::Check(root) => root.clone(),
        Command::ModelCatalog(_)
        | Command::ConstructionInventory { .. }
        | Command::ConstructionInventorySelftest { .. }
        | Command::DogfoodOracle { .. }
        | Command::FinalParityAudit { .. }
        | Command::ExtensionInventory { .. }
        | Command::ExtensionInventorySelftest { .. }
        | Command::ExternalExtensionInventory { .. }
        | Command::ExternalExtensionInventorySelftest { .. } => unreachable!(),
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
        Command::ModelCatalog(_)
        | Command::ConstructionInventory { .. }
        | Command::ConstructionInventorySelftest { .. }
        | Command::DogfoodOracle { .. }
        | Command::FinalParityAudit { .. }
        | Command::ExtensionInventory { .. }
        | Command::ExtensionInventorySelftest { .. }
        | Command::ExternalExtensionInventory { .. }
        | Command::ExternalExtensionInventorySelftest { .. } => unreachable!(),
    }
}

const MODEL_CATALOG_USAGE: &str = "\
pi-rs-tools model-catalog — normalize Pi's model catalog (A.3 Rust owner)

usage: pi-rs-tools model-catalog selftest
       pi-rs-tools model-catalog update --source PATH --overrides PATH \\
                 --output PATH --provenance PATH [--revision REV] \\
                 [--summary-output PATH] [--source-kind published|local] \\
                 [--source-url URL] [--source-path SP]
";

fn run_model_catalog(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    use pi_rs_tools::model_catalog;
    use std::path::PathBuf;

    if args.is_empty() {
        eprint!("{MODEL_CATALOG_USAGE}");
        return Ok(ExitCode::from(2));
    }
    match args[0].as_str() {
        "selftest" => {
            // Offline fixture selftest. `--root DIR` points at the repository
            // containing tests/model-catalog-update/ fixtures (the flake passes
            // ${self}); otherwise it resolves from CARGO_MANIFEST_DIR.
            let mut root_arg: Option<String> = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--root" => {
                        root_arg = it.next().cloned();
                        if root_arg.is_none() {
                            eprintln!("selftest --root requires a path");
                            return Ok(ExitCode::from(2));
                        }
                    }
                    other => {
                        eprintln!("unknown selftest flag {other:?}");
                        return Ok(ExitCode::from(2));
                    }
                }
            }
            match root_arg {
                Some(r) => pi_rs_tools::selftest::run_root(std::path::Path::new(&r))?,
                None => pi_rs_tools::selftest::run()?,
            }
            Ok(ExitCode::SUCCESS)
        }
        "update" => {
            let mut source = None;
            let mut overrides = None;
            let mut output = None;
            let mut provenance = None;
            let mut summary_output = None;
            let mut revision = String::new();
            let mut source_kind = "local".to_owned();
            let mut source_url = String::new();
            let mut source_path = String::new();
            let mut repository = "https://github.com/earendil-works/pi.git".to_owned();
            let mut args_iter = args[1..].iter();
            while let Some(a) = args_iter.next() {
                let take_value = |a: &str, args_iter: &mut std::slice::Iter<'_, String>| -> Option<String> {
                    if let Some((_, v)) = a.split_once('=') {
                        Some(v.to_owned())
                    } else {
                        args_iter.next().cloned()
                    }
                };
                match a.as_str() {
                    "--source" => source = take_value(a, &mut args_iter),
                    "--overrides" => overrides = take_value(a, &mut args_iter),
                    "--output" => output = take_value(a, &mut args_iter),
                    "--provenance" => provenance = take_value(a, &mut args_iter),
                    "--summary-output" => summary_output = take_value(a, &mut args_iter),
                    "--revision" => revision = take_value(a, &mut args_iter).unwrap_or(revision),
                    "--source-kind" => source_kind = take_value(a, &mut args_iter).unwrap_or(source_kind),
                    "--source-url" => source_url = take_value(a, &mut args_iter).unwrap_or(source_url),
                    "--source-path" => source_path = take_value(a, &mut args_iter).unwrap_or(source_path),
                    "--repository" => repository = take_value(a, &mut args_iter).unwrap_or(repository),
                    other => {
                        eprintln!("unknown model-catalog update flag {other:?}");
                        return Ok(ExitCode::from(2));
                    }
                }
            }
            let overrides = overrides.ok_or("model-catalog update: --overrides required")?;
            let output = output.ok_or("model-catalog update: --output required")?;
            let provenance = provenance.ok_or("model-catalog update: --provenance required")?;
            let summary_path = summary_output.map(PathBuf::from);

            use pi_rs_tools::acquire;
            // Pick the acquisition path. `--source` is a local file/checkout;
            // otherwise source_kind decides published-catalog vs git.
            // Remote/git acquisitions materialize into a TempDir that must stay
            // alive until after normalize reads the file, so we hold them here.
            let mut _temps: Vec<pi_rs_tools::acquire::TempDir> = Vec::new();
            let (file, source_desc, revision) = if let Some(src) = source {
                let _ = source_kind;
                let acq = acquire::from_source(std::path::Path::new(&src), &source_path, Some(&revision));
                let desc = serde_json::json!({
                    "kind": "local",
                    "revision": acq.revision,
                    "path": source_path,
                });
                (acq.file, desc, acq.revision)
            } else if source_kind == "published-catalog" {
                let url = if source_url.is_empty() { "https://pi.dev/api/models" } else { &source_url };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("tokio: {e}"))?;
                let (tmp, _value, rev) = runtime.block_on(acquire::from_catalog(url))?;
                let desc = serde_json::json!({
                    "kind": "published-catalog",
                    "url": url,
                    "revision": rev,
                });
                let file = tmp.path().join("catalog.json");
                _temps.push(tmp);
                (file, desc, rev)
            } else {
                // git clone path (`--local` in the old script).
                let rev = if revision.is_empty() { "main" } else { &revision };
                let (tmp, file, head) = acquire::from_git(&repository, rev, &source_path)?;
                let desc = serde_json::json!({
                    "kind": "git",
                    "revision": head,
                    "path": source_path,
                });
                _temps.push(tmp);
                (file, desc, head)
            };

            let opts = model_catalog::Options {
                source: &file,
                overrides: std::path::Path::new(&overrides),
                output: std::path::Path::new(&output),
                provenance: std::path::Path::new(&provenance),
                summary_output: summary_path.as_deref(),
                revision,
                source_desc,
                remote: source_kind != "local",
            };
            let result = model_catalog::run_normalize(&opts)?;
            println!("{}", result.report.trim_end());
            Ok(ExitCode::SUCCESS)
        }
        other => {
            eprintln!("unknown model-catalog subcommand {other:?}");
            eprint!("{MODEL_CATALOG_USAGE}");
            Ok(ExitCode::from(2))
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
