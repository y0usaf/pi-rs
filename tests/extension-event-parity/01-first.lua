local pi = ...
__event_trace = {}
local function log(event) __event_trace[#__event_trace + 1] = "first:" .. event.type end
for _, kind in ipairs({"session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"}) do
  pi.on(kind, function(event) log(event); if kind == "agent_end" then error("first agent error", 0) end end)
end
pi.on("resources_discover", function(event) log(event); return {skillPaths={"skills-a"}} end)
pi.on("session_before_switch", function(event) log(event); return {cancel=false,owner="first"} end)
pi.on("session_before_fork", function(event) log(event); return {cancel=false} end)
pi.on("session_before_compact", function(event) log(event); return {compaction={summary="hook summary",firstKeptEntryId="keep",tokensBefore=9,details={owner="first"}}} end)
pi.on("session_before_tree", function(event) log(event); return {customInstructions="first instructions",label="first-label"} end)
pi.on("context", function(event) log(event); local messages={} for _,m in ipairs(event.messages) do messages[#messages+1]=m end messages[#messages+1]={role="user",content="first",timestamp=0}; return {messages=messages} end)
pi.on("before_provider_request", function(event) log(event); local out={} for k,v in pairs(event.payload) do out[k]=v end out.first=true; return out end)
pi.on("before_agent_start", function(event) log(event); return {message={customType="first",content="notice",display=true},systemPrompt=event.systemPrompt.."|first"} end)
pi.on("message_end", function(event) log(event); if event.message.role ~= "assistant" then return end; for _,part in ipairs(event.message.content or {}) do if part.type == "toolCall" then return end end; local out={} for k,v in pairs(event.message) do out[k]=v end out.content={{type="text",text="first replacement"}}; return {message=out} end)
pi.register_tool({name="event_tool",label="Event Tool",description="event seam",parameters={type="object",properties={value={type="string"}},required={"value"}},execute=function(_,input) return {content={{type="text",text="tool:"..input.value}},details={base=true}} end})
pi.on("tool_call", function(event) log(event); if event.input.command then event.input.command=event.input.command.." --first" else event.input.first=true end; return {owner="first"} end)
pi.on("tool_result", function(event) log(event); return {content={{type="text",text="first result"}},details={first=true}} end)
pi.on("user_bash", function(event) log(event) end)
pi.on("input", function(event) log(event); return {action="transform",text=event.text.."|first"} end)
pi.on("project_trust", function(event) log(event); return {trusted="undecided"} end)
pi.register_command("event-trace", {handler=function() return __event_trace end})
