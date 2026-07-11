#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_agent::PACK;
use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

#[test]
fn public_example_preserves_transcript_and_settles_after_terminal_listeners() {
    let host = host();
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/agent-lifecycle-demo.lua"
    ))
    .expect("example");
    host.load("examples/extensions/agent-lifecycle-demo.lua", &source)
        .expect("load");
    let value = host
        .call_command("agent-lifecycle-demo", "")
        .expect("call")
        .expect("value");
    assert_eq!(value["messageCount"], 4);
    assert_eq!(value["requestSizes"], serde_json::json!([1, 3]));
    assert_eq!(value["finalText"], "I remember Alice");
    assert_eq!(value["activeAtEnd"], true);
    assert_eq!(value["idleAtEnd"], false);
    assert_eq!(value["idleNow"], true);
    assert_eq!(
        value["events"],
        serde_json::json!([
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end",
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end"
        ])
    );
}

#[test]
fn reduction_precedes_listeners_and_run_failures_settle_once() {
    let host = host();
    host.load("test://lifecycle", r#"
      local pi = ...
      local model={id="m",name="m",api="a",provider="p",baseUrl="",reasoning=false,input={"text"},
        cost={input=0,output=0,cacheRead=0,cacheWrite=0},contextWindow=1,maxTokens=1}
      pi.register_command("lifecycle", {handler=function()
        local events, observations = {}, {}
        local agent
        agent=pi.agent.new({initialState={model=model},streamFn=function() error("provider exploded",0) end})
        agent:subscribe(function(event, signal)
          events[#events+1]=event.type
          local state=agent:get_state()
          if event.type=="message_start" then observations[#observations+1]=state.streamingMessage.role end
          if event.type=="message_end" then observations[#observations+1]=#state.messages end
          if event.type=="agent_start" then
            local ok, err=pcall(function() agent:prompt("overlap") end)
            observations[#observations+1]=ok
            observations[#observations+1]=err
          end
          assert(state.signal == signal)
        end)
        agent:prompt("hello")
        local state=agent:get_state()
        return {events=events,observations=observations,count=#state.messages,
          reason=state.messages[#state.messages].stopReason,
          message=state.messages[#state.messages].errorMessage,error=state.errorMessage,
          streaming=state.isStreaming,pending=next(state.pendingToolCalls)==nil}
      end})
    "#).expect("load");
    let value = host
        .call_command("lifecycle", "")
        .expect("call")
        .expect("value");
    assert_eq!(
        value["events"],
        serde_json::json!([
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end"
        ])
    );
    assert_eq!(value["count"], 2);
    assert_eq!(value["reason"], "error");
    assert_eq!(value["message"], "provider exploded");
    assert_eq!(value["error"], "provider exploded");
    assert_eq!(value["streaming"], false);
    assert_eq!(value["pending"], true);
    assert_eq!(value["observations"][0], false);
    assert!(
        value["observations"][1]
            .as_str()
            .unwrap()
            .contains("already processing a prompt")
    );
    assert_eq!(value["observations"][2], "user");
    assert_eq!(value["observations"][3], 1);
    assert_eq!(value["observations"][4], "assistant");
    assert_eq!(value["observations"][5], 2);
}

#[test]
fn continue_abort_reset_and_queued_assistant_tail_match_contract() {
    let host = host();
    host.load("test://continuation", r#"
      local pi=...
      local model={id="m",name="m",api="a",provider="p",baseUrl="",reasoning=false,input={"text"},cost={},contextWindow=1,maxTokens=1}
      local function msg(text,reason,err) return {role="assistant",content={{type="text",text=text}},api="a",provider="p",model="m",usage={},stopReason=reason or "stop",errorMessage=err,timestamp=0} end
      pi.register_command("continuation",{handler=function()
        local n, starts = 0, 0
        local agent=pi.agent.new({initialState={model=model,messages={{role="user",content="old",timestamp=0}}},streamFn=function(_m,c,o,push)
          n=n+1; local out
          if o.signal:is_aborted() then out=msg("","aborted","cancelled") else out=msg("response "..n) end
          push({type="done",message=out}); return out end})
        agent:subscribe(function(event)
          if event.type=="agent_start" then
            starts=starts+1
            if starts==2 then agent:abort() end
          end
        end)
        agent:continue()
        agent:follow_up({role="user",content="queued",timestamp=1})
        agent:continue()
        local before=agent:get_state()
        local before_count=#before.messages
        local before_reason=before.messages[#before.messages].stopReason
        agent:reset()
        local after=agent:get_state()
        local empty_ok,empty_err=pcall(function() agent:continue() end)
        agent:abort()
        return {before=before_count,reason=before_reason,
          queued=agent:has_queued_messages(),after=#after.messages,emptyOk=empty_ok,emptyErr=empty_err,
          prompt=after.systemPrompt,model=after.model.id}
      end})
    "#).expect("load");
    let value = host
        .call_command("continuation", "")
        .expect("call")
        .expect("value");
    assert_eq!(value["before"], 4);
    assert_eq!(value["reason"], "aborted");
    assert_eq!(value["queued"], false);
    assert_eq!(value["after"], 0);
    assert_eq!(value["emptyOk"], false);
    assert!(
        value["emptyErr"]
            .as_str()
            .unwrap()
            .contains("No messages to continue from")
    );
    assert_eq!(value["model"], "m");
}
