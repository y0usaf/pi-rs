#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Differential for Plan 10 RPC mode. The oracle in
// tests/rpc-parity/oracle.json is generated from Pi's real `runRpcMode`
// (modes/rpc/rpc-mode.ts) by scripts/rpc-oracle, driving a scripted session
// over a JSONL command corpus. Each case records Pi's raw stdout bytes on
// stdin EOF.
//
// This test drives the real `pi --mode rpc` binary as a subprocess, feeds the
// same JSONL command lines, and compares each response record to Pi's oracle
// semantically (order-insensitive JSON object equality). RPC is a JSON
// protocol: a consumer parses records, so key order is not user-visible; the
// contract being pinned is the record vocabulary, envelope fields, id
// presence, success/error shape, and deterministic data — not byte order.
//
// Scope: the synchronous, deterministic command vocabulary that pi-rs's real
// session reproduces without injected agent state — set_steering_mode,
// set_follow_up_mode, set_auto_compaction, set_auto_retry, abort_retry,
// unknown-command, and JSON parse error. The empty-array serialization cases
// for get_fork_messages and get_commands are pinned byte-for-byte. The
// model/message-seeded cases (get_state, get_available_models, get_messages,
// get_last_assistant_text, set_model, cycle_model, get_commands with seeded
// data, set_session_name, get_session_stats, export_html) and the async
// agent-streaming commands (prompt, steer, follow_up, abort, bash, compact,
// fork, clone, new_session, switch_session) follow now under PLAN 10 through
// the seeded parity path, which exercises them against Pi's `runRpcMode`
// oracle; see the sibling `rpc_streaming_parity.rs`.
use std::io::Write;
use std::process::{Command, Stdio};
use serde_json::Value;

fn oracle() -> Value {
    serde_json::from_str(include_str!("../../../tests/rpc-parity/oracle.json")).unwrap()
}

/// Run `pi --mode rpc` as a subprocess, feed `input` on stdin, return stdout.
fn run_rpc(input: &str, cwd: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
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
    String::from_utf8(out.stdout).unwrap()
}

/// Split a JSONL stream into parsed records (ignoring blank/trailing).
fn records(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect()
}

#[test]
fn rpc_deterministic_framing_matches_pi_oracle() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();

    // Deterministic, reproducible command corpus (mirrors oracle cases
    // unknown-command-no-id + the set_* subset of state-and-simple).
    let input = concat!(
        "{\"type\":\"set_steering_mode\",\"mode\":\"all\",\"id\":\"r3\"}\n",
        "{\"type\":\"set_follow_up_mode\",\"mode\":\"one-at-a-time\",\"id\":\"r4\"}\n",
        "{\"type\":\"set_auto_compaction\",\"enabled\":true,\"id\":\"r5\"}\n",
        "{\"type\":\"set_auto_retry\",\"enabled\":true,\"id\":\"r6\"}\n",
        "{\"type\":\"abort_retry\",\"id\":\"r7\"}\n",
        "{\"type\":\"definitely_not_a_command\",\"id\":\"r1\"}\n",
    );
    let stdout = run_rpc(input, &cwd);
    let got = records(&stdout);

    // Expected from Pi's oracle (semantic equality; order-insensitive).
    let expected = [
        serde_json::json!({"id":"r3","type":"response","command":"set_steering_mode","success":true}),
        serde_json::json!({"id":"r4","type":"response","command":"set_follow_up_mode","success":true}),
        serde_json::json!({"id":"r5","type":"response","command":"set_auto_compaction","success":true}),
        serde_json::json!({"id":"r6","type":"response","command":"set_auto_retry","success":true}),
        serde_json::json!({"id":"r7","type":"response","command":"abort_retry","success":true}),
        serde_json::json!({"type":"response","command":"definitely_not_a_command","success":false,"error":"Unknown command: definitely_not_a_command"}),
    ];
    assert_eq!(got.len(), expected.len(), "record count");
    for (g, e) in got.iter().zip(expected.iter()) {
        assert_eq!(g, e, "record mismatch");
    }
}

