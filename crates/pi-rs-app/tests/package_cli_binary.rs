#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// PLAN 9.7 package CLI end-to-end: run the real `pi` binary over the hermetic
// package CLI surface and compare exit code / stdout / stderr against the
// Pi-generated oracle in tests/package-cli-parity/oracle.json. This proves the
// dispatch wiring (raw argv before parseArgs) matches Pi, not just the parse
// function. Non-handled cases (e.g. "echo hi") fall through to the normal
// parser path and are covered by the parser-level test + args-parity suite.
use serde_json::Value;
use std::process::{Command, Stdio};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/package-cli-parity/oracle.json"
    ))
    .unwrap()
}

fn run_pi(argv: &[String]) -> (i32, String, String) {
    let exe = std::env::var("PI_TEST_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_pi").to_owned());
    let mut child = Command::new(&exe)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pi");
    let mut stdout = String::new();
    let mut stderr = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    (status.code().unwrap_or(-1), stdout, stderr)
}

#[test]
fn package_cli_binary_matches_pi_oracle() {
    let oracle = fixture();
    let cases = oracle["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    let mut checked = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let handled = case["handled"].as_bool().unwrap();
        if !handled {
            // Falls through to the normal parser; covered at parser level.
            continue;
        }
        let argv: Vec<String> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let expected_exit = case["exitCode"].as_i64().unwrap();
        let expected_stdout = case["stdout"].as_str().unwrap();
        let expected_stderr = case["stderr"].as_str().unwrap();

        let (code, stdout, stderr) = run_pi(&argv);
        assert_eq!(
            i64::from(code),
            expected_exit,
            "{name}: exit code mismatch for {argv:?}"
        );
        assert_eq!(
            &stdout, expected_stdout,
            "{name}: stdout mismatch for {argv:?}"
        );
        assert_eq!(
            &stderr, expected_stderr,
            "{name}: stderr mismatch for {argv:?}"
        );
        checked += 1;
    }
    assert!(checked >= 20, "compared {checked} handled cases end-to-end");
}
