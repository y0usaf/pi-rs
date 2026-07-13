#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::Path;
use std::process::{Command, Output};

fn write(path: &Path, source: &str) {
    std::fs::write(path, source).expect("write Lua package");
}

fn invoke(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pi"))
        .current_dir(root)
        .args(arguments)
        .output()
        .expect("run pi")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8 stderr")
}

#[test]
fn file_packages_load_in_declared_order_from_the_selected_root() {
    let scratch = tempfile::tempdir().unwrap();
    let selected = scratch.path().join("selected");
    let elsewhere = scratch.path().join("elsewhere");
    std::fs::create_dir_all(&selected).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    write(
        &selected.join("dependency.lua"),
        r#"
local k = (...).kernel.v1
k.module.define({
  name="ordering", version="1", dependencies={},
  factory=function() return { value="first" } end,
})
"#,
    );
    write(
        &selected.join("application.lua"),
        r#"
local k = (...).kernel.v1
local dependency = k.module.require("ordering", "1")
k.root({
  kind="application", id="scratch", active=true, priority=0,
  dispatch=function(snapshot)
    local arguments = {}
    for index = 1, #snapshot.event.arguments do
      arguments[index] = snapshot.event.arguments[index]
    end
    local packages = {}
    for index = 1, #snapshot.context.packages do
      packages[index] = snapshot.context.packages[index]
    end
    k.action("observed", {
      dependency=dependency.value,
      kind=snapshot.event.kind,
      arguments=arguments,
      root=snapshot.context.root,
      packages=packages,
    })
  end,
})
"#,
    );

    let output = invoke(
        &elsewhere,
        &[
            "--root",
            selected.to_str().unwrap(),
            "--package",
            "dependency.lua",
            "--package",
            "application.lua",
            "--",
            "alpha",
            "beta",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["actions"][0]["kind"], "observed");
    assert_eq!(value["actions"][0]["payload"]["dependency"], "first");
    assert_eq!(value["actions"][0]["payload"]["kind"], "startup");
    assert_eq!(
        value["actions"][0]["payload"]["arguments"],
        serde_json::json!(["alpha", "beta"])
    );
    assert_eq!(
        value["actions"][0]["payload"]["root"],
        std::fs::canonicalize(&selected)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    let packages = value["actions"][0]["payload"]["packages"]
        .as_array()
        .unwrap();
    assert!(packages[0].as_str().unwrap().ends_with("dependency.lua"));
    assert!(packages[1].as_str().unwrap().ends_with("application.lua"));
}

#[test]
fn zero_package_launch_has_no_embedded_application_policy() {
    let scratch = tempfile::tempdir().unwrap();
    let output = invoke(scratch.path(), &[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "pi: application root dispatch failed: no active kernel root for 'application'\n"
    );
}

#[test]
fn package_file_failures_are_nonzero_and_identify_the_selected_input() {
    let scratch = tempfile::tempdir().unwrap();
    let directory = scratch.path().join("directory.lua");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(scratch.path().join("unreadable.lua"), [0xff]).unwrap();
    write(&scratch.path().join("syntax.lua"), "local =");
    write(
        &scratch.path().join("failure.lua"),
        "error('package exploded')",
    );

    let cases = [
        ("absent.lua", "package 1 is absent"),
        (
            "directory.lua",
            "package 1 is unreadable because it is not a regular file",
        ),
        ("unreadable.lua", "package 1"),
        ("syntax.lua", "failed to load package 1"),
        ("failure.lua", "package exploded"),
    ];
    for (path, expected) in cases {
        let output = invoke(scratch.path(), &["--package", path]);
        assert!(!output.status.success(), "{path}");
        assert!(output.stdout.is_empty(), "{path}");
        let diagnostic = stderr(&output);
        assert!(diagnostic.contains(expected), "{path}: {diagnostic}");
        assert!(diagnostic.contains(path), "{path}: {diagnostic}");
        if path == "unreadable.lua" {
            assert!(diagnostic.contains("is unreadable"), "{diagnostic}");
            assert!(diagnostic.contains("valid UTF-8"), "{diagnostic}");
        }
    }
}

#[test]
fn conflicting_or_missing_application_roots_fail_at_dispatch() {
    let scratch = tempfile::tempdir().unwrap();
    write(
        &scratch.path().join("one.lua"),
        "local k=(...).kernel.v1; k.root({kind='application', id='one', priority=4, dispatch=function() end})",
    );
    write(
        &scratch.path().join("two.lua"),
        "local k=(...).kernel.v1; k.root({kind='application', id='two', priority=4, dispatch=function() end})",
    );
    write(
        &scratch.path().join("rootless.lua"),
        "local k=(...).kernel.v1; k.module.define({name='rootless', version='1', dependencies={}, factory=function() return {} end})",
    );

    let conflict = invoke(
        scratch.path(),
        &["--package", "one.lua", "--package", "two.lua"],
    );
    assert!(!conflict.status.success());
    let diagnostic = stderr(&conflict);
    assert!(diagnostic.contains("application root dispatch failed: declaration conflict"));
    assert!(diagnostic.contains("one.lua:one"));
    assert!(diagnostic.contains("two.lua:two"));

    let missing = invoke(scratch.path(), &["--package", "rootless.lua"]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("no active kernel root for 'application'"));
}
