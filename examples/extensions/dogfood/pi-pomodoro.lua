-- File-backed pi-pomodoro translation (dogfood package).
-- Public surface only: events, register_command, ctx.ui.{setStatus,notify,theme.fg},
-- pi.fs (watch_file/read_file/write_file_atomic/exists/mkdir/rename/mkdtemp/tmpdir),
-- pi.path.join/dirname, pi.env, pi.set_interval/clear_interval, pi.json.
-- Long-lived resources: the file watcher and the 1s interval timer are both
-- disposed on session_shutdown; the watcher's background thread also stops on
-- handle:close() and on Drop. A stale ctxRef is never written after shutdown.
local pi = ...

local KEY = "pi-pomodoro"
local VERSION = 1
local TICK_MS = 1000
local ICON = "🍅"
local BAR_LEN = 10
local FILLED = "▰"
local EMPTY = "▱"

local now = function() return pi.now_ms() end

local settings = {}
local state = {}
local ctxRef
local timer
local watchedFile
local lastPhase = ""

local function agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE or "."
  return pi.path.join(home, ".pi", "agent")
end

local function defaults()
  local uid = pi.env.USER or "user"
  local syncDir = pi.env.XDG_RUNTIME_DIR or pi.fs.tmpdir()
  return {
    workMinutes = 25,
    breakMinutes = 5,
    longBreakMinutes = 15,
    longBreakEvery = 4,
    syncFile = pi.path.join(syncDir, "pi-pomodoro-" .. uid .. ".json"),
    notifyTransitions = true,
  }
end

local function idle()
  local time = now()
  return { version = VERSION, running = false, paused = false, phase = "work", cycle = 0,
    startedAt = time, endsAt = time, updatedAt = time }
end

local function read_json(path)
  if not pi.fs.exists(path) then return nil end
  local ok, value = pcall(pi.json.decode, pi.fs.read_file(path))
  if not ok then return nil end
  if type(value) == "table" then return value end
  return nil
end

local function minutes(value, fallback, integer)
  local n = (type(value) == "number" and value > 0) and value or fallback
  if integer then return math.max(1, math.floor(n)) end
  return n
end

local function extension_settings(path)
  local raw = read_json(path)
  local ext = raw and raw.extensionSettings
  if type(ext) ~= "table" then return nil end
  local ours = ext[KEY]
  if type(ours) == "table" then return ours end
  return nil
end

local function apply_settings(raw, base)
  if not raw then return base end
  local syncFile = base.syncFile
  if type(raw.syncFile) == "string" and raw.syncFile:gsub("%s", "") ~= "" then
    syncFile = raw.syncFile
  end
  return {
    workMinutes = minutes(raw.workMinutes, base.workMinutes),
    breakMinutes = minutes(raw.breakMinutes, base.breakMinutes),
    longBreakMinutes = minutes(raw.longBreakMinutes, base.longBreakMinutes),
    longBreakEvery = minutes(raw.longBreakEvery, base.longBreakEvery, true),
    syncFile = syncFile,
    notifyTransitions = type(raw.notifyTransitions) == "boolean" and raw.notifyTransitions or base.notifyTransitions,
  }
end

local function load_settings(cwd)
  local result = defaults()
  result = apply_settings(extension_settings(pi.path.join(agent_dir(), "settings.json")), result)
  result = apply_settings(extension_settings(pi.path.join(cwd, ".pi", "settings.json")), result)
  return result
end

local function parse_state(raw)
  if type(raw) ~= "table" or type(raw.running) ~= "boolean" or type(raw.paused) ~= "boolean" then return nil end
  if raw.phase ~= "work" and raw.phase ~= "break" then return nil end
  local function f(value, fallback)
    return (type(value) == "number") and value or fallback
  end
  return {
    version = VERSION,
    running = raw.running,
    paused = raw.paused,
    phase = raw.phase,
    cycle = math.max(0, math.floor(f(raw.cycle, 0))),
    startedAt = f(raw.startedAt, now()),
    endsAt = f(raw.endsAt, f(raw.startedAt, now())),
    updatedAt = f(raw.updatedAt, now()),
    remainingMs = raw.remainingMs == nil and nil or math.max(0, f(raw.remainingMs, 0)),
    totalMs = raw.totalMs == nil and nil or math.max(1, f(raw.totalMs, 1)),
    source = type(raw.source) == "string" and raw.source or nil,
  }
end

local function read_state()
  return parse_state(read_json(settings.syncFile)) or idle()
end

local function write_state(next_state)
  pi.fs.mkdir(pi.path.dirname(settings.syncFile), nil, true)
  local merged = {}
  for key, value in pairs(next_state) do merged[key] = value end
  merged.version = VERSION
  merged.updatedAt = now()
  state = merged
  pi.fs.write_file_atomic(settings.syncFile, pi.json.encode(state, true) .. "\n")
end

local function duration(phase, cycle)
  cycle = cycle or state.cycle
  if phase == "work" then return settings.workMinutes * 60000 end
  local breadValue = (cycle > 0 and cycle % settings.longBreakEvery == 0)
    and settings.longBreakMinutes or settings.breakMinutes
  return breadValue * 60000
end

local function start(phase, cycle, src, customMinutes)
  local time = now()
  local ms = (customMinutes or duration(phase, cycle) / 60000) * 60000
  return { version = VERSION, running = true, paused = false, phase = phase, cycle = cycle,
    startedAt = time, endsAt = time + ms, updatedAt = time, totalMs = ms, source = src }
end

