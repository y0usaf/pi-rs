-- File-backed pi-working-indicator translation (dogfood extension).
-- Renders an animated HSL-gradient working spinner while the agent streams.
-- Public surface only: events (session_start/session_shutdown/before_agent_start/
--   model_select), ctx.ui.{setWorkingMessage,setWorkingIndicator,setStatus,theme.fg},
--   pi.getThinkingLevel(), pi.now_ms(), pi.set_timeout/clear_timeout.
-- Long-lived resource: a single startup-switch setTimeout is tracked and cleared
--   by clearWorkingIndicatorTimer(); nothing survives session_shutdown.
local pi = ...

local ANSI_FG_RESET = "\27[39m"
local ANSI_BASE_RGB = {
  { 0, 0, 0 }, { 128, 0, 0 }, { 0, 128, 0 }, { 128, 128, 0 },
  { 0, 0, 128 }, { 128, 0, 128 }, { 0, 128, 128 }, { 192, 192, 192 },
  { 128, 128, 128 }, { 255, 0, 0 }, { 0, 255, 0 }, { 255, 255, 0 },
  { 0, 0, 255 }, { 255, 0, 255 }, { 0, 255, 255 }, { 255, 255, 255 },
}
local ANSI_CUBE_VALUES = { 0, 95, 135, 175, 215, 255 }

local INDICATOR = {
  frameMs = 50,
  width = 15,
  maxBirthOffsetMs = 1000,
  runes = "0123456789abcdefABCDEF~!@#$£€%^&*()+=_",
  initialChar = ".",
  hiddenMessage = "\226\128\139", -- zero-width space
}

local STARTUP_FRAMES = math.ceil(INDICATOR.maxBirthOffsetMs / INDICATOR.frameMs) + 1
local STARTUP_SWITCH_MS = INDICATOR.maxBirthOffsetMs
local PRERENDERED_FRAMES = INDICATOR.width * 2
local GRADIENT_RAMP_WIDTH = INDICATOR.width * 3

local THINKING_COLOR = {
  off = "thinkingOff", minimal = "thinkingMinimal", low = "thinkingLow",
  medium = "thinkingMedium", high = "thinkingHigh", xhigh = "thinkingXhigh",
}

local workingIndicatorTimer
local workingIndicatorGeneration = 0

local function clamp(value, min, max)
  return math.max(min, math.min(max, value))
end

local function rgb(t) return { r = t[1], g = t[2], b = t[3] } end

local function current_thinking_level(ctx)
  if not (ctx.model and ctx.model.reasoning) then return nil end
  local level = pi.getThinkingLevel()
  if level and THINKING_COLOR[level] then return level end
  return nil
end

local function ansi256_to_rgb(index)
  if index < 16 then return rgb(ANSI_BASE_RGB[clamp(index, 0, 15) + 1]) end
  if index >= 232 then
    local gray = 8 + clamp(index - 232, 0, 23) * 10
    return { r = gray, g = gray, b = gray }
  end
  local offset = clamp(index - 16, 0, 215)
  return {
    r = ANSI_CUBE_VALUES[math.floor(offset / 36) + 1],
    g = ANSI_CUBE_VALUES[math.floor(offset / 6) % 6 + 1],
    b = ANSI_CUBE_VALUES[offset % 6 + 1],
  }
end

local function ansi_to_rgb(ansi)
  local r, g, b = ansi:match("\27%[38;2;(%d+);(%d+);(%d+)m")
  if r then
    return { r = tonumber(r), g = tonumber(g), b = tonumber(b) }
  end
  local color256 = ansi:match("\27%[38;5;(%d+)m")
  if color256 then return ansi256_to_rgb(tonumber(color256)) end
  return nil
end

local function rgb_to_hsl(t)
  local r, g, b = t.r / 255, t.g / 255, t.b / 255
  local max = math.max(r, g, b)
  local min = math.min(r, g, b)
  local l = (max + min) / 2
  if max == min then return { h = 0, s = 0, l = l } end
  local d = max - min
  local s = l > 0.5 and d / (2 - max - min) or d / (max + min)
  local h = 0
  if max == r then
    h = (g - b) / d + (g < b and 6 or 0)
  elseif max == g then
    h = (b - r) / d + 2
  else
    h = (r - g) / d + 4
  end
  return { h = h * 60, s = s, l = l }
end

local function hsl_to_rgb(t)
  local c = (1 - math.abs(2 * t.l - 1)) * t.s
  local hp = ((t.h % 360) + 360) % 360 / 60
  local x = c * (1 - math.abs((hp % 2) - 1))
  local m = t.l - c / 2
  local r, g, b = 0, 0, 0
  if hp < 1 then r, g, b = c, x, 0
  elseif hp < 2 then r, g, b = x, c, 0
  elseif hp < 3 then r, g, b = 0, c, x
  elseif hp < 4 then r, g, b = 0, x, c
  elseif hp < 5 then r, g, b = x, 0, c
  else r, g, b = c, 0, x end
  return { r = math.floor((r + m) * 255 + 0.5), g = math.floor((g + m) * 255 + 0.5), b = math.floor((b + m) * 255 + 0.5) }
end

