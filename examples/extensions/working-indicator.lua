-- Translation of Pi v0.79.0 examples/extensions/working-indicator.ts.
-- Customizes the inline working indicator shown while streaming a response.
local pi = ...

local SPINNER_FRAMES = { "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏" }
local PASTEL_RAINBOW = {
  "\27[38;2;255;179;186m",
  "\27[38;2;255;223;186m",
  "\27[38;2;255;255;186m",
  "\27[38;2;186;255;201m",
  "\27[38;2;186;225;255m",
  "\27[38;2;218;186;255m",
}
local RESET_FG = "\27[39m"
local HIDDEN_INDICATOR = { frames = {} }

local function colorize(text, color)
  return color .. text .. RESET_FG
end

local function get_indicator(mode)
  if mode == "dot" then
    return { frames = { colorize("●", PASTEL_RAINBOW[1]) } }
  elseif mode == "none" then
    return HIDDEN_INDICATOR
  elseif mode == "pulse" then
    return {
      frames = {
        colorize("·", PASTEL_RAINBOW[1]),
        colorize("•", PASTEL_RAINBOW[3]),
        colorize("●", PASTEL_RAINBOW[5]),
        colorize("•", PASTEL_RAINBOW[6]),
      },
      intervalMs = 120,
    }
  elseif mode == "spinner" then
    local frames = {}
    for index, frame in ipairs(SPINNER_FRAMES) do
      frames[#frames + 1] = colorize(frame, PASTEL_RAINBOW[(index - 1) % #PASTEL_RAINBOW + 1])
    end
    return { frames = frames, intervalMs = 80 }
  end
  -- "default"
  return nil
end

local function describe_mode(mode)
  if mode == "dot" then return "static dot"
  elseif mode == "none" then return "hidden"
  elseif mode == "pulse" then return "custom pulse"
  elseif mode == "spinner" then return "custom spinner"
  else return "pi default spinner" end
end

local mode = "spinner"

local function apply_indicator(ctx)
  ctx.ui.setWorkingIndicator(get_indicator(mode))
  ctx.ui.setStatus("working-indicator", ctx.ui.theme:fg("dim", "Indicator: " .. describe_mode(mode)))
end

pi.on("session_start", function(_event, ctx)
  apply_indicator(ctx)
end)

pi.register_command("working-indicator", {
  description = "Set the streaming working indicator: dot, pulse, none, spinner, or reset.",
  handler = function(args, ctx)
    local next_mode = args:gsub("^%s+", ""):gsub("%s+$", ""):lower()
    if next_mode == "" then
      ctx.ui.notify("Working indicator: " .. describe_mode(mode), "info")
      return
    end

    if next_mode ~= "dot" and next_mode ~= "none" and next_mode ~= "pulse"
      and next_mode ~= "spinner" and next_mode ~= "reset" then
      ctx.ui.notify("Usage: /working-indicator [dot|pulse|none|spinner|reset]", "error")
      return
    end

    mode = next_mode == "reset" and "default" or next_mode
    apply_indicator(ctx)
    ctx.ui.notify("Working indicator set to: " .. describe_mode(mode), "info")
  end,
})