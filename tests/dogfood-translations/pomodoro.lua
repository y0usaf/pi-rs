-- pi-rs translation of the pinned dogfood package `pi-pomodoro` (v1.0.0,
-- `pi-flake` extensions/pi-pomodoro, src/index.ts). Reproduces the non-blocking
-- synced pomodoro timer through the public `pi.*` surface only: fs settings
-- via the extensionSettings seam, atomic state writes, a per-dispatch
-- `set_interval` tick, a `watch_file`-style sync-file watcher, and queued UI
-- (setStatus/notify). No privileged escape hatch and no JS runtime.
local pi = ...

local KEY = "pi-pomodoro"
local VERSION = 1
local TICK_MS = 1000
local ICON = "🍅"
local BAR_LEN = 10
local FILLED = "▰"
local EMPTY = "▱"

local settings = {}
local state = {}
local timer = nil
local watcher = nil
local ctx_ref = nil
local last_phase = ""

local function is_record(v) return type(v) == "table" and v[1] == nil end
local function now() return pi.monotonic_ms() end
local function now_epoch() return pi.now_ms() end

local function agent_dir()
  local env = pi.env.PI_CODING_AGENT_DIR
  if env and env ~= "" then return env end
  return (pi.env.HOME or "") .. "/.pi/agent"
end

local function defaults()
  return {
    workMinutes = 25,
    breakMinutes = 5,
    longBreakMinutes = 15,
    longBreakEvery = 4,
    syncFile = pi.path.join(pi.env.XDG_RUNTIME_DIR or (pi.env.HOME or "."), "pi-pomodoro-user.json"),
    notifyTransitions = true,
  }
end

local function idle()
  local time = now_epoch()
  return { version = VERSION, running = false, paused = false, phase = "work",
    cycle = 0, startedAt = time, endsAt = time, updatedAt = time }
end

local function read_json(path)
  local ok_exists, exists = pcall(pi.fs.exists, path)
  if not ok_exists or not exists then return nil end
  local ok_read, content = pcall(pi.fs.read_file, path)
  if not ok_read then return nil end
  local ok_json, parsed = pcall(pi.json.decode, content)
  if not ok_json or not is_record(parsed) then return nil end
  return parsed
end

local function minutes(value, fallback, integer)
  local n = type(value) == "number" and value > 0 and value or fallback
  if integer then n = math.max(1, math.floor(n)) end
  return n
end

local function extension_settings(path)
  local raw = read_json(path)
  if not raw then return nil end
  local extension = raw.extensionSettings
  if not is_record(extension) then return nil end
  local ours = extension[KEY]
  return is_record(ours) and ours or nil
end

local function apply_settings(raw, base)
  if not raw then return base end
  local out = {}
  for k, v in pairs(base) do out[k] = v end
  out.workMinutes = minutes(raw.workMinutes, base.workMinutes)
  out.breakMinutes = minutes(raw.breakMinutes, base.breakMinutes)
  out.longBreakMinutes = minutes(raw.longBreakMinutes, base.longBreakMinutes)
  out.longBreakEvery = minutes(raw.longBreakEvery, base.longBreakEvery, true)
  out.syncFile = type(raw.syncFile) == "string" and raw.syncFile:match("^%s*(.-)%s*$") ~= "" and raw.syncFile:gsub("^%s+", ""):gsub("%s+$", "") or base.syncFile
  out.notifyTransitions = type(raw.notifyTransitions) == "boolean" and raw.notifyTransitions or base.notifyTransitions
  return out
end

local function load_settings(cwd)
  local current = defaults()
  current = apply_settings(extension_settings(pi.path.join(agent_dir(), "settings.json")), current)
  current = apply_settings(extension_settings(pi.path.join(cwd, ".pi", "settings.json")), current)
  return current
end

local function parse_state(raw)
  if not is_record(raw) or type(raw.running) ~= "boolean" or type(raw.paused) ~= "boolean" then return nil end
  if raw.phase ~= "work" and raw.phase ~= "break" then return nil end
  local function finite(v, fallback) return type(v) == "number" and v or fallback end
  return {
    version = VERSION, running = raw.running, paused = raw.paused, phase = raw.phase,
    cycle = math.max(0, math.floor(finite(raw.cycle, 0))),
    startedAt = finite(raw.startedAt, now_epoch()),
    endsAt = finite(raw.endsAt, now_epoch()),
    updatedAt = finite(raw.updatedAt, now_epoch()),
    remainingMs = raw.remainingMs == nil and nil or math.max(0, finite(raw.remainingMs, 0)),
    totalMs = raw.totalMs == nil and nil or math.max(1, finite(raw.totalMs, 1)),
    source = type(raw.source) == "string" and raw.source or nil,
  }
end

local function read_state()
  return parse_state(read_json(settings.syncFile)) or idle()
end

local function write_state(next_state)
  -- mkdirSync(dirname, recursive)
  local dir = settings.syncFile:match("^(.*)[/\\][^/\\]*$") or "."
  pcall(pi.fs.mkdir, pi.path.join(pi.cwd(), dir))
  state = next_state
  state.version = VERSION
  state.updatedAt = now_epoch()
  -- Node's writeState writes `data.tmp` then renameSync(tmp, syncFile):
  -- atomic write via the public pi.fs.write_file_atomic mechanism.
  pcall(pi.fs.write_file_atomic, settings.syncFile, pi.json.encode(state, false) .. "\n")
end

