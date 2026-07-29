-- Shipped frontend root: the only owner of the retained display and the
-- bounded terminal input buffer.
--
-- The frontend answers events with two kinds of output: `ansi` display
-- actions, which Rust presents, and named intents (`frontend_submit`,
-- `frontend_interrupt`, `frontend_exit`), which are ordinary data for the
-- application root. It never talks to the agent and holds no host state, so
-- registering a higher-priority frontend root replaces the whole presentation
-- without touching application or agent policy.

local pi = ...
local roots = pi.roots.v1
local module = pi.kernel.v1.module
local terminal = pi.terminal.v1

local keys = module.require("pi.frontend.keys", "1")
local editor_module = module.require("pi.frontend.editor", "1")
local transcript_module = module.require("pi.frontend.transcript", "1")
local chrome_module = module.require("pi.frontend.chrome", "1")
local view = module.require("pi.frontend.view", "1")

local DEFAULT_COLUMNS = 80
local DEFAULT_ROWS = 24

local state = nil

local function ensure()
  if state == nil then
    state = {
      display = terminal.display(),
      input = terminal.input_buffer(),
      editor = editor_module.new(),
      transcript = transcript_module.new(),
      chrome = chrome_module.new(),
      columns = DEFAULT_COLUMNS,
      rows = DEFAULT_ROWS,
      focus = "editor",
      dirty = true,
    }
  end
  return state
end

local function invalidate()
  ensure().dirty = true
end

-- One frame per invalidation: an unchanged frontend submits nothing, and the
-- retained display suppresses an empty diff even when it does.
local function render(force)
  local current = ensure()
  if not force and not current.dirty then
    return false
  end
  local frame = current.display:submit(view.build({
    columns = current.columns,
    rows = current.rows,
    header = current.chrome:header(),
    footer = current.chrome:footer(),
    guidance = current.chrome:guidance_row(),
    -- Presentation builds at most one viewport of lines, so a long
    -- conversation costs the same per frame as a short one.
    transcript = current.transcript:lines(current.columns, current.rows),
    editor_lines = current.editor:lines(),
    cursor = current.editor:cursor(),
  }))
  current.dirty = false
  if frame.ansi and #frame.ansi > 0 then
    roots.action("ansi", { data = frame.ansi })
    return true
  end
  return false
end

-- Input routing: keys go to the focused component. Only the editor takes
-- focus today, but routing is a lookup rather than a branch so a later
-- component can join without reshaping the root.
local EDITOR_KEYS = {
  left = "left",
  right = "right",
  up = "up",
  down = "down",
  home = "home",
  ["end"] = "end",
}

local function route_editor_key(current, key)
  local kind = key.kind
  if kind == "text" then
    current.editor:insert(key.text)
    return true
  end
  if kind == "newline" then
    current.editor:newline()
    return true
  end
  if kind == "backspace" then
    current.editor:backspace()
    return true
  end
  if kind == "clear_line" then
    current.editor:clear_line()
    return true
  end
  local direction = EDITOR_KEYS[kind]
  if direction then
    current.editor:move(direction)
    return true
  end
  return false
end

local function handle_key(current, key)
  local kind = key.kind

  if kind == "submit" then
    if current.editor:is_empty() then
      current.editor:clear()
      return true
    end
    local text = current.editor:text()
    current.editor:clear()
    current.transcript:user(text)
    current.chrome:set_status("streaming")
    current.chrome:clear_guidance()
    roots.action("frontend_submit", { text = text })
    return true
  end

  if kind == "interrupt" then
    current.transcript:notice("info", "interrupt requested")
    roots.action("frontend_interrupt", {})
    return true
  end

  if kind == "eof" then
    if current.editor:is_empty() then
      current.chrome:set_status("exiting")
      current.transcript:notice("info", "exiting")
      roots.action("frontend_exit", { reason = "eof" })
      return true
    end
    current.editor:clear()
    return true
  end

  if kind == "toggle_thinking" then
    -- Thinking visibility is presentation policy, so it is one transcript
    -- option read by the block renderer plus one keyed status row. Nothing
    -- about it reaches the agent or the host.
    local visible = current.transcript:option("thinking_visible") ~= false
    current.transcript:set_option("thinking_visible", not visible)
    current.transcript:status(
      "thinking_visibility",
      "info",
      "Thinking blocks: " .. (visible and "hidden" or "visible")
    )
    return true
  end

  if current.focus == "editor" then
    return route_editor_key(current, key)
  end
  return false
end