local function remaining()
  if not state.running then return 0 end
  if state.paused then
    return state.remainingMs or math.max(0, state.endsAt - now())
  end
  return math.max(0, state.endsAt - now())
end

local function advance()
  if not state.running or state.paused or now() < state.endsAt then return end
  local nextPhase = state.phase == "work" and "break" or "work"
  local src = "auto:" .. tostring(pi.process and pi.process.pid or "0")
  write_state(start(nextPhase, nextPhase == "break" and state.cycle + 1 or state.cycle, src))
end

local function format(ms)
  local seconds = math.ceil(ms / 1000)
  return math.floor(seconds / 60) .. ":" .. string.format("%02d", seconds % 60)
end

local function progress(width)
  if not state.running then return 0 end
  local total = math.max(1, state.totalMs or state.endsAt - state.startedAt)
  local left = remaining()
  local elapsed = math.max(0, total - left)
  return math.max(0, math.min(width, math.floor((elapsed / total) * width)))
end

local function bar(label, phase, theme, paused)
  local filled = progress(BAR_LEN)
  local tint = (not paused and phase == "break") and "warning" or "dim"
  return theme:fg(tint, label) .. " " .. theme:fg(tint, string.rep(FILLED, filled))
    .. theme:fg("dim", string.rep(EMPTY, BAR_LEN - filled))
end

local function render(ctx)
  advance()
  local theme = ctx.ui.theme

  if not state.running then
    ctx.ui.setStatus(KEY, theme:fg("dim", "idle"))
  else
    local phase = state.phase
    local time = format(remaining())
    local label = phase .. (state.paused and " ⏸" or "") .. " " .. time
    ctx.ui.setStatus(KEY, bar(label, phase, theme, state.paused))
  end

  local phaseKey = state.running and not state.paused and (state.phase .. ":" .. state.cycle) or "idle"
  if settings.notifyTransitions and lastPhase ~= "" and phaseKey ~= lastPhase and phaseKey ~= "idle" then
    ctx.ui.notify(state.phase == "break" and (ICON .. " break time") or (ICON .. " work time"), "info")
  end
  lastPhase = phaseKey
end

local function refresh()
  local next_state = read_state()
  if next_state.updatedAt >= state.updatedAt or next_state.source == state.source then state = next_state end
  if ctxRef then render(ctxRef) end
end

local function watch_sync_file()
  if watchedFile then
    watchedFile:close()
    watchedFile = nil
  end
  watchedFile = pi.fs.watch_file(settings.syncFile, function() refresh() end)
end

local function ensure_timer()
  if timer then return end
  timer = pi.set_interval(TICK_MS, function()
    if ctxRef then render(ctxRef) end
  end)
end

local source = function()
  return "command:" .. tostring(pi.process and pi.process.pid or "0")
end

local function parse_minutes(arg)
  local n = tonumber((arg or ""):gsub("%s", ""))
  return (n and n > 0) and n or nil
end

local function status()
  if state.running then
    return string.format("%s %s%s %s · cycle %d", ICON, state.phase,
      state.paused and " paused" or "", format(remaining()), state.cycle)
  end
  return ICON .. "  pomodoro idle"
end

pi.on("session_start", function(_event, ctx)
  ctxRef = ctx
  settings = load_settings(ctx.cwd)
  state = read_state()
  watch_sync_file()
  ensure_timer()
  render(ctx)
end)

pi.on("session_shutdown", function(_event, ctx)
  if watchedFile then watchedFile:close() end
  if timer then pi.clear_interval(timer) end
  watchedFile = nil
  timer = nil
  ctx.ui.setStatus(KEY, nil)
  ctxRef = nil
end)

pi.register_command("pomodoro", {
  description = "Synced non-blocking pomodoro timer: start|stop|pause|resume|status|work|break [minutes]",
  handler = function(args, ctx)
    ctxRef = ctx
    settings = load_settings(ctx.cwd)
    watch_sync_file()
    refresh()

    local trimmed = args:gsub("%s", "")
    local rawAction, rawMinutes = trimmed:match("^(%S*)%s*(%S*)")
    if rawAction == nil then rawAction = trimmed end
    local action = (rawAction == "" and "start" or rawAction):lower()
    local customMinutes = parse_minutes(rawMinutes)

    if action == "start" or action == "work" then
      write_state(start("work", state.cycle, source(), customMinutes))
    elseif action == "break" then
      write_state(start("break", state.phase == "work" and state.cycle + 1 or state.cycle, source(), customMinutes))
    elseif action == "pause" and state.running and not state.paused then
      local total = state.totalMs or math.max(1, state.endsAt - state.startedAt)
      local s = { version = VERSION, running = true, paused = true, phase = state.phase,
        cycle = state.cycle, startedAt = state.startedAt, endsAt = state.endsAt,
        remainingMs = remaining(), totalMs = total, source = source() }
      write_state(s)
    elseif action == "resume" and state.running and state.paused then
      local time = now()
      local left = remaining()
      local total = math.max(1, state.totalMs or state.endsAt - state.startedAt)
      local s = { version = VERSION, running = true, paused = false, phase = state.phase,
        cycle = state.cycle, startedAt = time - (total - left), endsAt = time + left,
        remainingMs = nil, totalMs = total, source = source() }
      write_state(s)
    elseif action == "stop" or action == "reset" then
      local s = idle()
      s.source = source()
      write_state(s)
    elseif action ~= "status" then
      ctx.ui.notify("Usage: /pomodoro [start|stop|pause|resume|status|work|break|reset] [minutes]", "warning")
      return
    end

    render(ctx)
    ctx.ui.notify(status(), "info")
  end,
})