local function duration(phase, cycle)
  cycle = cycle or state.cycle
  if phase == "work" then return settings.workMinutes * 60000 end
  if cycle > 0 and cycle % settings.longBreakEvery == 0 then
    return settings.longBreakMinutes * 60000
  end
  return settings.breakMinutes * 60000
end

local function start(phase, cycle, source, custom_minutes)
  local time = now_epoch()
  local ms = (custom_minutes or (duration(phase, cycle) / 60000)) * 60000
  return { version = VERSION, running = true, paused = false, phase = phase, cycle = cycle,
    startedAt = time, endsAt = time + ms, updatedAt = time, totalMs = ms, source = source }
end

local function remaining()
  if not state.running then return 0 end
  if state.paused then return state.remainingMs or math.max(0, state.endsAt - now_epoch()) end
  return math.max(0, state.endsAt - now_epoch())
end

local function advance()
  if not state.running or state.paused or now_epoch() < state.endsAt then return end
  local next_phase = state.phase == "work" and "break" or "work"
  local cycle = next_phase == "break" and state.cycle + 1 or state.cycle
  write_state(start(next_phase, cycle, "auto:pi"))
end

local function format(ms)
  local seconds = math.ceil(ms / 1000)
  return string.format("%d:%02d", math.floor(seconds / 60), seconds % 60)
end

local function progress(width)
  if not state.running then return 0 end
  local total = math.max(1, state.totalMs or (state.endsAt - state.startedAt))
  local left = remaining()
  local elapsed = math.max(0, total - left)
  return math.max(0, math.min(width, math.floor((elapsed / total) * width + 0.5)))
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
  local phase_key = (state.running and not state.paused) and (state.phase .. ":" .. state.cycle) or "idle"
  if settings.notifyTransitions and last_phase ~= "" and phase_key ~= last_phase and phase_key ~= "idle" then
    ctx.ui.notify(state.phase == "break" and (ICON .. " break time") or (ICON .. " work time"), "info")
  end
  last_phase = phase_key
end

local function refresh()
  local next_state = read_state()
  if next_state.updatedAt >= state.updatedAt or next_state.source == state.source then
    state = next_state
  end
  if ctx_ref then render(ctx_ref) end
end

local function watch_sync_file()
  if watcher then
    local old = watcher
    watcher = nil
    pcall(function() old:close() end)
  end
  watcher = pi.fs.watch_file(settings.syncFile, function() refresh() end)
end

local function ensure_timer()
  if timer then return end
  timer = pi.set_interval(TICK_MS, function()
    if ctx_ref then render(ctx_ref) end
  end)
end

local function source() return "command:pi" end
local function parse_minutes(arg)
  local n = tonumber((arg or ""):gsub("^%s+", ""):gsub("%s+$", ""))
  return (n and n > 0) and n or nil
end
local function status()
  if state.running then
    return (ICON .. "  " .. state.phase .. (state.paused and " paused" or "") .. " "
      .. format(remaining()) .. " · cycle " .. state.cycle)
  end
  return ICON .. "  pomodoro idle"
end

pi.on("session_start", function(_event, ctx)
  ctx_ref = ctx
  settings = load_settings(ctx.cwd)
  state = read_state()
  watch_sync_file()
  ensure_timer()
  render(ctx)
end)

pi.on("session_shutdown", function(_event, ctx)
  if watcher then pcall(function() watcher:close() end); watcher = nil end
  if timer then pi.clear_interval(timer); timer = nil end
  ctx.ui.setStatus(KEY, nil)
  ctx_ref = nil
end)

pi.register_command("pomodoro", {
  description = "Synced non-blocking pomodoro timer: start|stop|pause|resume|status|work|break [minutes]",
  handler = function(args, ctx)
    ctx_ref = ctx
    settings = load_settings(ctx.cwd)
    watch_sync_file()
    refresh()

    local raw_action, raw_minutes = (args or ""):match("^%s*(%S+)%s*(%S*)%s*$")
    if not raw_action then raw_action = "" end
    local action = raw_action:lower()
    if action == "" then action = "start" end
    local custom = parse_minutes(raw_minutes or "")

    if action == "start" or action == "work" then
      write_state(start("work", state.cycle, source(), custom))
    elseif action == "break" then
      local cycle = state.phase == "work" and state.cycle + 1 or state.cycle
      write_state(start("break", cycle, source(), custom))
    elseif action == "pause" and state.running and not state.paused then
      local next_state = {}
      for k, v in pairs(state) do next_state[k] = v end
      next_state.paused = true
      next_state.remainingMs = remaining()
      next_state.totalMs = state.totalMs or math.max(1, state.endsAt - state.startedAt)
      next_state.source = source()
      write_state(next_state)
    elseif action == "resume" and state.running and state.paused then
      local time = now_epoch()
      local left = remaining()
      local total = math.max(1, state.totalMs or (state.endsAt - state.startedAt))
      local next_state = {}
      for k, v in pairs(state) do next_state[k] = v end
      next_state.paused = false
      next_state.startedAt = time - (total - left)
      next_state.endsAt = time + left
      next_state.remainingMs = nil
      next_state.totalMs = total
      next_state.source = source()
      write_state(next_state)
    elseif action == "stop" or action == "reset" then
      write_state(idle_with_source())
    elseif action ~= "status" then
      ctx.ui.notify("Usage: /pomodoro [start|stop|pause|resume|status|work|break|reset] [minutes]", "warning")
      return
    end

    render(ctx)
    ctx.ui.notify(status(), "info")
  end,
})

function idle_with_source()
  local st = idle()
  st.source = source()
  return st
end