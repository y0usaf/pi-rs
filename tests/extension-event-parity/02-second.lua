local pi = ...
local function log(event) __event_trace[#__event_trace + 1] = "second:" .. event.type end
for _, kind in ipairs({"session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"}) do
  pi.on(kind, function(event) log(event) end)
end
pi.on("resources_discover", function(event) log(event); return {promptPaths={"prompts-b"},themePaths={"themes-b"}} end)
pi.on("session_before_switch", function(event) log(event); return {cancel=true,owner="second"} end)
pi.on("session_before_switch", function() __event_trace[#__event_trace + 1]="second:after-cancel" end)
pi.on("session_before_fork", function(event) log(event); return {cancel=true} end)
pi.on("session_before_compact", function(event) log(event); return {cancel=true} end)
pi.on("session_before_tree", function(event) log(event); return {summary={summary="tree summary",details={second=true}},replaceInstructions=true,label="second-label"} end)
pi.on("context", function(event) log(event); local messages={} for _,m in ipairs(event.messages) do messages[#messages+1]=m end messages[#messages+1]={role="user",content="second",timestamp=0}; return {messages=messages} end)
pi.on("before_provider_request", function(event) log(event); local out={} for k,v in pairs(event.payload) do out[k]=v end out.second=event.payload.first; return out end)
pi.on("before_agent_start", function(event) log(event); return {message={customType="second",content=event.systemPrompt,display=false},systemPrompt=event.systemPrompt.."|second"} end)
pi.on("message_end", function(event) log(event); return {message={role="user",content="invalid",timestamp=0}} end)
pi.on("tool_call", function(event) log(event); if event.toolName == "bash" then return {block=true,reason=event.input.command} end end)
pi.on("tool_result", function(event) log(event); return {isError=true,details={second=event.details.first}} end)
pi.on("user_bash", function(event) log(event); return {result={output="handled bash",exitCode=7,cancelled=false,truncated=false}} end)
pi.on("input", function(event) log(event); if event.text:find("handle",1,true) then return {action="handled"} end return {action="transform",text=event.text.."|second",images=event.images} end)
pi.on("project_trust", function(event) log(event); return {trusted="yes",remember=true} end)
