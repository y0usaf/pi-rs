-- Exercises WS4.3 through the registered WS3 write tool and the Lua loop.
local pi = ...

local MODEL = {
  id="fixture", name="fixture", api="anthropic-messages", provider="fixture", baseUrl="",
  reasoning=false, input={"text"}, cost={input=0,output=0,cacheRead=0,cacheWrite=0},
  contextWindow=1000, maxTokens=100,
}
local function message(content, reason)
  return { role="assistant", content=content, api=MODEL.api, provider=MODEL.provider,
    model=MODEL.id, usage={input=0,output=0,cacheRead=0,cacheWrite=0,totalTokens=0,
      cost={input=0,output=0,cacheRead=0,cacheWrite=0,total=0}},
    stopReason=reason or "stop", timestamp=0 }
end


pi.register_command("agent-tool-roundtrip-demo", { handler=function()
  local requests, events = {}, {}
  local context={systemPrompt="demo",messages={}} -- tools resolve from the public registry
  local calls=0
  local function fixture(_model, request, _options, push)
    calls=calls+1; requests[calls]=request
    local final
    if calls==1 then
      final=message({{type="toolCall",id="call-1",name="write",
        arguments={path="agent-gate.txt",content="from tool"}}},"toolUse")
    else final=message({{type="text",text="done"}}) end
    push({type="done",reason=final.stopReason,message=final}); return final
  end
  local result=pi.agent.run_turn({{role="user",content="write it",timestamp=0}},context,
    {model=MODEL},function(event) events[#events+1]=event.type end,pi.abort_signal(),fixture)
  return {calls=calls,events=events,result=result,second=requests[2],
    content=pi.fs.read_file(pi.path.resolve(pi.cwd(),"agent-gate.txt"))}
end })
