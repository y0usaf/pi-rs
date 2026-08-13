-- Translation of Pi v0.79.0 examples/extensions/titlebar-spinner.ts.
-- Shows a braille spinner animation in the terminal title while the agent
-- is working, via ctx.ui.setTitle().
local pi = ...

local BRAILLE_FRAMES = { "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏" }

local function get_base_title()
  local cwd = pi.path.basename(pi.cwd())
  local session = pi.getSessionName()
  return session and ("π - " .. session .. " - " .. cwd) or ("π - " .. cwd)
end

local timer = nil
local frame_index = 0

local function stop_animation(ctx)
  if timer then
    pi.clear_interval(timer)
    timer = nil
  end
  frame_index = 0
  ctx.ui.setTitle(get_base_title())
end

local function start_animation(ctx)
  stop_animation(ctx)
  timer = pi.set_interval(80, function()
    local frame = BRAILLE_FRAMES[(frame_index % #BRAILLE_FRAMES) + 1]
    local cwd = pi.path.basename(pi.cwd())
    local session = pi.getSessionName()
    local title = session and (frame .. " π - " .. session .. " - " .. cwd) or (frame .. " π - " .. cwd)
    ctx.ui.setTitle(title)
    frame_index = frame_index + 1
  end)
end

pi.on("agent_start", function(_event, ctx)
  start_animation(ctx)
end)

pi.on("agent_end", function(_event, ctx)
  stop_animation(ctx)
end)

pi.on("session_shutdown", function(_event, ctx)
  stop_animation(ctx)
end)