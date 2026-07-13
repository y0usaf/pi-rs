#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::process::Command;

#[test]
fn help_describes_only_generic_package_launching() {
    let output = Command::new(env!("CARGO_BIN_EXE_pi"))
        .arg("--help")
        .output()
        .expect("run pi --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("generic Lua application launcher"));
    assert!(help.contains("--package FILE"));
    for product_term in [
        "--login",
        "--model",
        "--session",
        "--resume",
        "--approve",
        "--extension",
    ] {
        assert!(!help.contains(product_term), "unexpected {product_term}");
    }
}
