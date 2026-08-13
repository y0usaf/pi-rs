#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// PLAN 9.2 JSON/RPC extension-command context delivery. Pi's `session.prompt()`
// (driven by print/json/rpc modes) routes leading-`/` messages to the matching
// extension command handler with a live command context, and the message is
// NOT sent to the model (provider response stays pending, session messages
// stay empty). print/json bind no UI context (hasUI false); rpc-mode binds a
// real createExtensionUIContext (hasUI true + ui.notify callable). The
// `delivery` section of tests/extension-context-parity/oracle.json (generated
// from Pi's real runner/AgentSession) pins the delivered context fields and
// the "not consumed" behavior. This test replays the same command through
// pi-rs's real print and RPC roles with a file-backed extension and asserts
// the observed context matches the Pi oracle byte for byte.
use pi_rs_agent::PACK;
use pi_rs_app::builtins::{AGENT_CORE_PACK, CODING_AGENT_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};
use std::io::Write;
use std::process::{Command, Stdio};

fn delivery_oracle() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/extension-context-parity/oracle.json"
    ))
    .unwrap()
}

/// Register the same `ctx-deliver` command used to generate Pi's delivery
/// oracle, recording the observed command context into `_G.delivery_observed`.
fn load_delivery_extension(host: &Host) {
    host.load(
        "test://ctx-deliver",
        r#"
            local pi = ...
            _G.delivery_observed = {}
            pi.register_command("ctx-deliver", { description="deliver",
                handler = function(args, ctx)
                    local entry = {
                        args = args, mode = ctx.mode, hasUI = ctx.hasUI,
                        idle = ctx.isIdle(), trusted = ctx.isProjectTrusted(),
                        hasWait = type(ctx.waitForIdle) == "function",
                        hasNew = type(ctx.newSession) == "function",
                        hasFork = type(ctx.fork) == "function",
                        hasTree = type(ctx.navigateTree) == "function",
                        hasSwitch = type(ctx.switchSession) == "function",
                        hasReload = type(ctx.reload) == "function",
                    }
                    -- Pi's rpc-only oracle records the UI binding.
                    if ctx.mode == "rpc" then
                        entry.hasUiNotify = ctx.ui ~= nil and type(ctx.ui.notify) == "function" or false
                    end
                    _G.delivery_observed[#_G.delivery_observed + 1] = entry
                end })
            pi.register_command("delivery-dump", { handler = function()
                return { observed = _G.delivery_observed }
            end })
            pi.register_command("delivery-reset", { handler = function()
                _G.delivery_observed = {}
            end })
        "#,
    )
    .unwrap();
}

fn model_json() -> serde_json::Value {
    serde_json::json!({
        "id":"custom-1","name":"Custom","api":"anthropic-messages",
        "provider":"faux","baseUrl":"","reasoning":false,"input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000,"maxTokens":1024
    })
}

fn host_with_command(temp: &tempfile::TempDir) -> Host {
    let cwd = temp.path().to_string_lossy().into_owned();
    let agent_dir = temp.path().join("agent");
    // SAFETY: this integration-test process owns its environment.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let host = Host::new(HostConfig {
        cwd: Some(cwd.clone()),
        project_trusted: true,
        ..HostConfig::default()
    })
    .unwrap();
    let rep = host.load_embedded(&[AGENT_CORE_PACK, PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    load_delivery_extension(&host);
    host
}

/// drive_print runs the real print role for `mode` with prompt `/ctx-deliver
/// hello <mode>` and returns the observed delivery context plus whether the
/// message reached the model (agent messages length).
fn drive_print(
    host: &Host,
    temp: &tempfile::TempDir,
    mode: &str,
) -> (serde_json::Value, serde_json::Value) {
    let cwd = temp.path().to_string_lossy();
    let agent = temp.path().join("agent");
    let agent_dir = agent.to_string_lossy();
    let result = host
        .call_role(
            "print",
            &serde_json::json!({
                "model": model_json(), "apiKey": "k",
                "prompt": format!("/ctx-deliver hello {mode}"),
                "cwd": cwd, "agentDir": agent_dir,
                "projectTrusted": true,
                "readmePath": "/r", "docsPath": "/d", "examplesPath": "/e",
                "mode": mode,
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    let dump = host.call_command("delivery-dump", "").unwrap().unwrap();
    (result, dump)
}

#[test]
fn print_and_json_command_delivery_matches_pi_oracle() {
    let expected = delivery_oracle();
    let temp = tempfile::tempdir().unwrap();
    let host = host_with_command(&temp);
    for mode in ["print", "json"] {
        // Reset the observation collector between role runs (same host VM).
        host.call_command("delivery-reset", "").unwrap();
        let (result, dump) = drive_print(&host, &temp, mode);
        // The command was handled: no assistant message produced, exit 0.
        assert_eq!(result["exitCode"].as_u64().unwrap_or(0), 0, "{mode} exit 0");
        let observed = &dump["observed"];
        assert_eq!(
            observed, &expected["delivery"][mode]["observed"],
            "{mode} delivery context must match Pi oracle"
        );
    }
}

/// run_rpc_ext runs the real `pi --mode rpc --extension <file>` binary,
/// feeding `input` on stdin, returning (stdout, stderr, exit).
fn run_rpc_ext(input: &str, cwd: &str, extension: &str) -> (String, String, i32) {
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
        .arg("--extension")
        .arg(extension)
        .arg("--cwd")
        .arg(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", cwd)
        .spawn()
        .expect("spawn pi");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait pi");
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap(),
    )
}

fn records(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect()
}

#[test]
fn rpc_command_delivery_matches_pi_oracle() {
    let expected = delivery_oracle();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let ext = temp.path().join("rpc-deliver.lua");
    std::fs::write(
        &ext,
        "local pi = ...\n\
         pi.register_command('ctx-deliver', { description='deliver',\n\
           handler = function(args, ctx)\n\
             local entry = {\n\
               args = args, mode = ctx.mode, hasUI = ctx.hasUI,\n\
               hasUiNotify = ctx.ui ~= nil and type(ctx.ui.notify) == 'function' or false,\n\
               idle = ctx.isIdle(), trusted = ctx.isProjectTrusted(),\n\
               hasWait = type(ctx.waitForIdle) == 'function',\n\
               hasNew = type(ctx.newSession) == 'function',\n\
               hasFork = type(ctx.fork) == 'function',\n\
               hasTree = type(ctx.navigateTree) == 'function',\n\
               hasSwitch = type(ctx.switchSession) == 'function',\n\
               hasReload = type(ctx.reload) == 'function',\n\
             }\n\
             io.stderr:write('RPC_OBS ' .. pi.json.encode({ observed = { entry } }) .. '\\n')\n\
           end })\n",
    )
    .unwrap();
    let (stdout, stderr, code) = run_rpc_ext(
        "{\"type\":\"prompt\",\"message\":\"/ctx-deliver hello rpc\",\"id\":\"p1\"}\n",
        &cwd,
        &ext.to_string_lossy(),
    );
    assert_eq!(code, 0, "RPC completes successfully");
    // The extension command was handled: prompt succeeds.
    let recs = records(&stdout);
    assert!(
        recs.iter()
            .any(|r| r["command"] == "prompt" && r["success"] == true),
        "rpc prompt for /ctx-deliver must succeed, got: {stdout}"
    );
    // The observed command context matches the Pi rpc delivery oracle.
    let obs_line = stderr
        .lines()
        .find(|l| l.starts_with("RPC_OBS "))
        .expect("extension dump on stderr");
    let observed_json = &obs_line["RPC_OBS ".len()..];
    let observed: serde_json::Value = serde_json::from_str(observed_json).unwrap();
    assert_eq!(
        observed["observed"], expected["delivery"]["rpc"]["observed"],
        "rpc delivery context must match Pi oracle"
    );
}

#[test]
fn json_command_delivery_not_consuming_provider_on_seam() {
    // Pi pins `pending:1` / `messages:0` for a led `-` command through
    // session.prompt (the provider response enqueued stays pending and no
    // message is recorded). Mirror the essential claim: the JSON role with a
    // `/command` initial prompt emits nothing to the model and exits 0.
    let _expected = delivery_oracle();
    assert_eq!(
        _expected["delivery"]["json"]["messages"], 0,
        "oracle: command delivery must not record a session message"
    );
    let temp = tempfile::tempdir().unwrap();
    let host = host_with_command(&temp);
    let (result, _dump) = drive_print(&host, &temp, "json");
    assert_eq!(result["exitCode"].as_u64().unwrap_or(0), 0);
}