local function gradient_ansi(start, finish, index, total)
  local t = (total <= 1) and 0 or math.min(1, index / (total - 1))
  local a = rgb_to_hsl(start)
  local b = rgb_to_hsl(finish)
  if a.s < 0.05 then a.h = b.h end
  if b.s < 0.05 then b.h = a.h end
  local hue_delta = ((((b.h - a.h) % 360) + 540) % 360) - 180
  local saturation = a.s + (b.s - a.s) * t
  local out = hsl_to_rgb({
    h = a.h + hue_delta * t,
    s = math.min(1, (t == 0 or t == 1) and saturation or math.max(0.4, saturation * 1.25)),
    l = a.l + (b.l - a.l) * t,
  })
  return string.format("\27[38;2;%d;%d;%dm", out.r, out.g, out.b)
end

local function clear_working_indicator_timer()
  workingIndicatorGeneration = workingIndicatorGeneration + 1
  if workingIndicatorTimer then
    pi.clear_timeout(workingIndicatorTimer)
    workingIndicatorTimer = nil
  end
end

-- theme:fg(key,"") yields the open ANSI code, i.e. getFgAnsi(key).
local function get_fg_ansi(theme, key)
  return theme:fg(key, "")
end

local function working_gradient(theme, thinking)
  local accentAnsi = get_fg_ansi(theme, "accent")
  local accentRgb = ansi_to_rgb(accentAnsi)
  local endAnsi = get_fg_ansi(theme, THINKING_COLOR[thinking or "high"])
  local endRgb = ansi_to_rgb(endAnsi)
  return { accentAnsi = accentAnsi, accentRgb = accentRgb, endRgb = endRgb }
end

local function working_gradient_ansi(gradient, index)
  if not gradient.accentRgb or not gradient.endRgb then return gradient.accentAnsi end
  local wrapped = ((index % GRADIENT_RAMP_WIDTH) + GRADIENT_RAMP_WIDTH) % GRADIENT_RAMP_WIDTH
  local segment = math.floor(wrapped / INDICATOR.width)
  local localIndex = wrapped % INDICATOR.width
  if segment == 1 then
    return gradient_ansi(gradient.endRgb, gradient.accentRgb, localIndex, INDICATOR.width)
  end
  return gradient_ansi(gradient.accentRgb, gradient.endRgb, localIndex, INDICATOR.width)
end

local function working_cell_colors(gradient, offset)
  local colors = {}
  for i = 0, INDICATOR.width - 1 do
    colors[i + 1] = working_gradient_ansi(gradient, i + offset)
  end
  return colors
end

local function render_working_frame(colors, chars)
  local cells = {}
  for i, color in ipairs(colors) do
    cells[i] = color .. (chars[i] or INDICATOR.initialChar)
  end
  return table.concat(cells) .. ANSI_FG_RESET
end

local function random_working_char()
  local index = math.random(#INDICATOR.runes)
  return INDICATOR.runes:sub(index, index)
end

local function random_working_chars()
  local chars = {}
  for _ = 1, INDICATOR.width do chars[#chars + 1] = random_working_char() end
  return chars
end

local function build_working_loop_frames(gradient, charFrames)
  local frames = {}
  for frame, chars in ipairs(charFrames) do
    frames[frame] = render_working_frame(working_cell_colors(gradient, frame - 1), chars)
  end
  return frames
end

local function build_working_startup_frames(gradient, loopCharFrames)
  local birthOffsets = {}
  for _ = 1, INDICATOR.width do birthOffsets[#birthOffsets + 1] = math.random() * INDICATOR.maxBirthOffsetMs end
  local frames = {}
  for frame = 0, STARTUP_FRAMES - 1 do
    local elapsedMs = frame * INDICATOR.frameMs
    local cyclingChars = loopCharFrames[(frame % #loopCharFrames) + 1] or {}
    local chars = {}
    for index = 1, INDICATOR.width do
      if elapsedMs < (birthOffsets[index] or 0) then
        chars[index] = INDICATOR.initialChar
      else
        chars[index] = cyclingChars[index] or INDICATOR.initialChar
      end
    end
    frames[frame + 1] = render_working_frame(working_cell_colors(gradient, frame), chars)
  end
  return frames
end

local function apply_working_indicator(ctx, startup)
  clear_working_indicator_timer()
  local gradient = working_gradient(ctx.ui.theme, current_thinking_level(ctx))
  local loopCharFrames = {}
  for _ = 1, PRERENDERED_FRAMES do loopCharFrames[#loopCharFrames + 1] = random_working_chars() end
  local loopFrames = build_working_loop_frames(gradient, loopCharFrames)
  ctx.ui.setWorkingMessage(INDICATOR.hiddenMessage)

  if startup then
    ctx.ui.setWorkingIndicator({ frames = build_working_startup_frames(gradient, loopCharFrames), intervalMs = INDICATOR.frameMs })
    local generation = workingIndicatorGeneration
    workingIndicatorTimer = pi.set_timeout(STARTUP_SWITCH_MS, function()
      if generation ~= workingIndicatorGeneration then return end
      workingIndicatorTimer = nil
      ctx.ui.setWorkingIndicator({ frames = loopFrames, intervalMs = INDICATOR.frameMs })
    end)
    return
  end

  ctx.ui.setWorkingIndicator({ frames = loopFrames, intervalMs = INDICATOR.frameMs })
end

pi.on("session_start", function(_event, ctx)
  apply_working_indicator(ctx, false)
end)

pi.on("session_shutdown", function(_event, ctx)
  clear_working_indicator_timer()
  ctx.ui.setWorkingIndicator()
  ctx.ui.setWorkingMessage()
end)

pi.on("before_agent_start", function(_event, ctx)
  apply_working_indicator(ctx, true)
end)

pi.on("model_select", function(_event, ctx)
  apply_working_indicator(ctx, false)
end)
