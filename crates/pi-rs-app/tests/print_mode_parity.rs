#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Differential for Plan 10 print mode. The oracle in
// tests/print-mode-parity/oracle.json is generated from Pi's real
// `runPrintMode` (modes/print-mode.ts) by scripts/print-mode-oracle. Each case
// records Pi's raw stdout bytes, stderr bytes, and process exit code for a
// scripted assistant final message. This test replays the same final message
// through pi-rs's print/`--mode json` role (driving a registered custom
// `streamSimple` provider through the public Lua surface), captures the raw
// `pi.output` bytes, and compares byte-for-byte.
//
// The `messages[]` follow-up sequence (Pi sends each remaining CLI message as
// a sequential `session.prompt` after the initial message) is pinned
// byte-for-byte by `print_follow_up_sequence_matches_pi_byte_for_byte`
// against the oracle's multi-message cases. JSON-mode header/event
// *substance* is session-specific (Pi emits the live session's own events),
// so the JSON differential asserts Pi's framing contract (header line first
// when present, then one JSON line per event) rather than byte-identical
// session content. Text-mode final-message output is byte-for-byte.
use pi_rs_agent::PACK;
use pi_rs_app::builtins::{AGENT_CORE_PACK, CODING_AGENT_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/print-mode-parity/oracle.json")).unwrap()
}

