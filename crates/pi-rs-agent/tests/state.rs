#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_agent::PACK;
use pi_rs_host::{Host, HostConfig};

#[test]
fn defaults_and_mutators_match_agent_state_contract() {
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/agent-state-demo.lua"
    ))
    .expect("example");
    host.load("examples/extensions/agent-state-demo.lua", &source)
        .expect("load example");
    let result = host
        .call_command("agent-state-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(
        result,
        serde_json::json!({
            "systemPrompt": "demo",
            "thinkingLevel": "low",
            "messageCount": 1,
            "isStreaming": false,
            "transport": "websocket",
        })
    );
}

#[test]
fn assigned_arrays_are_copied_and_reset_retains_configuration() {
    let host = Host::new(HostConfig::default()).expect("host");
    host.load_embedded(&[PACK]);
    host.load(
        "test://state",
        r#"
        local pi = ...
        pi.register_command("state-copy", {
          handler = function()
            local input = {{ name = "one" }}
            local agent = pi.agent.new({ initialState = { systemPrompt = "keep" } })
            agent:set_tools(input)
            input[1] = { name = "changed" }
            agent:steer({ role = "user" })
            agent:reset()
            local state = agent:get_state()
            return { prompt = state.systemPrompt, tool = state.tools[1].name,
                     queued = agent:has_queued_messages(), messages = #state.messages }
          end,
        })
    "#,
    )
    .expect("load");
    assert_eq!(
        host.call_command("state-copy", "").expect("call"),
        Some(serde_json::json!({
            "prompt": "keep", "tool": "one", "queued": false, "messages": 0
        }))
    );
}
