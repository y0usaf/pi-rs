#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_agent::PACK;
use pi_rs_host::{Host, HostConfig};

fn host_with_pack() -> Host {
    let host = Host::new(HostConfig::default()).expect("host");
    let report = host.load_embedded(&[PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

#[test]
fn public_example_replays_text_and_replaces_partial_transcript() {
    let host = host_with_pack();
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/agent-turn-demo.lua"
    ))
    .expect("example");
    host.load("examples/extensions/agent-turn-demo.lua", &source)
        .expect("load example");
    let result = host
        .call_command("agent-turn-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(result["text"], "hello");
    assert_eq!(result["transcriptText"], "hello");
    assert_eq!(
        result["events"],
        serde_json::json!([
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_update",
            "message_update",
            "message_end",
            "turn_end",
            "agent_end"
        ])
    );
}

#[test]
fn transform_convert_key_resolution_and_replaceable_stream_match_spec_order() {
    let host = host_with_pack();
    host.load(
        "test://stream-order",
        r#"
        local pi = ...
        local model = {
          id="m", name="m", api="anthropic-messages", provider="dynamic",
          baseUrl="", reasoning=false, input={"text"},
          cost={input=0,output=0,cacheRead=0,cacheWrite=0}, contextWindow=1,maxTokens=1,
        }
        local function msg(text)
          return { role="assistant", content={{type="text",text=text}}, api=model.api,
            provider=model.provider, model=model.id, stopReason="stop", timestamp=0,
            usage={input=0,output=0,cacheRead=0,cacheWrite=0,totalTokens=0,
              cost={input=0,output=0,cacheRead=0,cacheWrite=0,total=0}} }
        end
        pi.register_command("stream-order", { handler=function()
          local order, observed = {}, {}
          local context={systemPrompt="system",messages={
            {role="notification",text="ignore",timestamp=0},
            {role="user",content="old",timestamp=0}},tools={}}
          local config={model=model,apiKey="stale",
            transformContext=function(messages, signal)
              order[#order+1]="transform"
              observed.signal=not signal:is_aborted()
              return {messages[#messages]}
            end,
            convertToLlm=function(messages)
              order[#order+1]="convert"
              observed.converted=#messages
              return messages
            end,
            getApiKey=function(provider)
              order[#order+1]="key"
              observed.provider=provider
              return "fresh"
            end}
          local function custom(_model, request, options, on_event)
            order[#order+1]="stream"
            observed.key=options.apiKey
            observed.requestCount=#request.messages
            local final=msg("done")
            on_event({type="done",reason="stop",message=final})
            return final
          end
          local events={}
          local result=pi.agent.run_turn({},context,config,
            function(e) events[#events+1]=e.type end,pi.abort_signal(),custom)
          return {order=order,observed=observed,events=events,text=result[1].content[1].text}
        end})
        "#,
    )
    .expect("load");
    let result = host
        .call_command("stream-order", "")
        .expect("command")
        .expect("result");
    assert_eq!(
        result["order"],
        serde_json::json!(["transform", "convert", "key", "stream"])
    );
    assert_eq!(result["observed"]["signal"], true);
    assert_eq!(result["observed"]["converted"], 1);
    assert_eq!(result["observed"]["provider"], "dynamic");
    assert_eq!(result["observed"]["key"], "fresh");
    assert_eq!(result["observed"]["requestCount"], 1);
    assert_eq!(result["text"], "done");
    assert_eq!(
        result["events"],
        serde_json::json!([
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end"
        ])
    );
}

#[test]
fn thinking_and_fragmented_tool_call_fixtures_emit_updates_and_final_content() {
    let host = host_with_pack();
    host.load(
        "test://stream-fixtures",
        r#"
        local pi = ...
        local model={id="m",name="m",api="anthropic-messages",provider="p",baseUrl="",
          reasoning=true,input={"text"},cost={input=0,output=0,cacheRead=0,cacheWrite=0},
          contextWindow=1,maxTokens=1}
        local usage={input=0,output=0,cacheRead=0,cacheWrite=0,totalTokens=0,
          cost={input=0,output=0,cacheRead=0,cacheWrite=0,total=0}}
        local function msg(content,reason) return {role="assistant",content=content,api=model.api,
          provider=model.provider,model=model.id,usage=usage,stopReason=reason,timestamp=0} end
        pi.register_command("stream-fixtures", {handler=function()
          local context={messages={},tools={}}
          local updates={}
          local function fixture(_m,_c,_o,push)
            local partial=msg({},"toolUse")
            push({type="start",partial=partial})
            partial=msg({{type="thinking",thinking="plan"}},"toolUse")
            push({type="thinking_start",contentIndex=0,partial=partial})
            push({type="thinking_delta",contentIndex=0,delta="plan",partial=partial})
            push({type="thinking_end",contentIndex=0,content="plan",partial=partial})
            partial=msg({{type="thinking",thinking="plan"},
              {type="toolCall",id="call-1",name="read",arguments={}}},"toolUse")
            push({type="toolcall_start",contentIndex=1,partial=partial})
            push({type="toolcall_delta",contentIndex=1,delta='{"path":',partial=partial})
            partial=msg({{type="thinking",thinking="plan"},
              {type="toolCall",id="call-1",name="read",arguments={path="a.txt"}}},"toolUse")
            push({type="toolcall_delta",contentIndex=1,delta='"a.txt"}',partial=partial})
            push({type="toolcall_end",contentIndex=1,toolCall=partial.content[2],partial=partial})
            push({type="done",reason="toolUse",message=partial})
            return partial
          end
          local final=pi.agent.stream_assistant_response(context,{model=model},pi.abort_signal(),
            function(e) if e.type=="message_update" then
              updates[#updates+1]=e.assistantMessageEvent.type end end,fixture)
          return {updates=updates,thinking=final.content[1].thinking,
            tool=final.content[2].name,path=final.content[2].arguments.path,
            transcriptPath=context.messages[1].content[2].arguments.path}
        end})
        "#,
    )
    .expect("load");
    let result = host
        .call_command("stream-fixtures", "")
        .expect("command")
        .expect("result");
    assert_eq!(
        result["updates"],
        serde_json::json!([
            "thinking_start",
            "thinking_delta",
            "thinking_end",
            "toolcall_start",
            "toolcall_delta",
            "toolcall_delta",
            "toolcall_end"
        ])
    );
    assert_eq!(result["thinking"], "plan");
    assert_eq!(result["tool"], "read");
    assert_eq!(result["path"], "a.txt");
    assert_eq!(result["transcriptPath"], "a.txt");
}
