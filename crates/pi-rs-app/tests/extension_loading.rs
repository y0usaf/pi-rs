#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 9.1 slice: ordinary files enter the shipped product VM in resource
//! precedence order; async/failing initialization is isolated; translated
//! hello + permission-gate execute through product Lua composition.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use pi_rs_app::builtins::{CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_app::cli::extensions::load_product_extensions;
use pi_rs_host::{Host, HostConfig};
use pi_rs_session::SessionManager;

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
fn queued_extension_ui_actions_match_pi_examples() {
    let root = tempfile::tempdir().unwrap();
    let agent_dir = root.path().join("agent");
    // SAFETY: this integration-test process owns its environment.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.load(
        "examples/extensions/commands.lua",
        include_str!("../../../examples/extensions/commands.lua"),
    )
    .unwrap();
    host.load(
        "examples/extensions/permission-gate.lua",
        include_str!("../../../examples/extensions/permission-gate.lua"),
    )
    .unwrap();

    let scenario = include_str!("../../../tests/ui-parity/extension-ui-turn.json");
    let result = host
        .call_command("interactive-extension-ui-parity-sequence", scenario)
        .unwrap()
        .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/extension-ui-parity/oracle.json"
    ))
    .unwrap();
    assert_eq!(result["actions"], expected["actions"]);
    assert_eq!(result["permissionResult"], expected["permissionResult"]);
    assert_eq!(result["frames"].as_array().unwrap().len(), 7);
}

