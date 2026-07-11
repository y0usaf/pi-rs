#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_agent::PACK;
use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    let temp = tempfile::tempdir().expect("temp");
    let cwd = temp.keep();
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .expect("host");
    let report = host.load_embedded(&[PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

#[test]
fn errors_hooks_preparation_validation_and_updates_match_sequential_contract() {
    let host = host();
    host.load("test://tool-contract", r#"
      local pi=...
      pi.register_tool({name="echo",parameters={type="object",properties={value={type="string"}},required={"value"}},
        prepare_arguments=function(args) return {value=args.raw} end,
        execute=function(_id,args,_signal,update) update({content={{type="text",text="partial"}},details={}});
          if args.value=="explode" then error("boom",0) end
          return {content={{type="text",text=args.value}},details={}} end})
      pi.register_command("contract",{handler=function()
        local queue={
          {role="assistant",content={{type="toolCall",id="a",name="echo",arguments={raw="ok"}},
            {type="toolCall",id="b",name="missing",arguments={}},
            {type="toolCall",id="c",name="echo",arguments={bad=true}},
            {type="toolCall",id="d",name="echo",arguments={raw="explode"}}},stopReason="toolUse"},
          {role="assistant",content={{type="text",text="done"}},stopReason="stop"}}
        local function stream(_m,_c,_o,push) local x=table.remove(queue,1); x.api="a";x.provider="p";x.model="m";x.usage={};x.timestamp=0;push({type="done",message=x});return x end
        local events={}; local context={messages={},tools=pi.registered_tools()}
        local out=pi.agent.run_turn({},context,{model={provider="p"},
          beforeToolCall=function(e) if e.toolCall.id=="a" then e.args.value="mutated" end end,
          afterToolCall=function(e) if e.toolCall.id=="a" then return {content={{type="text",text=e.result.content[1].text.."!"}}} end end},
          function(e) events[#events+1]=e end,pi.abort_signal(),stream)
        return {out=out,events=events}
      end})
    "#).expect("load");
    let value = host
        .call_command("contract", "")
        .expect("call")
        .expect("result");
    assert_eq!(value["out"][1]["content"][0]["text"], "mutated!");
    assert_eq!(
        value["out"][2]["content"][0]["text"],
        "Tool missing not found"
    );
    assert!(
        value["out"][3]["content"][0]["text"]
            .as_str()
            .is_some_and(|s| s.contains("must have required properties")),
        "{}",
        value["out"][3]["content"][0]["text"]
    );
    assert_eq!(value["out"][4]["content"][0]["text"], "boom");
    let updates = value["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| e["type"] == "tool_execution_update")
        .count();
    assert_eq!(updates, 2);
}

/// Spec runner.ts `get model()`: the ExtensionContext handed to
/// tool.execute exposes the current agent model — the read tool's
/// non-vision image note depends on it (PLAN 5.2).
#[test]
fn execute_ctx_carries_the_current_model() {
    let host = host();
    host.load(
        "test://ctx-model",
        r#"
      local pi=...
      pi.register_tool({name="probe",parameters={type="object"},
        execute=function(_id,_args,_signal,_update,ctx)
          return {content={{type="text",text=ctx.model.id.."/"..table.concat(ctx.model.input,",")}},details={}} end})
      pi.register_command("probe-model",{handler=function()
        local queue={
          {role="assistant",content={{type="toolCall",id="a",name="probe",arguments={}}},stopReason="toolUse"},
          {role="assistant",content={{type="text",text="done"}},stopReason="stop"}}
        local function stream(_m,_c,_o,push) local x=table.remove(queue,1); x.api="a";x.provider="p";x.model="m";x.usage={};x.timestamp=0;push({type="done",message=x});return x end
        local context={messages={},tools=pi.registered_tools()}
        local out=pi.agent.run_turn({},context,
          {model={id="text-only",provider="p",input={"text"}}},
          function() end,pi.abort_signal(),stream)
        return {out=out}
      end})
    "#,
    )
    .expect("load");
    let value = host
        .call_command("probe-model", "")
        .expect("call")
        .expect("result");
    assert_eq!(value["out"][1]["content"][0]["text"], "text-only/text");
}
