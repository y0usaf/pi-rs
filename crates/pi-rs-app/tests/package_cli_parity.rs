#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// PLAN 9.7 package CLI differential: replay the Pi-generated oracle in
// tests/package-cli-parity/oracle.json (from Pi's real handlePackageCommand
// in package-manager-cli.ts, driven with Bun) through the pi-rs Rust port of
// the parse/help/early-error surface and compare handled / exitCode / stdout /
// stderr byte-for-byte.
use pi_rs_app::cli::packages::handle_package_command_hermetic;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/package-cli-parity/oracle.json"
    ))
    .unwrap()
}

#[test]
fn package_cli_hermetic_surface_matches_pi_oracle() {
    let oracle = fixture();
    let cases = oracle["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "oracle has no cases");
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let argv: Vec<String> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let expected_handled = case["handled"].as_bool().unwrap();
        let expected_exit: Option<i64> = case["exitCode"].as_i64();
        let expected_stdout = case["stdout"].as_str().unwrap();
        let expected_stderr = case["stderr"].as_str().unwrap();

        let result = handle_package_command_hermetic(&argv);
        match result {
            None => {
                assert!(!expected_handled, "{name}: pi handled but pi-rs did not");
                assert_eq!(expected_exit, Some(0), "{name}: handled=false exitCode");
                assert!(expected_stdout.is_empty(), "{name}");
                assert!(expected_stderr.is_empty(), "{name}");
            }
            Some((code, stdout, stderr)) => {
                assert!(expected_handled, "{name}: pi-rs handled but pi did not");
                // i32::MIN sentinel marks an out-of-scope (would-execute) case;
                // none are in this hermetic oracle.
                assert_ne!(code, std::i32::MIN, "{name}: unexpected execute case");
                assert_eq!(
                    i64::from(code),
                    expected_exit.unwrap_or(-1),
                    "{name}: exitCode"
                );
                assert_eq!(&stdout, expected_stdout, "{name}: stdout mismatch");
                assert_eq!(&stderr, expected_stderr, "{name}: stderr mismatch");
            }
        }
    }
}