-- Agent actions are data. Mapping them to rows, status, and guidance is
-- presentation policy; each transcript change renders immediately so
-- streaming output appears incrementally rather than at turn end.
local function apply_agent_action(current, action)
  local kind = action.kind
  local payload = action.payload or {}

  if kind == "agent_text_delta" then
    current.transcript:assistant_delta(payload.text)
    invalidate()
    render(false)
    return
  end
  if kind == "agent_message" then
    current.transcript:assistant_done(payload.text)
    invalidate()
    render(false)
    return
  end
  if kind == "agent_thinking_delta" then
    current.transcript:thinking_delta(payload.text)
    invalidate()
    render(false)
    return
  end
  if kind == "agent_thinking" then
    current.transcript:thinking(payload.text)
    invalidate()
    render(false)
    return
  end
  if kind == "agent_tool_start" then
    current.transcript:tool_start(payload.id, payload.name, payload.arguments)
    invalidate()
    render(false)
    return
  end
  if kind == "agent_tool_result" then
    current.transcript:tool_result(payload.id, payload.name, payload.ok == true, payload.output)
    invalidate()
    render(false)
    return
  end
  if kind == "agent_retry" then
    current.transcript:notice("warn", "retrying: " .. tostring(payload.reason))
    invalidate()
    render(false)
    return
  end
  if kind == "agent_error" then
    current.chrome:set_status("error")
    current.chrome:set_guidance(current.chrome:guidance_for(payload.reason))
    current.transcript:notice("error", tostring(payload.reason))
    invalidate()
    render(false)
    return
  end
  if kind == "agent_cancelled" then
    current.chrome:set_status("cancelled")
    -- The canonical set records this state as one failure-coloured row, so
    -- the wording and the level are the reviewed ones.
    current.transcript:notice("error", "Operation aborted")
    invalidate()
    render(false)
    return
  end
  if kind == "agent_queued" then
    if payload.queue == "interrupt" then
      current.transcript:notice("info", "interrupt queued")
      invalidate()
      render(false)
    end
    return
  end
  if kind == "agent_status" then
    if payload.state == "idle" then
      current.chrome:set_status("idle")
    elseif payload.state == "streaming" then
      current.chrome:set_status("streaming")
    end
    invalidate()
    return
  end
  if kind == "agent_steered" or kind == "agent_follow_up" then
    current.transcript:user(tostring(payload.text or ""))
    invalidate()
    render(false)
    return
  end
  if kind == "agent_reset" then
    current.transcript:clear()
    current.chrome:set_status("idle")
    current.chrome:clear_guidance()
    invalidate()
  end
end

roots.register({
  kind = "frontend",
  id = "pi.builtins.frontend",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    local current = ensure()
    local event = snapshot.event
    local kind = type(event) == "table" and event.kind or nil

    if kind == "startup" then
      current.chrome:set_status("idle")
      -- Startup owns the whole screen: drop any retained presentation so the
      -- first frame is a complete input-ready paint.
      current.display:reset_presentation()
      render(true)
      roots.action("frontend_ready", { columns = current.columns, rows = current.rows })
      return
    end

    if kind == "configure" then
      current.chrome:set_model(event.model)
      if event.model ~= nil then
        current.chrome:clear_guidance()
      end
      invalidate()
      render(false)
      return
    end

    if kind == "input" then
      local decoded = keys.decode(current.input:feed(tostring(event.data or "")))
      local changed = false
      for _, key in ipairs(decoded) do
        if handle_key(current, key) then
          changed = true
        end
      end
      if changed then
        invalidate()
      end
      render(false)
      return
    end

    if kind == "agent" then
      local actions = type(event.actions) == "table" and event.actions or {}
      for _, action in ipairs(actions) do
        apply_agent_action(current, {
          kind = action.kind,
          payload = action.payload,
        })
      end
      render(false)
      return
    end

    if kind == "resize" then
      current.columns = tonumber(event.columns) or current.columns
      current.rows = tonumber(event.rows) or current.rows
      -- A resize invalidates every retained cell, so the next frame is a
      -- full repaint by construction.
      current.display:reset_presentation()
      render(true)
      return
    end

    if kind == "notice" then
      current.transcript:notice(event.level, event.text)
      invalidate()
      render(false)
      return
    end

    if kind == "shutdown" then
      current.chrome:set_status("exiting")
      current.transcript:notice("info", "session closed")
      invalidate()
      render(true)
      -- Leave the terminal on its own line for the shell prompt.
      roots.action("ansi", { data = "\r\n" })
      roots.action("frontend_closed", { reason = tostring(event.reason or "shutdown") })
      return
    end

    if kind == "status" then
      roots.action("frontend_status", {
        columns = current.columns,
        rows = current.rows,
        rows_used = current.transcript:len(),
        status = current.chrome.status,
        guidance = current.chrome:guidance_row(),
        input = current.editor:text(),
      })
      return
    end

    roots.action("frontend_diagnostic", { reason = "unknown_event", kind = tostring(kind) })
  end,
})