#[test]
fn rpc_unknown_command_matches_pi_oracle_byte_shape() {
    let oracle = oracle();
    let case = oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "unknown-command-no-id")
        .cloned()
        .expect("unknown-command case in oracle");
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    // Re-serialize the case's command lines.
    let input = case["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| serde_json::to_string(c).unwrap() + "\n")
        .collect::<String>();
    let stdout = run_rpc(&input, &cwd);
    let got = records(&stdout);
    // Pi's oracle output for this case.
    let expected: Vec<Value> = case["stdout"]
        .as_str()
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(got, expected, "unknown-command response must match Pi oracle");
}

#[test]
fn rpc_parse_error_matches_pi_oracle_shape() {
    let oracle = oracle();
    let probe = &oracle["parseErrorProbe"];
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let stdout = run_rpc("not json at all\n", &cwd);
    let got = records(&stdout);
    assert_eq!(got.len(), 1, "one parse-error record");
    let rec = &got[0];
    // Pi: `{type:"response", command:"parse", success:false, error:"Failed to
    // parse command: ..."}`. The error text differs (Node vs serde), so pin
    // the envelope + prefix.
    assert_eq!(rec["type"], "response");
    assert_eq!(rec["command"], "parse");
    assert_eq!(rec["success"], false);
    let err = rec["error"].as_str().unwrap();
    assert!(
        err.starts_with("Failed to parse command:"),
        "parse error prefix: {err}"
    );
    // The oracle's error text is Node's; assert the same prefix contract.
    let oracle_err = probe["stdout"]
        .as_str()
        .unwrap()
        .lines()
        .next()
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .and_then(|v| v["error"].as_str().map(str::to_owned))
        .unwrap();
    assert!(
        oracle_err.starts_with("Failed to parse command:"),
        "oracle parse error prefix"
    );
}

/// Generic: replay one oracle case's command corpus through the real
/// pi --mode rpc binary and assert the response records (semantically, order-
/// insensitive per record) match Pi's oracle stdout for that case.
fn assert_case_matches(mut names: Vec<&str>) {
    let oracle = oracle();
    for name in names.drain(..) {
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("case {name} in oracle"));
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy().into_owned();
        let input = case["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| serde_json::to_string(c).unwrap() + "\n")
            .collect::<String>();
        let got = records(&run_rpc(&input, &cwd));
        let expected: Vec<Value> = case["stdout"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            got.len(),
            expected.len(),
            "{name}: record count differs (got {got:#?} expected {expected:#?})"
        );
        // RPC is order-sensitive at the *record* level (the exact JSONL byte
        // stream a consumer reads), even though key order within a record is
        // not visible. Compare zipped in Pi's exact emission order.
        for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
            assert_eq!(g, e, "{name}: record {i} differs from Pi oracle");
        }
    }
}

#[test]
fn rpc_async_deterministic_commands_match_pi_oracle() {
    // Pi runs each RPC command as its own async task. Sync commands emit in
    // arrival order during input processing; awaited commands (depth-1:
    // steer/follow_up/abort) defer to microtask completion in FIFO order.
    // abort_bash is synchronous in rpc-mode.ts, so it emits first among the
    // mixed corpus — pinned by the oracle's exact record ordering
    // (r4, r1, r2, r3).
    assert_case_matches(vec!["async-steer-followup-abort"]);
}

#[test]
fn rpc_empty_fork_messages_matches_pi_byte_for_byte() {
    // Pi's `getUserMessagesForForking()` on an empty session returns a real
    // empty array, so `data` is `{messages: []}`. pi-rs previously serialized
    // the empty Lua table as `{}`; this asserts the Pi-correct empty-array
    // framing byte-for-byte. (The oracle's `session-fork-clone`/`fork-messages`
    // cases seed scripted forking messages pi-rs's real empty session doesn't
    // have, so only the empty case is data-faithful to a real run.)
    assert_case_matches(vec!["empty-fork-messages"]);
}

#[test]
fn rpc_empty_commands_matches_pi_byte_for_byte() {
    // Pi's `get_commands` on a session with no extension commands/prompts/
    // skills returns a real empty array, so `data` is `{commands: []}`.
    // pi-rs previously serialized the empty Lua table as `{}`.
    assert_case_matches(vec!["empty-commands"]);
}

