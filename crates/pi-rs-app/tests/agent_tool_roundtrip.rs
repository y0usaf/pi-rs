#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_app::builtins::TOOLS_PACK;
use pi_rs_host::EmbeddedPack;
use pi_rs_host::{Host, HostConfig};

#[test]
fn public_path_runs_registered_tool_then_sends_result_to_request_two() {
    let temp = tempfile::tempdir().expect("temp");
    let host = Host::new(HostConfig {
        cwd: Some(temp.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .expect("host");
    let agent_pack = EmbeddedPack {
        name: "pi-rs-agent",
        source: include_str!("../../pi-rs-agent/lua/agent.lua"),
    };
    let report = host.load_embedded(&[TOOLS_PACK, agent_pack]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/agent-tool-roundtrip-demo.lua"
    );
    let source = std::fs::read_to_string(path).expect("example");
    host.load("agent-tool-roundtrip-demo.lua", &source)
        .expect("load");
    let value = host
        .call_command("agent-tool-roundtrip-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(value["calls"], 2);
    assert_eq!(value["content"], "from tool");
    assert_eq!(value["second"]["messages"][1]["role"], "assistant");
    assert_eq!(value["second"]["messages"][2]["role"], "toolResult");
    assert_eq!(value["second"]["messages"][2]["toolCallId"], "call-1");
    assert_eq!(value["result"][1]["stopReason"], "toolUse");
    assert_eq!(value["result"][2]["role"], "toolResult");
    assert_eq!(value["result"][3]["content"][0]["text"], "done");
    assert_eq!(
        value["events"],
        serde_json::json!([
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_end",
            "tool_execution_start",
            "tool_execution_end",
            "message_start",
            "message_end",
            "turn_end",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end"
        ])
    );
}
