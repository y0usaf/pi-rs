-- Translation of Pi v0.79.0 examples/extensions/input-transform-streaming.ts.
-- Streaming-aware input gate: skips expensive pre-processing during
-- mid-stream steering.
local pi = ...

local function has_trigger(text)
  return text:find("[Cc]hanges?") ~= nil or text:find("[Dd]iff") ~= nil or text:find("[Mm]odified") ~= nil
end

pi.on("input", function(event)
  -- During steering, skip the exec call — corrections should be fast
  if event.streamingBehavior == "steer" then
    return { action = "continue" }
  end

  if not has_trigger(event.text) then
    return { action = "continue" }
  end

  local diff = pi.exec("git", { "diff", "--stat" })
  if diff.code ~= 0 or #(diff.stdout:gsub("%s+", "")) == 0 then
    return { action = "continue" }
  end

  return {
    action = "transform",
    text = event.text .. "\n\nCurrent uncommitted changes:\n```\n" .. diff.stdout:gsub("%s+$", "") .. "\n```",
  }
end)