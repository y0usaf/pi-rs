//! WS1.2 acceptance: registries on the coroutine path.
//! - register_tool / register_command land in the per-extension registry
//! - host-side metadata mirror (tools/commands, functions stripped)
//! - spec resolution: first tool registration per name wins across
//!   extensions; command collisions get `name:N` invocation names
//! - tool execute and command handlers run under the watchdog and may
//!   await host futures
//! - the exerciser examples load and run through the public API

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig, HostError};

fn host_with_budget(ms: i64) -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: ms,
        ..HostConfig::default()
    })
    .expect("host starts")
}

fn host() -> Host {
    host_with_budget(5000)
}

#[test]
fn tool_registers_and_executes_on_coroutine_path() {
    let host = host_with_budget(100);
    host.load(
        "test://greet",
        r#"
            local pi = ...
            pi.register_tool({
                name = "greet",
                label = "Greet",
                description = "Greets someone",
                parameters = {
                    type = "object",
                    properties = { who = { type = "string" } },
                },
                execute = function(tool_call_id, params)
                    pi.sleep(200) -- longer than the budget: await time is free
                    return {
                        content = { { type = "text", text = "hi " .. params.who } },
                        details = { id = tool_call_id },
                    }
                end,
            })
        "#,
    )
    .expect("load");

    let tools = host.tools().expect("tools mirror");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "greet");
    assert_eq!(tools[0].source, "test://greet");
    assert_eq!(tools[0].meta["label"], "Greet");
    assert_eq!(tools[0].meta["description"], "Greets someone");
    assert_eq!(tools[0].meta["parameters"]["type"], "object");
    // Function fields are stripped from the mirror.
    assert!(tools[0].meta.get("execute").is_none());

    let result = host
        .call_tool("greet", "call-1", &serde_json::json!({ "who": "pi" }))
        .expect("tool runs");
    assert_eq!(
        result,
        serde_json::json!({
            "content": [{ "type": "text", "text": "hi pi" }],
            "details": { "id": "call-1" },
        })
    );
}

#[test]
fn first_tool_registration_wins_across_extensions() {
    let host = host();
    host.load(
        "test://first",
        r#"
            local pi = ...
            pi.register_tool({ name = "dup", execute = function() return { from = "first" } end })
        "#,
    )
    .expect("load first");
    host.load(
        "test://second",
        r#"
            local pi = ...
            pi.register_tool({ name = "dup", execute = function() return { from = "second" } end })
        "#,
    )
    .expect("load second");

    let tools = host.tools().expect("tools mirror");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].source, "test://first");

    let result = host
        .call_tool("dup", "call-1", &serde_json::json!({}))
        .expect("tool runs");
    assert_eq!(result, serde_json::json!({ "from": "first" }));
}

#[test]
fn reregistration_within_extension_replaces_in_place() {
    let host = host();
    host.load(
        "test://replace",
        r#"
            local pi = ...
            pi.register_tool({ name = "t", execute = function() return { v = 1 } end })
            pi.register_tool({ name = "u", execute = function() return { v = 0 } end })
            pi.register_tool({ name = "t", execute = function() return { v = 2 } end })
        "#,
    )
    .expect("load");

    // JS Map.set: the second "t" replaces the first but keeps its position.
    let tools = host.tools().expect("tools mirror");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["t", "u"]);

    let result = host
        .call_tool("t", "call-1", &serde_json::json!({}))
        .expect("tool runs");
    assert_eq!(result, serde_json::json!({ "v": 2 }));
}

#[test]
fn unknown_tool_and_command_error() {
    let host = host();
    match host.call_tool("nope", "call-1", &serde_json::json!({})) {
        Err(HostError::UnknownTool(name)) => assert_eq!(name, "nope"),
        other => panic!("expected UnknownTool, got {other:?}"),
    }
    match host.call_command("nope", "") {
        Err(HostError::UnknownCommand(name)) => assert_eq!(name, "nope"),
        other => panic!("expected UnknownCommand, got {other:?}"),
    }
}

#[test]
fn invalid_registrations_are_load_errors() {
    let host = host();
    let empty_name = host.load(
        "test://bad-tool-name",
        r#"
            local pi = ...
            pi.register_tool({ name = "", execute = function() end })
        "#,
    );
    assert!(empty_name.is_err());

    let no_execute = host.load(
        "test://bad-tool-execute",
        r#"
            local pi = ...
            pi.register_tool({ name = "x" })
        "#,
    );
    assert!(no_execute.is_err());

    let no_handler = host.load(
        "test://bad-command",
        r#"
            local pi = ...
            pi.register_command("x", { description = "no handler" })
        "#,
    );
    assert!(no_handler.is_err());

    let bad_shortcut = host.load(
        "test://bad-shortcut",
        r#"
            local pi = ...
            pi.register_shortcut("  ", { handler = function() end })
        "#,
    );
    assert!(bad_shortcut.is_err());

    let shortcut_no_handler = host.load(
        "test://bad-shortcut-handler",
        r#"
            local pi = ...
            pi.register_shortcut("ctrl+x", { description = "no handler" })
        "#,
    );
    assert!(shortcut_no_handler.is_err());
}

