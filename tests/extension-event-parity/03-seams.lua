-- PLAN 9.3 seam differential. Registers the full event vocabulary via pi.on and
-- records each event as a stable, normalized line so the trace compares against
-- the Pi-generated seams-oracle. Collapses consecutive identical lines (Pi's own
-- test suite normalizes duplicate message_update deltas, which are
-- timing-dependent counts).
local pi = ...
local lines = {}
local function record(line)
  if lines[#lines] == line then return end
  lines[#lines + 1] = line
end
for _, type in ipairs({
  "input", "before_agent_start", "agent_start", "agent_end",
  "turn_start", "turn_end", "message_start", "message_update", "message_end",
  "context", "session_start", "session_shutdown", "resources_discover",
}) do
  pi.on(type, function(event)
    local line = "ext:" .. type
    if type == "agent_start" or type == "agent_end" then
      line = line .. ":" .. tostring(not event.messages and 0 or #event.messages)
      if type == "agent_end" then line = line .. ":" .. tostring(event.willRetry == true) end
    elseif type == "turn_start" or type == "turn_end" then
      line = line .. ":" .. tostring(event.turnIndex)
    elseif type == "message_start" or type == "message_end" or type == "message_update" then
      line = line .. ":" .. tostring(event.message and event.message.role or "-")
      if type == "message_end" and event.message and event.message.role == "assistant" then
        line = line .. ":" .. tostring(event.message.stopReason or "")
            .. ":" .. tostring(event.message.errorMessage or "")
      end
    elseif type == "resources_discover" then
      line = line .. ":" .. tostring(event.reason)
    elseif type == "session_start" or type == "session_shutdown" then
      line = line .. ":" .. tostring(event.reason)
    end
    record(line)
  end)
end
pi.register_command("seams-trace", { handler = function() return lines end })