#[test]
fn extension_context_snapshots_and_shutdown_match_pi() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    // SAFETY: this integration-test process owns its environment.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.load(
        "examples/extensions/shutdown-command.lua",
        include_str!("../../../examples/extensions/shutdown-command.lua"),
    )
    .unwrap();

    let model = serde_json::json!({
        "id":"faux-1", "name":"Faux", "api":"anthropic-messages",
        "provider":"faux", "baseUrl":"http://127.0.0.1:1", "reasoning":false,
        "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000, "maxTokens":1024
    });
    let request = serde_json::json!({
        "model":model, "cwd":cwd, "agentDir":agent_dir,
        "readmePath":"/pi-rs-pkg/README.md", "docsPath":"/pi-rs-pkg/docs",
        "examplesPath":"/pi-rs-pkg/examples"
    });
    let actual = host
        .call_command("interactive-extension-context-parity", &request.to_string())
        .unwrap()
        .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/extension-context-parity/oracle.json"
    ))
    .unwrap();
    assert_eq!(actual["snapshot"], expected["snapshot"]);
    assert_eq!(actual["stale"], expected["stale"]);
    assert_eq!(actual["modes"], expected["modes"]);
    assert_eq!(actual["actions"], expected["actions"]);
    assert_eq!(actual["replacement"], expected["replacement"]);
    assert_eq!(actual["reload"], expected["reload"]);

    let mut tool_request = request;
    tool_request["tool"] = "finish_and_exit".into();
    let tool = host
        .call_command(
            "interactive-extension-context-parity",
            &tool_request.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(tool["snapshot"]["shutdowns"], 1);
    assert_eq!(
        tool["toolResult"]["content"][0]["text"],
        "Shutdown requested. Exiting after this response."
    );

    host.load(
        "examples/extensions/session-lifecycle-demo.lua",
        include_str!("../../../examples/extensions/session-lifecycle-demo.lua"),
    )
    .unwrap();
    tool_request["lifecycleAction"] = serde_json::json!({ "action": "new" });
    let replaced = host
        .call_command(
            "interactive-extension-action-behavior",
            &tool_request.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(replaced["result"]["result"]["cancelled"], false);
    assert_eq!(replaced["result"]["callback"]["mode"], "tui");
    assert_eq!(replaced["result"]["callback"]["idle"], true);
    assert_eq!(
        replaced["eventContext"],
        serde_json::json!({
            "mode":"tui", "hasUI":true, "cwd":cwd,
            "idle":true, "pending":false, "commandOnly":false
        })
    );
    assert_ne!(
        replaced["result"]["beforeId"],
        replaced["result"]["callback"]["sessionId"]
    );
    assert_eq!(
        replaced["currentSessionId"],
        replaced["result"]["callback"]["sessionId"]
    );
    assert_eq!(replaced["result"]["stale"], expected["stale"]);
    assert_eq!(replaced["generation"], 2);

    let session_dir = root.path().join("sessions");
    let mut source_session = SessionManager::create(
        &cwd.to_string_lossy(),
        Some(&session_dir.to_string_lossy()),
        &agent_dir.to_string_lossy(),
        None,
    )
    .unwrap();
    let user_id = source_session
        .append_message(serde_json::json!({
            "role":"user", "content":[{"type":"text","text":"branch me"}],
            "timestamp":1
        }))
        .unwrap();
    source_session
        .append_message(serde_json::json!({
            "role":"assistant", "content":[{"type":"text","text":"done"}],
            "api":"anthropic-messages", "provider":"faux", "model":"faux-1",
            "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,
                "totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,
                    "cacheWrite":0,"total":0}},
            "stopReason":"stop", "timestamp":2
        }))
        .unwrap();
    let source_file = source_session.get_session_file().unwrap().to_owned();

    tool_request["sessionFile"] = source_file.clone().into();
    tool_request["sessionDir"] = session_dir.to_string_lossy().into_owned().into();
    tool_request["lifecycleAction"] = serde_json::json!({
        "action": "tree", "entryId": user_id
    });
    let navigated = host
        .call_command(
            "interactive-extension-action-behavior",
            &tool_request.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(navigated["result"]["result"]["cancelled"], false);
    assert_eq!(navigated["result"]["stale"], "");
    assert_eq!(navigated["currentSessionFile"], source_file);
    assert_eq!(navigated["generation"], 1);

    tool_request["lifecycleAction"] = serde_json::json!({
        "action": "fork", "entryId": user_id, "position": "at"
    });
    let forked = host
        .call_command(
            "interactive-extension-action-behavior",
            &tool_request.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(forked["result"]["result"]["cancelled"], false);
    assert_eq!(forked["result"]["stale"], expected["stale"]);
    assert_eq!(forked["generation"], 2);
    assert_ne!(forked["currentSessionFile"], source_file);
    assert_eq!(
        forked["currentSessionFile"],
        forked["result"]["callback"]["sessionFile"]
    );

    let mut target_session = SessionManager::create(
        &cwd.to_string_lossy(),
        Some(&session_dir.to_string_lossy()),
        &agent_dir.to_string_lossy(),
        None,
    )
    .unwrap();
    target_session
        .append_message(serde_json::json!({
            "role":"assistant", "content":[{"type":"text","text":"target"}],
            "api":"anthropic-messages", "provider":"faux", "model":"faux-1",
            "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,
                "totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,
                    "cacheWrite":0,"total":0}},
            "stopReason":"stop", "timestamp":3
        }))
        .unwrap();
    let target_file = target_session.get_session_file().unwrap().to_owned();
    tool_request["sessionFile"] = source_file.into();
    tool_request["lifecycleAction"] = serde_json::json!({
        "action": "switch", "sessionPath": target_file
    });
    let switched = host
        .call_command(
            "interactive-extension-action-behavior",
            &tool_request.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(switched["result"]["result"]["cancelled"], false);
    assert_eq!(switched["result"]["stale"], expected["stale"]);
    assert_eq!(switched["currentSessionFile"], target_file);
    assert_eq!(switched["generation"], 2);

    tool_request["lifecycleAction"] = serde_json::json!({ "action": "reload" });
    let reloaded = host
        .call_command(
            "interactive-extension-action-behavior",
            &tool_request.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(reloaded["result"]["result"]["cancelled"], false);
    assert_eq!(reloaded["result"]["stale"], expected["stale"]);
    assert_eq!(reloaded["generation"], 2);
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

fn read_request(stream: &mut TcpStream) -> serde_json::Value {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        if count == 0 {
            return serde_json::Value::Null;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

fn stable_source(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn product_extension_runtime_matches_pi_oracle() {
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/extension-runtime-parity/oracle.json"
    ))
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();

    let sources = [
        (
            "01-first.lua",
            r#"local pi = ...
__extension_trace = {"first:start"}
pi.sleep(0)
__extension_trace[#__extension_trace + 1] = "first:end"
pi.register_tool({name="shared",label="Shared First",description="first wins",parameters={type="object",properties={},required=pi.json.decode("[]")},execute=function() return {content={{type="text",text="first"}},details={owner="first"}} end})
pi.register_tool({name="hello",label="Hello",description="A simple greeting tool",parameters={type="object",properties={name={type="string",description="Name to greet"}},required={"name"}},execute=function(_,params) return {content={{type="text",text="Hello, "..params.name.."!"}},details={greeted=params.name}} end})
pi.register_command("dup",{description="first dup",handler=function() return "first-command" end})
pi.register_command("trace",{description="trace",handler=function() return __extension_trace end})
pi.register_flag("plan",{description="Plan mode",type="boolean",default=false})
pi.register_flag("profile",{description="Profile name",type="string",default="safe"})
pi.register_command("flag-values",{handler=function() return {plan=pi.get_flag("plan"),profile=pi.get_flag("profile"),missing=pi.get_flag("missing")} end})
pi.register_command("catalog",{description="catalog",get_argument_completions=function(prefix) local values={} for _,source in ipairs({"extension","prompt","skill"}) do if source:sub(1,#prefix)==prefix then values[#values+1]={value=source,label=source} end end return values end,handler=function() return pi.get_commands() end})
pi.on("tool_call",function() __extension_trace[#__extension_trace + 1]="hook:first" return {tag="first"} end)"#,
        ),
        (
            "02-bad.lua",
            r#"local pi = ...
__extension_trace[#__extension_trace + 1]="bad:start"
pi.register_tool({name="ghost",label="Ghost",description="must roll back",parameters={},execute=function() return {} end})
pi.register_command("ghost",{handler=function() return "ghost" end})
pi.on("tool_call",function() __extension_trace[#__extension_trace + 1]="hook:ghost" end)
pi.sleep(0)
error("broken init")"#,
        ),
        (
            "03-second.lua",
            r#"local pi = ...
__extension_trace[#__extension_trace + 1]="second:start"
pi.sleep(1)
__extension_trace[#__extension_trace + 1]="second:end"
pi.register_tool({name="shared",label="Shared Second",description="loses",parameters={type="object",properties={},required=pi.json.decode("[]")},execute=function() return {content={{type="text",text="second"}},details={owner="second"}} end})
pi.register_command("dup",{description="second dup",handler=function() return "second-command" end})
pi.register_flag("plan",{description="Conflicting plan",type="boolean",default=true})
pi.register_flag("second-only",{type="string"})
pi.on("tool_call",function() __extension_trace[#__extension_trace + 1]="hook:second" return {tag="second"} end)"#,
        ),
        (
            "04-block.lua",
            r#"local pi = ...
__extension_trace[#__extension_trace + 1]="block:loaded"
pi.on("tool_call",function() __extension_trace[#__extension_trace + 1]="hook:block" return {block=true,reason="blocked"} end)
pi.on("tool_call",function() __extension_trace[#__extension_trace + 1]="hook:after-block" return {tag="after"} end)"#,
        ),
    ];
    let paths: Vec<String> = sources
        .iter()
        .map(|(name, source)| {
            let path = root.path().join(name);
            write(&path, source);
            path.to_string_lossy().into_owned()
        })
        .collect();

    // Isolate auth from the developer's real ~/.pi credentials before the VM
    // constructs its per-host AuthStorage.
    // SAFETY: this integration-test process owns its environment.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
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
    let report = host.load_extensions(&paths);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        seen.lock().unwrap().push(read_request(&mut stream));
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_ext\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-test\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    let model = serde_json::json!({
        "id": "claude-test", "name": "Claude Test", "api": "anthropic-messages",
        "provider": "anthropic", "baseUrl": format!("http://{address}"), "reasoning": false,
        "input": ["text"], "cost": {"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow": 100000, "maxTokens": 1024
    });
    host.call_role(
        "print",
        &serde_json::json!({
            "model": model, "apiKey": "test-key", "prompt": "hello",
            "cwd": cwd, "agentDir": agent_dir,
            "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
            "examplesPath": "/pi-rs-pkg/examples"
        })
        .to_string(),
    )
    .unwrap();
    server.join().unwrap();

    let errors: Vec<serde_json::Value> = report
        .errors
        .iter()
        .map(|error| {
            let message = if error.error.contains("broken init") {
                "Failed to load extension: broken init".to_owned()
            } else {
                error
                    .error
                    .replace(&format!("{}/", root.path().display()), "")
                    .replace(".lua", "")
            };
            serde_json::json!({"path": stable_source(&error.path), "error": message})
        })
        .collect();
    let tools: Vec<serde_json::Value> = host
        .tools()
        .unwrap()
        .into_iter()
        .filter(|tool| !tool.source.starts_with('<'))
        .map(|tool| serde_json::json!({"name":tool.name,"source":stable_source(&tool.source)}))
        .collect();
    let commands: Vec<serde_json::Value> = host
        .commands()
        .unwrap()
        .into_iter()
        .filter(|command| !command.source.starts_with('<'))
        .map(|command| {
            serde_json::json!({
                "name":command.name,"invocationName":command.invocation_name,
                "source":stable_source(&command.source),"description":command.description
            })
        })
        .collect();
    let flags: Vec<serde_json::Value> = host
        .flags()
        .unwrap()
        .into_iter()
        .filter(|flag| !flag.source.starts_with('<'))
        .map(|flag| {
            serde_json::json!({
                "name":flag.name,"source":stable_source(&flag.source),
                "description":flag.description,"type":flag.flag_type,"default":flag.default
            })
        })
        .collect();
    let command_results = ["dup:1", "dup:2"]
        .into_iter()
        .map(|name| {
            let result = host.call_command(name, "").unwrap().unwrap();
            serde_json::json!({"name":name,"result":result["message"]})
        })
        .collect::<Vec<_>>();
    let hello_result = host
        .call_tool("hello", "call-1", &serde_json::json!({"name":"Ada"}))
        .unwrap();
    let hook = host
        .call_command(
            "extension-vertical-slice",
            r#"{"toolCall":{"name":"bash","arguments":{"command":"sudo true"}}}"#,
        )
        .unwrap()
        .unwrap();
    let trace = host.call_command("trace", "").unwrap().unwrap();
    let flag_values = host.call_command("flag-values", "").unwrap().unwrap();
    let mut command_catalog = host.call_command("catalog", "").unwrap().unwrap();
    for command in command_catalog.as_array_mut().unwrap() {
        let source_info = command["sourceInfo"].as_object_mut().unwrap();
        let path = source_info["path"].as_str().unwrap().to_owned();
        source_info.insert("path".to_owned(), stable_source(&path).into());
        let source = source_info["source"].as_str().unwrap().to_owned();
        source_info.insert("source".to_owned(), stable_source(&source).into());
        if !command.as_object().unwrap().contains_key("description") {
            command
                .as_object_mut()
                .unwrap()
                .insert("description".to_owned(), serde_json::Value::Null);
        }
    }
    let argument_completions = host
        .call_command(
            "extension-vertical-slice",
            r#"{"commandCompletion":{"name":"catalog","prefix":"pr"}}"#,
        )
        .unwrap()
        .unwrap()["completions"]
        .clone();
    let request = &requests.lock().unwrap()[0];
    let extension_tools = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tool| tool["name"] == "hello" || tool["name"] == "shared")
        .map(|tool| {
            serde_json::json!({
                "name":tool["name"],"description":tool["description"],
                "parameters":tool["input_schema"]
            })
        })
        .collect::<Vec<_>>();
    let captured_requests = vec![serde_json::json!({
        "toolNames":request["tools"].as_array().unwrap().iter().map(|tool| tool["name"].clone()).collect::<Vec<_>>(),
        "extensionTools":extension_tools
    })];
    let actual = serde_json::json!({
        "loaded":report.loaded.iter().map(|path| stable_source(path)).collect::<Vec<_>>(),
        "errors":errors,"tools":tools,"commands":commands,"flags":flags,
        "commandResults":command_results,"commandCatalog":command_catalog,
        "argumentCompletions":argument_completions,"flagValues":flag_values,
        "helloResult":hello_result,"hookResult":hook["hookResult"],"trace":trace,
        "capturedRequests":captured_requests
    });
    assert_eq!(actual, expected);
}

#[test]

fn complete_event_folds_match_pi_runner_oracle() {
    let mut expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/extension-event-parity/oracle.json"
    ))
    .unwrap();
    let fold_error_count = expected["foldErrorCount"].as_u64().unwrap() as usize;
    expected.as_object_mut().unwrap().remove("foldErrorCount");
    expected.as_object_mut().unwrap().remove("productTrace");
    expected["errors"]
        .as_array_mut()
        .unwrap()
        .truncate(fold_error_count);
    let host = Host::new(HostConfig::default()).unwrap();
    let embedded = host.load_embedded(&[pi_rs_agent::PACK, CODING_AGENT_PACK]);
    assert!(embedded.errors.is_empty(), "{:?}", embedded.errors);
    host.load(
        "01-first",
        include_str!("../../../tests/extension-event-parity/01-first.lua"),
    )
    .unwrap();
    host.load(
        "02-second",
        include_str!("../../../tests/extension-event-parity/02-second.lua"),
    )
    .unwrap();
    let actual = host
        .call_command(
            "extension-event-fold-parity",
            r#"{"cwd":"/oracle/project"}"#,
        )
        .unwrap()
        .unwrap();
    assert_eq!(actual, expected);
}

fn write_sse_response(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\nX-Event-Oracle: yes\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).unwrap();
}

#[test]
fn real_product_seams_follow_pi_generated_event_order() {
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/extension-event-parity/oracle.json"
    ))
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let agent_dir = root.path().join("agent");
    std::fs::create_dir_all(&cwd).unwrap();
    // SAFETY: this integration-test process owns its environment.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let server = thread::spawn(move || {
        let tool_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tool\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-test\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"event-call\",\"name\":\"event_tool\",\"input\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"value\\\":\\\"x\\\"}\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let text_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_done\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-test\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        for body in [tool_body, text_body] {
            let (mut stream, _) = listener.accept().unwrap();
            seen.lock().unwrap().push(read_request(&mut stream));
            write_sse_response(&mut stream, body);
        }
    });

    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .unwrap();
    let embedded = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(embedded.errors.is_empty(), "{:?}", embedded.errors);
    host.load(
        "01-first",
        include_str!("../../../tests/extension-event-parity/01-first.lua"),
    )
    .unwrap();
    host.load(
        "02-second",
        include_str!("../../../tests/extension-event-parity/02-second.lua"),
    )
    .unwrap();
    let model = serde_json::json!({
        "id":"claude-test", "name":"Claude Test", "api":"anthropic-messages",
        "provider":"anthropic", "baseUrl":format!("http://{address}"), "reasoning":false,
        "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":100000, "maxTokens":1024
    });
    let result = host
        .call_role(
            "print",
            &serde_json::json!({
                "model":model,"apiKey":"test-key","prompt":"go",
                "cwd":cwd,"agentDir":agent_dir,"projectTrusted":true,
                "readmePath":"/pi-rs-pkg/README.md","docsPath":"/pi-rs-pkg/docs",
                "examplesPath":"/pi-rs-pkg/examples"
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();

    server.join().unwrap();
    assert_eq!(result["text"], "first replacement");
    let fold_error_count = expected["foldErrorCount"].as_u64().unwrap() as usize;
    let product_errors = serde_json::Value::Array(
        expected["errors"].as_array().unwrap()[fold_error_count..].to_vec(),
    );
    assert_eq!(result["extensionErrors"], product_errors);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["first"], true);
    assert_eq!(requests[0]["second"], true);
    let tool_result = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .find(|block| block["type"] == "tool_result")
        .expect("tool_result block");
    assert_eq!(tool_result["content"], "first result");
    drop(requests);

    let trace = host.call_command("event-trace", "").unwrap().unwrap();
    let significant = [
        "session_start",
        "resources_discover",
        "input",
        "before_agent_start",
        "agent_start",
        "turn_start",
        "message_start",
        "message_end",
        "after_provider_response",
        "tool_execution_start",
        "tool_call",
        "tool_result",
        "tool_execution_end",
        "turn_end",
        "agent_end",
    ];
    let mut filtered = trace
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            let kind = entry.as_str().unwrap().split_once(':').unwrap().1;
            significant.contains(&kind)
        })
        .cloned()
        .collect::<Vec<_>>();
    let first_input = filtered
        .iter()
        .position(|entry| entry == "first:input")
        .unwrap();
    filtered.drain(..first_input);
    assert_eq!(serde_json::Value::Array(filtered), expected["productTrace"]);
}

#[test]
fn file_backed_compact_middleware_composes_without_frontend_patching() {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.load(
        "examples/extensions/pi-compact.lua",
        include_str!("../../../examples/extensions/pi-compact.lua"),
    )
    .unwrap();
    // A broken earlier middleware cannot suppress later middleware/defaults.
    host.load(
        "test://broken-renderer",
        r#"local pi=...; pi.register_render_middleware("user", {name="broken", order=-200, render=function(row) row.text="mutate" end})"#,
    )
    .unwrap();

    let request = serde_json::json!({
        "width": 64, "version": "0.79.0", "cwd": "/tmp/project",
        "model": {"id":"faux","provider":"faux","contextWindow":128000,"reasoning":false},
        "transcript": [
            {"kind":"user","text":"Please inspect the renderer"},
            {"kind":"assistant","message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"private chain of thought"},
                {"type":"text","text":"Visible answer"}],"stopReason":"stop"}},
            {"kind":"tool","name":"read","toolCallId":"t1","args":{"path":"src/main.rs"},
                "state":"success","result":{"content":[{"type":"text","text":"ok"}]}},
            {"kind":"custom","message":{"customType":"status-update","content":"indexed 3 files"}}
        ]
    });
    let result = host
        .call_command("interactive-frame", &request.to_string())
        .unwrap()
        .unwrap();
    let rendered = result["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("Visible answer"), "{rendered}");
    assert!(rendered.contains("🧠 0.0s"), "{rendered}");
    assert!(!rendered.contains("private chain of thought"), "{rendered}");
    assert!(rendered.contains("✓ read ╱ src/main.rs"), "{rendered}");
    assert!(
        rendered.contains("[status-update] indexed 3 files"),
        "{rendered}"
    );
    // Default pi-compact user mode is borderless, not its compact ::: mode.
    assert!(
        rendered.contains("Please inspect the renderer"),
        "{rendered}"
    );
}

#[test]
fn file_backed_ui_showcase_drives_dialogs_slots_editor_and_cleanup() {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.load(
        "examples/extensions/ui-surface-demo.lua",
        include_str!("../../../examples/extensions/ui-surface-demo.lua"),
    )
    .unwrap();
    let scenario = serde_json::json!({
        "columns": 72, "rows": 30, "fullFrame": true,
        "steps": [
            {"name":"open","submit":"/ui-showcase"},
            {"name":"input","input":["A","d","a","\r"]},
            {"name":"editor","input":["\r"]},
            {"name":"custom","input":["\r"]}
        ]
    });
    let result = host
        .call_command(
            "interactive-extension-ui-parity-sequence",
            &scenario.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(result["uiState"]["title"], "pi · showcase");
    assert_eq!(result["uiState"]["toolsExpanded"], true);
    assert_eq!(result["uiState"]["customHeader"], true);
    assert_eq!(result["uiState"]["customFooter"], true);
    assert_eq!(result["uiState"]["aboveWidget"], true);
    assert_eq!(result["uiState"]["belowWidget"], true);
    assert_eq!(result["uiState"]["active"], false);
    assert!(
        result["uiState"]["status"]
            .as_str()
            .unwrap()
            .contains("UI showcase")
    );
    assert!(
        result["uiState"]["editorText"]
            .as_str()
            .unwrap()
            .contains("seed + paste")
    );
    assert!(
        result["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["type"] == "custom_result")
    );
    let rendered = result["frames"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|frame| frame["ansi"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for expected in [
        "Custom header",
        "Custom footer",
        "Above editor widget",
        "Below editor widget",
        "Showcase status slot",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
}

#[test]
fn print_context_uses_pinned_no_ui_outcomes() {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.load(
        "examples/extensions/headless-ui-probe.lua",
        r#"
            local pi = ...
            pi.register_tool({ name="headless-ui-probe", execute=function(_, _, _, _, ctx)
                return { content={{type="text", text="headless"}}, details={
                    selected=ctx.ui.select("x", {"a"}), confirmed=ctx.ui.confirm("x", "y"),
                    input=ctx.ui.input("x"), editor=ctx.ui.editor("x"), text=ctx.ui.getEditorText(),
                    expanded=ctx.ui.getToolsExpanded(), themes=#ctx.ui.getAllThemes(),
                    themeResult=ctx.ui.setTheme("dark")
                }}
            end })
        "#,
    )
    .unwrap();
    let result = host
        .call_command(
            "extension-vertical-slice",
            r#"{"tool":"headless-ui-probe"}"#,
        )
        .unwrap()
        .unwrap();
    let details = &result["result"]["details"];
    assert_eq!(details["confirmed"], false);
    assert_eq!(details["text"], "");
    assert_eq!(details["expanded"], false);
    assert_eq!(details["themes"], 0);
    assert_eq!(details["themeResult"]["success"], false);
    assert_eq!(details["themeResult"]["error"], "UI not available");
    assert!(details.get("selected").is_none() || details["selected"].is_null());
}