#[test]
fn shortcut_registry_resolves_first_registration_and_lowercases_keys() {
    let host = host();
    host.load(
        "test://short-a",
        r#"
            local pi = ...
            pi.register_shortcut("Ctrl+X", {
                description = "first",
                handler = function() end,
            })
            pi.register_command("list-shortcuts", {
                handler = function()
                    local out = {}
                    for i, shortcut in ipairs(pi.registered_shortcuts()) do
                        out[i] = { shortcut = shortcut.shortcut, description = shortcut.description }
                    end
                    return out
                end,
            })
        "#,
    )
    .expect("first extension loads");
    host.load(
        "test://short-b",
        r#"
            local pi = ...
            pi.register_shortcut("ctrl+x", {
                description = "second extension loses",
                handler = function() end,
            })
            pi.register_shortcut("alt+z", {
                description = "other key",
                handler = function() end,
            })
        "#,
    )
    .expect("second extension loads");
    let reply = host
        .call_command("list-shortcuts", "")
        .expect("command runs")
        .expect("result");
    assert_eq!(
        reply,
        serde_json::json!([
            { "shortcut": "ctrl+x", "description": "first" },
            { "shortcut": "alt+z", "description": "other key" }
        ])
    );
}

#[test]
fn command_runs_with_args_and_awaits() {
    let host = host_with_budget(100);
    host.load(
        "test://cmd",
        r#"
            local pi = ...
            pi.register_command("shout", {
                description = "Uppercase the arguments",
                handler = function(args)
                    pi.sleep(200) -- await time is free
                    return string.upper(args)
                end,
            })
        "#,
    )
    .expect("load");

    let commands = host.commands().expect("commands mirror");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "shout");
    // A unique name keeps its plain invocation name.
    assert_eq!(commands[0].invocation_name, "shout");
    assert_eq!(
        commands[0].description.as_deref(),
        Some("Uppercase the arguments")
    );

    let reply = host.call_command("shout", "hey").expect("command runs");
    assert_eq!(reply, Some(serde_json::json!({ "message": "HEY" })));
}

#[test]
fn command_collisions_get_numbered_invocation_names() {
    let host = host();
    host.load(
        "test://one",
        r#"
            local pi = ...
            pi.register_command("deploy", { handler = function() return "one" end })
        "#,
    )
    .expect("load one");
    host.load(
        "test://two",
        r#"
            local pi = ...
            pi.register_command("deploy", { handler = function() return "two" end })
        "#,
    )
    .expect("load two");

    let commands = host.commands().expect("commands mirror");
    let resolved: Vec<(&str, &str)> = commands
        .iter()
        .map(|c| (c.invocation_name.as_str(), c.source.as_str()))
        .collect();
    assert_eq!(
        resolved,
        vec![("deploy:1", "test://one"), ("deploy:2", "test://two")]
    );

    let reply = host.call_command("deploy:2", "").expect("command runs");
    assert_eq!(reply, Some(serde_json::json!({ "message": "two" })));
}

#[test]
fn hung_tool_is_killed_by_watchdog() {
    let host = host_with_budget(100);
    host.load(
        "test://hang-tool",
        r#"
            local pi = ...
            pi.register_tool({ name = "hang", execute = function() while true do end end })
        "#,
    )
    .expect("load");

    match host.call_tool("hang", "call-1", &serde_json::json!({})) {
        Err(HostError::Timeout(_)) => {}
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[test]
fn exerciser_hello_runs() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/hello.lua"
    );
    let source = std::fs::read_to_string(path).expect("exerciser example exists");
    let host = host();
    host.load("examples/extensions/hello.lua", &source)
        .expect("example loads");

    let tools = host.tools().expect("tools mirror");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "hello");
    assert_eq!(tools[0].meta["label"], "Hello");

    let result = host
        .call_tool("hello", "call-1", &serde_json::json!({ "name": "World" }))
        .expect("tool runs");
    assert_eq!(
        result,
        serde_json::json!({
            "content": [{ "type": "text", "text": "Hello, World!" }],
            "details": { "greeted": "World" },
        })
    );
}

#[test]
fn exerciser_command_demo_runs() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/command-demo.lua"
    );
    let source = std::fs::read_to_string(path).expect("exerciser example exists");
    let host = host();
    host.load("examples/extensions/command-demo.lua", &source)
        .expect("example loads");

    let commands = host.commands().expect("commands mirror");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].invocation_name, "echo");

    let reply = host.call_command("echo", "hello").expect("command runs");
    assert_eq!(reply, Some(serde_json::json!({ "message": "echo: hello" })));
}