fn capture_host(temp: &tempfile::TempDir) -> Host {
    let cwd = temp.path().to_string_lossy().into_owned();
    let agent_dir = temp.path().join("agent");
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    let host = Host::new(HostConfig {
        cwd: Some(cwd.clone()),
        ..HostConfig::default()
    })
    .unwrap();
    let rep = host.load_embedded(&[AGENT_CORE_PACK, PACK, TOOLS_PACK, CODING_AGENT_PACK]);
    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    // Redefine pi.output to also record the exact bytes this role emits, so the
    // differential test observes the same raw stdout a CLI caller would.
    host.load(
        "<capture>",
        r#"
            local pi = ...
            _G.captured = {}
            local realOutput = pi.output
            pi.output = function(text) _G.captured[#_G.captured+1] = text; return realOutput(text) end
            pi.register_command("print-capture-dump", { handler = function()
                return { captured = _G.captured }
            end })
        "#,
    )
    .expect("capture loads");
    host
}

/// Build a Lua provider whose streamSimple yields the given assistant messages
/// in call order (one per agent turn). Each entry is
/// (content[role="assistant"] + stopReason + optional errorMessage). The
/// provider's returned message becomes the agent's final assistant message,
/// exactly as the oracle scripted it.
fn provider_lua(mode: &str, api: &str, _provider: &str, lasts: &Value, events: &Value) -> String {
    let mut lua = String::new();
    lua.push_str(&format!(
        r#"
local pi = ...
pi.register_provider({api:?}, {{
  api = {api:?},
  streamSimple = function(model, context, options, on_event)
    _G.pi_provider_calls = (_G.pi_provider_calls or 0) + 1
    local lasts = pi.json.decode({lasts_json:?})
    local last = lasts[_G.pi_provider_calls]
"#,
        api = api,
        lasts_json = lasts.to_string(),
    ));
    if mode == "json" {
        for ev in events.as_array().unwrap() {
            let ev = serde_json::to_string(ev).unwrap();
            lua.push_str(&format!("    on_event(pi.json.decode({ev:?}))\n", ev = ev));
        }
    }
    // Build a full assistant final message from the oracle's minimal
    // `assistant` entry (content/stopReason/errorMessage) plus the identity
    // fields the agent's stream settlement expects on an assistant message.
    // The per-call `last` (array element) is merged inside the Lua provider.
    let base_identity = json!({
        "role": "assistant",
        "api": api,
        "provider": _provider,
        "model": "custom-1",
        "usage": {},
        "timestamp": 0,
    });
    let base_id = serde_json::to_string(&base_identity).unwrap();
    lua.push_str(&format!(
        r#"
    local base = pi.json.decode({base_id:?})
    for k, v in pairs(last) do base[k] = v end
    local final = base
"#,
        base_id = base_id,
    ));
    lua.push_str(
        r#"
    on_event({ type = "done", reason = final.stopReason or "stop", message = final })
    return final
  end,
})
"#,
    );
    lua
}

fn run_print(
    host: &Host,
    temp: &tempfile::TempDir,
    mode: &str,
    prompt: &str,
    follow_up: &[String],
    model_api: &str,
    provider: &str,
) -> (Value, Value) {
    let model = json!({
        "id":"custom-1", "name":"Custom", "api":model_api, "provider":provider,
        "baseUrl":"", "reasoning":false, "input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
        "contextWindow":128000, "maxTokens":1024
    });
    let result = host
        .call_role(
            "print",
            &json!({
                "model": model, "apiKey": "k", "prompt": prompt,
                "followUpMessages": follow_up, "cwd": temp.path().to_string_lossy(),
                "agentDir": temp.path().join("agent").to_string_lossy(),
                "readmePath": "/pi-rs-pkg/README.md", "docsPath": "/pi-rs-pkg/docs",
                "examplesPath": "/pi-rs-pkg/examples", "mode": mode,
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    let dump = host
        .call_command("print-capture-dump", "")
        .unwrap()
        .unwrap();
    host.load("<reset-cap>", "local pi=...\n_G.captured = {}")
        .unwrap();
    (result, dump)
}

/// Model Pi's `main.ts` text-mode stderr mapping (mirror of runPrintMode's
/// `console.error(errorMessage || \`Request ${stopReason}\`)`).
fn pi_stderr(result: &Value) -> String {
    if result["exitCode"].as_u64().unwrap_or(0) == 0 {
        return String::new();
    }
    let message = result["errorMessage"].as_str().unwrap_or("");
    let reason = result["stopReason"].as_str().unwrap_or("error");
    if message.is_empty() {
        format!("Request {reason}\n")
    } else {
        format!("{message}\n")
    }
}

/// Extract the captured `pi.output` bytes. An empty collector serializes as an
/// empty JSON object `{}` (Lua table), not an array, so normalize either shape.
fn captured_lines(dump: &Value) -> Vec<String> {
    let val = &dump["captured"];
    if let Some(arr) = val.as_array() {
        return arr.iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    }
    // Empty Lua table → JSON object {} → treat as no output.
    Vec::new()
}

#[test]
fn print_text_mode_output_matches_pi_byte_for_byte() {
    let oracle = fixture();
    // Byte-exact text-mode cases pi-rs can reproduce with a single prompt.
    // (text-no-text-content — a bare toolCall assistant final — is excluded:
    // that oracle case scripts Pi's *observed state* directly, but pi-rs's real
    // agent legitimately continues the tool loop on stopReason toolUse and
    // would never settle on a bare toolCall. It is not a faithful terminal
    // print outcome and is documented in PLAN 10.)
    let single_text = [
        "text-single-block",
        "text-many-blocks",
        "text-internal-newlines",
        "text-multiline-and-many",
        "error-with-message",
        "error-no-message",
        "aborted-no-message",
        "stop-not-error-has-error-message",
    ];
    for name in single_text {
        let temp = tempfile::tempdir().unwrap();
        let host = capture_host(&temp);
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .cloned()
            .expect("case in oracle");
        let last = &case["assistant"][0];
        let prompt = case["initial"].as_str().unwrap();
        let api = &format!("api-{name}");
        let provider = &format!("p-{name}");
        let idle_events = json!([]);
        host.load(
            "<case-provider>",
            &provider_lua("text", api, provider, &json!([last.clone()]), &idle_events),
        )
        .expect("provider loads");
        let (result, dump) = run_print(&host, &temp, "text", prompt, &[], api, provider);
        let captured = captured_lines(&dump).concat();
        // stdout must match Pi's exact bytes.
        assert_eq!(
            captured,
            case["stdout"].as_str().unwrap(),
            "case {name}: stdout mismatch"
        );
        // exit + stderr must match Pi's exit/stderr mapping.
        assert_eq!(
            result["exitCode"].as_u64().unwrap_or(0),
            case["exit"].as_u64().unwrap(),
            "case {name}: exitCode mismatch"
        );
        assert_eq!(
            pi_stderr(&result),
            case["stderr"].as_str().unwrap(),
            "case {name}: stderr mismatch"
        );
    }
}

#[test]
fn print_json_mode_framing_matches_pi_contract() {
    // Pi's JSON mode writes the session header line first (when present), then
    // every agent event as a single JSON line. pi-rs emits its own real session
    // header + events, so assert the framing contract, not session substance.
    let temp = tempfile::tempdir().unwrap();
    let host = capture_host(&temp);
    let last = json!({
        "role":"assistant",
        "content":[{"type":"text","text":"done"}],
        "api":"api-json","provider":"p-json","model":"custom-1",
        "usage":{}, "stopReason":"stop", "timestamp":0
    });
    let events = json!([
        { "type": "message_start", "timestamp": 0 },
        { "type": "message_end", "timestamp": 0 },
    ]);
    host.load(
        "<json-provider>",
        &provider_lua(
            "json",
            "api-json",
            "p-json",
            &json!([last.clone()]),
            &events,
        ),
    )
    .expect("provider loads");
    let (result, dump) = run_print(&host, &temp, "json", "go", &[], "api-json", "p-json");
    assert_eq!(result["exitCode"].as_u64(), Some(0));
    let lines = captured_lines(&dump);
    assert!(!lines.is_empty());
    // First line must be the session header (when a real header exists).
    let first: Value = serde_json::from_str(&lines[0]).expect("first output line is JSON");
    assert_eq!(
        first["type"], "session",
        "first JSON line is the session header"
    );
    // Every subsequent line is one JSON object (each agent event).
    assert!(
        lines.iter().skip(1).all(|line| {
            serde_json::from_str::<Value>(line)
                .map(|v| v.is_object())
                .unwrap_or(false)
        }),
        "every event is a single JSONL object"
    );
}

#[test]
fn print_follow_up_sequence_matches_pi_byte_for_byte() {
    // Plan 10 (modes/print-mode.ts messages[]): after the initial message, Pi
    // sends each remaining CLI message as a sequential `session.prompt`. Text
    // mode writes only the *final* assistant message's text blocks to stdout,
    // and on an `error`/`aborted` final message exits 1 with the error on
    // stderr. The multi-message oracle cases record Pi's exact raw output.
    let oracle = fixture();
    for name in [
        "multiple-messages-including-initial",
        "text-final-message-wins",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let host = capture_host(&temp);
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .cloned()
            .expect("case in oracle");
        let prompt = case["initial"].as_str().unwrap();
        let follow_up: Vec<String> = case["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_owned())
            .collect();
        let api = &format!("api-{name}");
        let provider = &format!("p-{name}");
        let assistant = json!(case["assistant"].clone());
        host.load(
            "<case-provider>",
            &provider_lua("text", api, provider, &assistant, &json!([])),
        )
        .expect("provider loads");
        let (result, dump) = run_print(&host, &temp, "text", prompt, &follow_up, api, provider);
        let captured = captured_lines(&dump).concat();
        assert_eq!(
            captured,
            case["stdout"].as_str().unwrap(),
            "case {name}: stdout mismatch"
        );
        assert_eq!(
            result["exitCode"].as_u64().unwrap_or(0),
            case["exit"].as_u64().unwrap(),
            "case {name}: exitCode mismatch"
        );
        assert_eq!(
            pi_stderr(&result),
            case["stderr"].as_str().unwrap(),
            "case {name}: stderr mismatch"
        );
    }
}