#[test]
fn rpc_empty_messages_matches_pi_byte_for_byte() {
    // Pi's `get_messages` on an empty session returns a real empty array, so
    // `data` is `{messages: []}`. pi-rs previously serialized `{}`.
    assert_case_matches(vec!["empty-messages"]);
}

#[test]
fn rpc_await_depth_ordering_matches_pi_oracle() {
    // The two-phase scheduler reproduces Pi's microtask completion ORDER for
    // awaited commands. The oracle-covered cases (state-and-simple,
    // thinking-model-commands, export-html) also seed scripted session state
    // (a model, messages, a deterministic sessionId) that pi-rs's real empty
    // session produces from its own persistence, so the record DATA differs.
    // What the scheduler must match is the emitted record ORDER (the exact
    // JSONL stream a consumer reads): state-and-simple defers
    // get_available_models (depth 1) until after the sync queue;
    // thinking-model-commands resolves set_model (depth 2) after cycle_model
    // (depth 1) despite set_model arriving first; export-html defers
    // export_html (depth 1) after the sync get_session_stats.
    let oracle = oracle();
    for name in ["state-and-simple", "thinking-model-commands", "export-html"] {
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("case {name} in oracle"));
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy().into_owned();
        let input = case["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| serde_json::to_string(c).unwrap() + "\n")
            .collect::<String>();
        let got = records(&run_rpc(&input, &cwd));
        let expected_commands: Vec<String> = case["stdout"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str::<Value>(l)
                    .unwrap()["command"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let got_commands: Vec<String> = got
            .iter()
            .map(|r| {
                r["command"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            got_commands,
            expected_commands,
            "{name}: Pi record emission order not reproduced"
        );
    }
}

/// Run `pi --mode rpc --extension <file>` as a subprocess, returning
/// (stdout, stderr, exit status).
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

/// RPC loads CLI `--extension` files into the session runtime, like Pi's
/// createAgentSessionServices (which resolves parsed.extensions into the
/// runtime used by runRpcMode). A file that raises a sentinel error on load
/// must abort the RPC run — previously RPC silently ignored `--extension`.
#[test]
fn rpc_loads_cli_extension_files() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let ext = temp.path().join("boom.lua");
    std::fs::write(&ext, "local pi = ...\nerror(\"SENTINEL-EXT-LOADED\")\n").unwrap();

    let (stdout, stderr, code) = run_rpc_ext(
        "{\"type\":\"get_state\",\"id\":\"r1\"}\n",
        &cwd,
        &ext.to_string_lossy(),
    );
    // The extension load error aborts before any RPC record is produced and
    // the process exits nonzero with the sentinel on stderr.
    assert_eq!(stdout, "", "no RPC records when the CLI extension fails to load");
    assert_eq!(code, 1, "RPC exits nonzero on extension load failure");
    assert!(
        stderr.contains("SENTINEL-EXT-LOADED"),
        "extension load error surfaced on stderr, got: {stderr}"
    );
}

/// Output guard (spec: output-guard.ts takeOverStdout): during RPC mode,
/// stray Lua stdlib `print`/`io.write` from extension chunks route to stderr
/// so the stdout JSONL stream a consumer parses stays byte-clean, while the
/// product channel `pi.output` still writes to stdout.
#[test]
fn rpc_stdout_guard_routes_extension_stdout_to_stderr() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let ext = temp.path().join("nudge.lua");
    std::fs::write(
        &ext,
        "local pi = ...\n\
         print(\"EXT-PRINT\")\n\
         io.write(\"EXT-IOWRITE\")\n\
         io.stdout:write(\"EXT-HANDLE\")\n\
         io.output():write(\"EXT-DEFAULT\")\n\
         pi.output(\"EXT-PI-OUTPUT\\n\")\n",
    )
    .unwrap();

    let (stdout, stderr, code) = run_rpc_ext(
        "{\"type\":\"get_state\",\"id\":\"r1\"}\n",
        &cwd,
        &ext.to_string_lossy(),
    );
    assert_eq!(code, 0, "RPC completes successfully with a well-formed extension");
    // The extension's own `print`/`io.write` must NOT appear on stdout, so the
    // protocol stream a consumer reads stays clean of stray logging.
    assert!(!stdout.contains("EXT-PRINT"), "print must not pollute stdout");
    assert!(!stdout.contains("EXT-IOWRITE"), "io.write must not pollute stdout");
    assert!(!stdout.contains("EXT-HANDLE"), "io.stdout:write must not pollute stdout");
    assert!(!stdout.contains("EXT-DEFAULT"), "io.output():write must not pollute stdout");
    assert!(
        stderr.contains("EXT-PRINT")
            && stderr.contains("EXT-IOWRITE")
            && stderr.contains("EXT-HANDLE")
            && stderr.contains("EXT-DEFAULT"),
        "print/io.write/io.stdout/io.output all routed to stderr, got: {stderr:?}"
    );
    // The product channel writes to stdout and is unaffected by the guard.
    assert!(
        stdout.contains("EXT-PI-OUTPUT"),
        "pi.output still writes to stdout, got: {stdout:?}"
    );
    // The get_state record is intact on stdout.
    assert!(
        stdout.contains("\"command\":\"get_state\""),
        "get_state record present, got: {stdout:?}"
    );
}

/// RPC extension UI context (Plan 10 / 9.2). Pi's RPC mode binds a real
/// `ExtensionUIContext` (rpc-mode.ts `createExtensionUIContext`), so an
/// extension's context reports `mode == "rpc"` and `hasUI == true`, and
/// `ctx.ui.*` calls are transported to the client as `extension_ui_request`
/// JSONL records on stdout. This drives the real `pi --mode rpc` binary with
/// a CLI extension that reacts to `session_start`, reads its context, and
/// invokes `ctx.ui.notify` (fire-and-forget) — asserting both the faithful
/// binding and the request record on the protocol stream.
#[test]
fn rpc_binds_real_extension_ui_context_matching_pi() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let ext = temp.path().join("ui-probe.lua");
    std::fs::write(
        &ext,
        "local pi = ...\n\
         pi.on('session_start', function(event, ctx)\n\
           io.stderr:write('RPC_CTX mode=' .. tostring(ctx.mode) .. ' hasUI=' .. tostring(ctx.hasUI) .. '\\n')\n\
           if ctx.ui and ctx.ui.notify then\n\
             ctx.ui.notify('hello from extension')\n\
             io.stderr:write('RPC_CTX notify-callable=true\\n')\n\
           else\n\
             io.stderr:write('RPC_CTX notify-callable=false\\n')\n\
           end\n\
         end)\n",
    )
    .unwrap();

    let (stdout, stderr, code) = run_rpc_ext(
        "{\"type\":\"get_state\",\"id\":\"r1\"}\n",
        &cwd,
        &ext.to_string_lossy(),
    );
    assert_eq!(code, 0, "RPC completes successfully with a well-formed extension");
    // The extension observes a real RPC UI context: mode "rpc", hasUI true.
    assert!(
        stderr.contains("RPC_CTX mode=rpc hasUI=true"),
        "extension must observe rpc mode + hasUI=true, got: {stderr:?}"
    );
    assert!(
        stderr.contains("RPC_CTX notify-callable=true"),
        "ctx.ui.notify must be callable in RPC mode, got: {stderr:?}"
    );
    // Pi transports the notify as an `extension_ui_request` JSONL record on
    // the protocol stdout stream.
    let rec = records(&stdout)
        .into_iter()
        .find(|r| r["type"] == "extension_ui_request")
        .expect("extension_ui_request record on stdout");
    assert_eq!(rec["method"], "notify");
    assert_eq!(rec["message"], "hello from extension");
    assert!(rec["id"].as_str().is_some(), "request carries an id");
    // The protocol stream stays valid JSONL aside from the UI request slot.
    for r in records(&stdout) {
        assert!(r.is_object(), "each stdout record is an object");
    }
}
