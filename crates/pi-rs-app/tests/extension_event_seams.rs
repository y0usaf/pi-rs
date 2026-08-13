#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! PLAN 9.3: differential pin for the three acceptance paths the fold oracle
//! does not cover — provider-failure (auto-retry re-drive), abort (mid-stream
//! signal), and reload (session replacement). The interactive runtime is driven
//! through its scripted streamFn seam (a real AgentSession over pi.agent.new),
//! and the loaded 03-seams trace extension records the extension event trace
//! via the shared pi.on handlers. Those traces and the final messages compare
//! strictly against the Pi-generated tests/extension-event-parity/
//! seams-oracle.json.

use std::path::Path;
use std::sync::Mutex;

use pi_rs_app::builtins::{AGENT_CORE_PACK, CODING_AGENT_PACK, INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn load_oracle() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/extension-event-parity/seams-oracle.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn setup(temp: &tempfile::TempDir) -> (String, String) {
    let agent_dir = temp.path().join("agent").to_string_lossy().into_owned();
    let sessions = temp.path().join("sessions").to_string_lossy().into_owned();
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        format!("{agent_dir}/config.lua"),
        "local pi = ...\npi.config.settings({ retry = { baseDelayMs = 1, maxRetries = 2 } })\n",
    )
    .unwrap();
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir) };
    (agent_dir, sessions)
}

fn run(
    cwd: &str,
    agent_dir: &str,
    session_dir: &str,
    case: serde_json::Value,
) -> serde_json::Value {
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[AGENT_CORE_PACK,
        pi_rs_agent::PACK,
        TOOLS_PACK,
        CODING_AGENT_PACK,
        INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.load(
        "03-seams",
        include_str!("../../../tests/extension-event-parity/03-seams.lua"),
    )
    .unwrap();
    let mut body = serde_json::json!({
        "cwd": cwd, "agentDir": agent_dir, "sessionDir": session_dir,
        "apiKey": "seams-key", "modelFromCli": true, "thinkingFromCli": true,
        "version": "0.79.0",
        "model": {
            "id": "claude-seams", "name": "Claude Seams", "provider": "anthropic",
            "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:1",
            "reasoning": false, "input": ["text"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 200000, "maxTokens": 1024
        }
    });
    body.as_object_mut()
        .unwrap()
        .extend(case.as_object().unwrap().clone());
    host.call_command("extension-event-seams-parity", &body.to_string())
        .unwrap()
        .unwrap()
}

#[test]
fn provider_failure_reload_and_abort_paths_follow_pi_seam_oracle() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let expected = load_oracle();

    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("project").to_string_lossy().into_owned();
    std::fs::create_dir_all(&cwd).unwrap();
    let (agent_dir, session_dir) = setup(&temp);

    // provider-failure: error turn auto-retries into a recovered turn.
    let provider = run(
        &cwd,
        &agent_dir,
        &session_dir,
        serde_json::json!({
            "case": "providerFailure",
            "turns": [
                {"stopReason": "error", "errorMessage": "overloaded_error"},
                {"stopReason": "stop", "text": "recovered"}
            ]
        }),
    );
    assert_eq!(provider["trace"], expected["providerFailure"]["trace"]);
    assert_eq!(
        provider["callCount"],
        expected["providerFailure"]["callCount"]
    );
    assert_eq!(
        provider["messages"],
        expected["providerFailure"]["messages"]
    );

    // reload: session_shutdown then session_start then resources_discover.
    let reload = run(
        &cwd,
        &agent_dir,
        &session_dir,
        serde_json::json!({"case": "reload"}),
    );
    assert_eq!(reload["trace"], expected["reload"]["trace"]);

    // abort: a long response is aborted mid-stream after an update delta.
    let temp2 = tempfile::tempdir().unwrap();
    let cwd2 = temp2.path().join("project").to_string_lossy().into_owned();
    std::fs::create_dir_all(&cwd2).unwrap();
    let (agent_dir2, session_dir2) = setup(&temp2);
    let abort = run(
        &cwd2,
        &agent_dir2,
        &session_dir2,
        serde_json::json!({"case": "abort", "turns": [{"abort": true}]}),
    );
    assert_eq!(abort["trace"], expected["abort"]["trace"]);
    assert_eq!(abort["messages"], expected["abort"]["messages"]);
}
