#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn benchmark_schema_self_test_is_offline_and_debug_safe() {
    let output = Command::new(env!("CARGO_BIN_EXE_performance-baseline"))
        .arg("--self-test")
        .output()
        .expect("run benchmark schema self-test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "pi-rs-performance-v1 self-test passed\n"
    );
}

#[test]
fn checked_reference_parameters_are_versioned() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/performance/reference-v1.json");
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("read benchmark config"))
            .expect("parse benchmark config");
    assert_eq!(value["schema"], "pi-rs-performance-v1");
    assert_eq!(value["startup_samples"], 30);
    assert_eq!(value["render_samples"], 5000);
}
