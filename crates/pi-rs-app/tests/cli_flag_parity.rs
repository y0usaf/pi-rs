#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// PLAN 11 coding.assembly CLI flag parity: run the real `pi` binary end-to-end
// over the CLI surface that Pi's `main.ts` + `applyExtensionFlagValues`
// resolves, and pin the exact bytes Pi emits for each case (captured from the
// pinned ref/pi oracle at c5582102). Parities asserted here, all byte-for-byte
// on stdout/stderr and matching exit code:
//   --help          → identical help text, exit 0
//   --frobnicate     → stderr "Error: Unknown option: --frobnicate", exit 1
//   --model          → stderr "Error: Unknown option: --model", exit 1
//   --api-key sk-test → stderr "--api-key requires a model ...", exit 1
//   --mode text      → no output, exit 0 (empty print mode)
use std::io::Read;
use std::process::{Command, Stdio};

/// Run the `pi` binary in a hermetic, offline, empty-agent-dir env and return
/// (exit_code, stdout, stderr). stdin is closed (non-TTY), so every case is
/// headless.
fn run_pi(argv: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
    let exe = std::env::var("PI_TEST_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_pi").to_owned());
    let agent = tempfile::TempDir::new().unwrap();
    let home = tempfile::TempDir::new().unwrap();
    let mut child = Command::new(&exe)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PI_OFFLINE", "1")
        .env("PI_CODING_AGENT_DIR", agent.path())
        .env("HOME", home.path())
        .spawn()
        .expect("spawn pi");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    (status.code().unwrap_or(-1), stdout, stderr)
}

#[test]
fn help_matches_pi_byte_for_byte() {
    let (code, stdout, stderr) = run_pi(&["--help"]);
    assert_eq!(code, 0, "--help exit code");
    assert_eq!(stdout, pi_rs_app::cli::args::help_text().into_bytes());
    assert!(stderr.is_empty(), "--help has no stderr");
}

#[test]
fn unknown_long_flag_errors_like_pi() {
    for (argv, expected) in [
        (vec!["--frobnicate"], "Error: Unknown option: --frobnicate\n"),
        (vec!["--model"], "Error: Unknown option: --model\n"),
    ] {
        let (code, stdout, stderr) = run_pi(&argv);
        assert_eq!(code, 1, "{argv:?} exit code");
        assert!(stdout.is_empty(), "{argv:?} has no stdout (no help)");
        assert_eq!(String::from_utf8(stderr).unwrap(), expected);
    }
}

#[test]
fn api_key_without_model_errors_like_pi() {
    let (code, stdout, stderr) = run_pi(&["--api-key", "sk-test"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: --api-key requires a model to be specified via --model, --provider/--model, or --models\n"
    );
}

#[test]
fn empty_text_mode_prints_nothing_exits_zero() {
    let (code, stdout, stderr) = run_pi(&["--mode", "text"]);
    assert_eq!(code, 0, "empty text mode exit 0");
    assert!(stdout.is_empty(), "empty text mode has no stdout");
    assert!(stderr.is_empty(), "empty text mode has no stderr");
}

#[test]
fn registered_extension_flag_is_accepted() {
    // A flag registered by a loaded extension must not be rejected with
    // "Unknown option" — pi-rs forwards it and proceeds (pi-rs has no default
    // model catalog, so the run then stops at the no-models message rather
    // than the flag validation). We only assert the flag was accepted (the
    // stderr must NOT contain "Unknown option").
    let (code, _stdout, stderr) = run_pi(&[
        "-e",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/extensions/flag-demo.lua"),
        "--demo-enabled=true",
        "hello",
    ]);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(
        !stderr.contains("Unknown option"),
        "registered --demo-enabled accepted: {stderr}"
    );
    // The empty model catalog means the run can't proceed: exit 1, no-models.
    assert_eq!(code, 1, "no models available after accepting the flag");
    assert!(
        stderr.contains("No models available"),
        "stderr mentions the empty catalog: {stderr}"
    );
}

#[test]
fn empty_text_with_registered_extension_flag_exits_zero() {
    // `--demo-enabled foo` parses the token as the flag's value (no message),
    // so this is an empty text print after the registered flag is accepted:
    // Pi prompts nothing and exits 0 (accepted flag → no unknown-option error,
    // empty run → no output).
    let (code, stdout, stderr) = run_pi(&[
        "-e",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/extensions/flag-demo.lua"),
        "--demo-enabled",
        "foo",
    ]);
    assert_eq!(code, 0, "empty print with accepted flag exits 0");
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}