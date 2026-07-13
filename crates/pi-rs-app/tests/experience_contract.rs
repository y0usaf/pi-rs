#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/experience/canonical-v1.json")
}

#[test]
fn canonical_experience_is_offline_valid_and_byte_idempotent() {
    let output = Command::new(env!("CARGO_BIN_EXE_ui-diff"))
        .arg("--check")
        .arg(fixture())
        .output()
        .expect("run Rust fixture checker");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "experience fixture valid: 6 journeys, 20 steps\n"
    );
}

#[test]
fn negative_controls_report_the_first_cell_and_input_byte() {
    let output = Command::new(env!("CARGO_BIN_EXE_ui-diff"))
        .arg("--self-test")
        .arg(fixture())
        .output()
        .expect("run Rust fixture checker negative controls");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(
        "cell-negative: cell mismatch journey=\"startup\" step=\"startup\" row=0 column=0"
    ));
    assert!(stdout.contains(
        "input-negative: input mismatch journey=\"prompt-editing\" step=\"typed-wrap\" byte=0: expected 0x54, got 0xff"
    ));
}
