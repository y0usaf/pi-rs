-- Translation of Pi's permission-gate.ts. In non-interactive modes dangerous
-- bash calls are blocked; interactive confirmation joins the extension UI rung.
local pi = ...

local function dangerous(command)
  local lower = string.lower(command)
  return lower:match("%f[%w]rm%s+%-r[f]?%f[%W]") ~= nil
    or lower:match("%f[%w]rm%s+%-%-recursive%f[%W]") ~= nil
    or lower:match("%f[%w]sudo%f[%W]") ~= nil
    or lower:match("%f[%w]chmod%f[%W].*777") ~= nil
    or lower:match("%f[%w]chown%f[%W].*777") ~= nil
end

pi.on("tool_call", function(event, ctx)
  if event.toolName ~= "bash" or not dangerous(event.input.command or "") then return nil end
  if not ctx.hasUI then
    return { block = true, reason = "Dangerous command blocked (no UI for confirmation)" }
  end
  -- ctx.ui.select is completed in PLAN 9.5. Keeping the no-UI gate executable
  -- now proves blocking-hook composition without inventing a privileged prompt.
  if not ctx.ui then return { block = true, reason = "Blocked by user" } end
  local choice = ctx.ui.select("⚠️ Dangerous command:\n\n  " .. event.input.command .. "\n\nAllow?", { "Yes", "No" })
  if choice ~= "Yes" then return { block = true, reason = "Blocked by user" } end
  return nil
end)
