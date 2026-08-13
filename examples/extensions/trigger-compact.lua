-- Translation of Pi v0.79.0 examples/extensions/trigger-compact.ts.
-- Triggers compaction when context usage exceeds 100k tokens and adds a
-- /trigger-compact command.
local pi = ...

local COMPACT_THRESHOLD_TOKENS = 100000
local previous_tokens = nil

local function trigger_compaction(ctx, custom_instructions)
  if ctx.hasUI then
    ctx.ui.notify("Compaction started", "info")
  end
  ctx.compact({
    customInstructions = custom_instructions,
    onComplete = function()
      if ctx.hasUI then
        ctx.ui.notify("Compaction completed", "info")
      end
    end,
    onError = function(error)
      if ctx.hasUI then
        ctx.ui.notify("Compaction failed: " .. (error and error.message or tostring(error)), "error")
      end
    end,
  })
end

pi.on("turn_end", function(_event, ctx)
  local usage = ctx.getContextUsage()
  local current_tokens = usage and usage.tokens or nil
  if current_tokens == nil then
    return
  end

  local crossed_threshold = previous_tokens ~= nil and previous_tokens ~= 0 and previous_tokens <= COMPACT_THRESHOLD_TOKENS
  previous_tokens = current_tokens
  if not crossed_threshold or current_tokens <= COMPACT_THRESHOLD_TOKENS then
    return
  end
  trigger_compaction(ctx)
end)

pi.register_command("trigger-compact", {
  description = "Trigger compaction immediately",
  handler = function(args, ctx)
    local instructions = args:gsub("^%s+", ""):gsub("%s+$", "")
    if instructions == "" then instructions = nil end
    trigger_compaction(ctx, instructions)
  end,
})