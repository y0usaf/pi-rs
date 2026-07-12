#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 9.1 slice: ordinary files enter the shipped product VM in resource
//! precedence order; async/failing initialization is isolated; translated
//! hello + permission-gate execute through product Lua composition.

use pi_rs_app::builtins::{CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_app::cli::extensions::load_product_extensions;
use pi_rs_host::{Host, HostConfig};

fn write(path: &std::path::Path, source: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

fn tool(name: &str, value: &str) -> String {
    "local pi = ...\npi.register_tool({name=__NAME__, execute=function() return {content={{type='text',text=__VALUE__}},details={}} end})"
        .replace("__NAME__", &serde_json::to_string(name).unwrap())
        .replace("__VALUE__", &serde_json::to_string(value).unwrap())
}

#[test]
fn product_loader_runs_tool_and_blocking_hook_with_isolated_failures() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent_dir = root.path().join("agent");
    let cli = root.path().join("cli-hello.lua");
    let configured_bad = root.path().join("configured-bad.lua");
    let configured_good = root.path().join("configured-good.lua");

    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/extensions/hello.lua"
        ),
        &cli,
    )
    .unwrap();
    std::fs::create_dir_all(cwd.join(".pi/extensions")).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/extensions/permission-gate.lua"
        ),
        cwd.join(".pi/extensions/permission-gate.lua"),
    )
    .unwrap();
    let command_marker = root.path().join("command-result.txt");
    write(
        &cwd.join(".pi/extensions/project.lua"),
        &format!(
            "{}\npi.register_command('mark',{{description='mark command',handler=function(args) pi.fs.write_file({}, args) end}})",
            tool("project-tool", "project"),
            serde_json::to_string(&command_marker.to_string_lossy()).unwrap(),
        ),
    );
    write(
        &agent_dir.join("extensions/global.lua"),
        &format!(
            "local pi = ...\npi.sleep(2)\n{}",
            tool("global-tool", "global")
        ),
    );
    write(
        &configured_bad,
        "local pi = ...\npi.register_tool({name='ghost',execute=function() end})\nerror('broken init')",
    );
    write(&configured_good, &tool("configured-tool", "configured"));

    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .unwrap();
    let embedded = host.load_embedded(&[
        pi_rs_agent::PACK,
        TOOLS_PACK,
        CODING_AGENT_PACK,
        INTERACTIVE_PACK,
    ]);
    assert!(embedded.errors.is_empty(), "{:?}", embedded.errors);
    let report = load_product_extensions(
        &host,
        &[
            configured_bad.to_string_lossy().into_owned(),
            configured_good.to_string_lossy().into_owned(),
        ],
        &[cli.to_string_lossy().into_owned()],
        &cwd.to_string_lossy(),
        &agent_dir.to_string_lossy(),
        true,
        false,
    );
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].error.contains("broken init"));

    let hello = host
        .call_command(
            "extension-vertical-slice",
            r#"{"tool":"hello","arguments":{"name":"Ada"}}"#,
        )
        .unwrap()
        .unwrap();
    assert_eq!(hello["result"]["content"][0]["text"], "Hello, Ada!");
    assert_eq!(
        hello["toolNames"],
        serde_json::json!([
            "read",
            "bash",
            "edit",
            "write",
            "hello",
            "project-tool",
            "global-tool",
            "configured-tool"
        ])
    );
    assert!(
        !hello["toolNames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "ghost")
    );

    let command = host
        .call_command(
            "interactive-submit-route",
            &serde_json::json!({ "texts": ["/mark command args"], "cwd": cwd }).to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(command_marker).unwrap(),
        "command args"
    );
    assert_eq!(command["trace"][0]["action"], "extension_command");
    assert_eq!(command["trace"][0]["handled"], true);
    assert!(
        !command["trace"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["action"] == "prompt")
    );

    let blocked = host
        .call_command(
            "extension-vertical-slice",
            r#"{"toolCall":{"name":"bash","arguments":{"command":"sudo true"}}}"#,
        )
        .unwrap()
        .unwrap();
    assert_eq!(blocked["hookResult"]["block"], true);
    assert_eq!(
        blocked["hookResult"]["reason"],
        "Dangerous command blocked (no UI for confirmation)"
    );
}

#[test]
fn no_extensions_and_untrusted_project_keep_only_explicit_cli_sources() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent_dir = root.path().join("agent");
    let cli = root.path().join("cli.lua");
    write(
        &cwd.join(".pi/extensions/project.lua"),
        &tool("project", "p"),
    );
    write(
        &agent_dir.join("extensions/global.lua"),
        &tool("global", "g"),
    );
    write(&cli, &tool("cli", "c"));

    let host = Host::new(HostConfig::default()).unwrap();
    let report = load_product_extensions(
        &host,
        &[],
        &[cli.to_string_lossy().into_owned()],
        &cwd.to_string_lossy(),
        &agent_dir.to_string_lossy(),
        false,
        true,
    );
    assert!(report.errors.is_empty());
    assert_eq!(host.tools().unwrap()[0].name, "cli");
}
