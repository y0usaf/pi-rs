#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_agent::PACK;
use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

const PRELUDE: &str = r#"
local pi = ...
local model={id="m",name="m",api="a",provider="p",baseUrl="",reasoning=false,input={"text"},cost={},contextWindow=1,maxTokens=1}
local function assistant(content, reason)
  return {role="assistant",content=content,api="a",provider="p",model="m",usage={},stopReason=reason or "stop",timestamp=0}
end
local function user(text) return {role="user",content=text,timestamp=0} end
"#;

#[test]
fn parallel_tool_ends_follow_completion_but_results_follow_source() {
    let host = host();
    let source = format!(
        r#"{PRELUDE}
pi.register_command("parallel-policy", {{handler=function()
  local calls, events = 0, {{}}
  local tools={{{{name="echo",parameters={{type="object",properties={{value={{type="string"}}}},required={{"value"}}}},
    execute=function(_id,args) if args.value=="first" then pi.sleep(30) else pi.sleep(5) end
      return {{content={{{{type="text",text=args.value}}}},details={{value=args.value}}}} end}}}}
  local function stream(_m,_c,_o,push)
    calls=calls+1; local out
    if calls==1 then out=assistant({{{{type="toolCall",id="one",name="echo",arguments={{value="first"}}}},
      {{type="toolCall",id="two",name="echo",arguments={{value="second"}}}}}},"toolUse")
    else out=assistant({{{{type="text",text="done"}}}}) end
    push({{type="done",message=out}}); return out
  end
  local result=pi.agent.run_turn({{user("go")}},{{systemPrompt="",messages={{}},tools=tools}},
    {{model=model}},function(e) events[#events+1]=e end,nil,stream)
  local ends, persisted={{}},{{}}
  for _,e in ipairs(events) do
    if e.type=="tool_execution_end" then ends[#ends+1]=e.toolCallId end
    if e.type=="message_end" and e.message.role=="toolResult" then persisted[#persisted+1]=e.message.toolCallId end
  end
  local resultIds={{}}
  for _,m in ipairs(result) do if m.role=="toolResult" then resultIds[#resultIds+1]=m.toolCallId end end
  return {{ends=ends,persisted=persisted,resultIds=resultIds,calls=calls}}
end}})
"#
    );
    host.load("test://parallel-policy", &source).expect("load");
    let value = host
        .call_command("parallel-policy", "")
        .expect("call")
        .expect("value");
    assert_eq!(value["ends"], serde_json::json!(["two", "one"]));
    assert_eq!(value["persisted"], serde_json::json!(["one", "two"]));
    assert_eq!(value["resultIds"], serde_json::json!(["one", "two"]));
    assert_eq!(value["calls"], 2);
}

#[test]
fn turn_hooks_queues_and_batch_termination_match_reference_order() {
    let host = host();
    let source = format!(
        r#"{PRELUDE}
pi.register_command("policy", {{handler=function()
  local calls, order, steeringPolls, followPolls = 0, {{}}, 0, 0
  local queued={{user("steer")}}; local follow={{user("follow")}}
  local terminating={{{{name="finish",parameters={{type="object",properties={{}}}},
    execute=function() return {{content={{{{type="text",text="finished"}}}},details={{}},terminate=true}} end}}}}
  local function stream(m,c,o,push)
    calls=calls+1; order[#order+1]="stream:"..calls..":"..m.id..":"..(o.reasoning or "off")..":"..c.systemPrompt
    local out
    if calls==1 then out=assistant({{{{type="toolCall",id="done",name="finish",arguments={{}}}}}},"toolUse")
    else out=assistant({{{{type="text",text="answer"}}}}) end
    push({{type="done",message=out}}); return out
  end
  local config={{model=model,
    getSteeringMessages=function() steeringPolls=steeringPolls+1; local q=queued; queued={{}}; return q end,
    getFollowUpMessages=function() followPolls=followPolls+1; local q=follow; follow={{}}; return q end,
    prepareNextTurn=function(turn) order[#order+1]="prepare:"..#turn.context.messages
      if calls==1 then return {{context={{systemPrompt="replaced",messages=turn.context.messages,tools=turn.context.tools}},
        model={{id="m2",name="m2",api="a",provider="p"}},thinkingLevel="high"}} end end,
    shouldStopAfterTurn=function(turn) order[#order+1]="stop:"..#turn.newMessages; return calls==2 end}}
  local result=pi.agent.run_turn({{user("prompt")}},{{systemPrompt="old",messages={{}},tools=terminating}},config,
    function(e) order[#order+1]=e.type end,nil,stream)
  return {{calls=calls,order=order,steeringPolls=steeringPolls,followPolls=followPolls,count=#result}}
end}})
"#
    );
    host.load("test://policy", &source).expect("load");
    let value = host
        .call_command("policy", "")
        .expect("call")
        .expect("value");
    assert_eq!(value["calls"], 2);
    assert_eq!(value["steeringPolls"], 2);
    assert_eq!(value["followPolls"], 1);
    let order = value["order"].as_array().expect("order");
    let strings: Vec<_> = order.iter().map(|v| v.as_str().expect("string")).collect();
    assert!(strings.contains(&"stream:1:m:off:old"));
    assert!(strings.contains(&"stream:2:m2:high:replaced"));
    let turn = strings.iter().position(|v| *v == "turn_end").expect("turn");
    let prepare = strings
        .iter()
        .position(|v| v.starts_with("prepare:"))
        .expect("prepare");
    let stop = strings
        .iter()
        .position(|v| v.starts_with("stop:"))
        .expect("stop");
    assert!(turn < prepare && prepare < stop);
}

#[test]
fn stateful_queue_modes_drain_all_and_prioritize_steering() {
    let host = host();
    let source = format!(
        r#"{PRELUDE}
pi.register_command("queue-modes", {{handler=function()
  local requests={{}}
  local agent=pi.agent.new({{initialState={{model=model}},steeringMode="all",followUpMode="all",
    streamFn=function(_m,c,_o,push)
      local users={{}}; for _,msg in ipairs(c.messages) do if msg.role=="user" then users[#users+1]=msg.content end end
      requests[#requests+1]=users
      local out=assistant({{{{type="text",text="ok"}}}}); push({{type="done",message=out}}); return out
    end}})
  agent:steer(user("s1")); agent:steer(user("s2")); agent:follow_up(user("f1")); agent:follow_up(user("f2"))
  agent:prompt(user("prompt"))
  return {{requests=requests,queued=agent:has_queued_messages(),steering=agent:get_steering_mode(),follow=agent:get_follow_up_mode()}}
end}})
"#
    );
    host.load("test://queue-modes", &source).expect("load");
    let value = host
        .call_command("queue-modes", "")
        .expect("call")
        .expect("value");
    assert_eq!(
        value["requests"],
        serde_json::json!([["prompt", "s1", "s2"], ["prompt", "s1", "s2", "f1", "f2"]])
    );
    assert_eq!(value["queued"], false);
    assert_eq!(value["steering"], "all");
    assert_eq!(value["follow"], "all");
}

#[test]
fn per_tool_sequential_mode_overrides_parallel_default() {
    let host = host();
    let source = format!(
        r#"{PRELUDE}
pi.register_command("sequential-override", {{handler=function()
  local calls, ends = 0, {{}}
  local tools={{{{name="slow",executionMode="sequential",parameters={{type="object",properties={{value={{type="string"}}}}}},
    execute=function(_id,args) if args.value=="first" then pi.sleep(25) else pi.sleep(2) end
      return {{content={{{{type="text",text=args.value}}}},details={{}}}} end}}}}
  local function stream(_m,_c,_o,push)
    calls=calls+1; local out
    if calls==1 then out=assistant({{{{type="toolCall",id="one",name="slow",arguments={{value="first"}}}},
      {{type="toolCall",id="two",name="slow",arguments={{value="second"}}}}}},"toolUse")
    else out=assistant({{{{type="text",text="done"}}}}) end
    push({{type="done",message=out}}); return out
  end
  pi.agent.run_turn({{user("go")}},{{systemPrompt="",messages={{}},tools=tools}},{{model=model}},
    function(e) if e.type=="tool_execution_end" then ends[#ends+1]=e.toolCallId end end,nil,stream)
  return ends
end}})
"#
    );
    host.load("test://sequential-override", &source)
        .expect("load");
    let value = host
        .call_command("sequential-override", "")
        .expect("call")
        .expect("value");
    assert_eq!(value, serde_json::json!(["one", "two"]));
}
