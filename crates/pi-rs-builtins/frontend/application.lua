-- Shipped application root: the coordinator.
--
-- It owns no display, no turn policy, and no tool: it routes host events to
-- the frontend root, turns frontend intents into agent events, and feeds the
-- agent's settled actions back to the frontend for presentation. Only
-- mechanism actions (`ansi`, `shutdown`) are republished to Rust; every
-- product action stays inside the Lua roots.
--
-- Each cross-root call is one public `roots.v1.dispatch`, so replacing either
-- root — or wrapping them in middleware — needs no change here.

local pi = ...
local roots = pi.roots.v1

local session = {
  model = nil,
  model_label = nil,
}

local ACTION_ANSI = "ansi"

-- Snapshot payloads are read-only views. Anything kept across dispatches, or
-- sent back through another dispatch, is copied into a plain table first.
local function clone(value)
  if type(value) ~= "table" then
    return value
  end
  local copy = {}
  for key, item in pairs(value) do
    copy[key] = clone(item)
  end
  return copy
end

-- Republish only the display actions Rust interprets; product actions stay
-- inside the Lua roots.
local function present(batch)
  for _, action in ipairs(batch.actions) do
    if action.kind == ACTION_ANSI then
      roots.action(ACTION_ANSI, { data = action.payload.data })
    end
  end
end

local function intents(batch)
  local found = {}
  for _, action in ipairs(batch.actions) do
    local kind = action.kind
    if kind == "frontend_submit" or kind == "frontend_interrupt" or kind == "frontend_exit" then
      found[#found + 1] = { kind = kind, payload = action.payload }
    end
  end
  return found
end

-- Agent batches cross back as ordinary data; the frontend receives them as a
-- plain event rather than through any private channel.
local function show_agent(batch)
  local actions = {}
  for _, action in ipairs(batch.actions) do
    actions[#actions + 1] = { kind = action.kind, payload = clone(action.payload) }
  end
  present(roots.dispatch("frontend", { kind = "agent", actions = actions }))
end

local function to_frontend(event)
  present(roots.dispatch("frontend", event))
end

local function configure(model)
  session.model = clone(model)
  session.model_label = type(session.model) == "table" and tostring(session.model.id) or nil
  roots.dispatch("agent", { kind = "configure", model = session.model })
  to_frontend({ kind = "configure", model = session.model_label })
end

local function run_prompt(text)
  local batch = roots.dispatch("agent", {
    kind = "prompt",
    text = text,
    model = session.model,
  })
  show_agent(batch)
end

local function shutdown(reason)
  to_frontend({ kind = "shutdown", reason = reason })
  roots.action("shutdown", { reason = reason })
end

local function handle_intent(intent)
  if intent.kind == "frontend_submit" then
    -- A submitted line reaches the agent as a prompt when it is idle and as
    -- a queue event while it works. The frontend names which one it meant;
    -- the agent decides whether the queue accepts it.
    local queue = tostring(intent.payload.queue or "")
    if queue == "steer" or queue == "follow_up" then
      show_agent(roots.dispatch("agent", {
        kind = queue,
        text = tostring(intent.payload.text or ""),
      }))
      return false
    end
    run_prompt(tostring(intent.payload.text or ""))
    return false
  end
  if intent.kind == "frontend_interrupt" then
    show_agent(roots.dispatch("agent", { kind = "interrupt" }))
    return false
  end
  if intent.kind == "frontend_exit" then
    shutdown(tostring(intent.payload.reason or "user exit"))
    return true
  end
  return false
end

roots.register({
  kind = "application",
  id = "pi.builtins.application",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    local event = snapshot.event
    local kind = type(event) == "table" and event.kind or nil

    if kind == "configure" then
      configure(event.model)
      return
    end

    if kind == "startup" then
      if event.model ~= nil then
        configure(event.model)
      end
      -- The launcher measured the terminal before the first dispatch, so the
      -- first frame is already the right size instead of repainting after a
      -- resize event.
      local terminal = type(snapshot.context) == "table" and snapshot.context.terminal or nil
      to_frontend({
        kind = "startup",
        columns = type(terminal) == "table" and terminal.columns or nil,
        rows = type(terminal) == "table" and terminal.rows or nil,
      })
      return
    end

    if kind == "input" then
      local batch = roots.dispatch("frontend", {
        kind = "input",
        data = tostring(event.data or ""),
      })
      present(batch)
      for _, intent in ipairs(intents(batch)) do
        if handle_intent(intent) then
          return
        end
      end
      return
    end

    if kind == "resize" then
      to_frontend({
        kind = "resize",
        columns = event.columns,
        rows = event.rows,
      })
      return
    end

    if kind == "prompt" then
      -- Programmatic prompt: same path as a submitted one.
      to_frontend({ kind = "notice", level = "info", text = tostring(event.text or "") })
      run_prompt(tostring(event.text or ""))
      return
    end

    if kind == "shutdown" then
      shutdown(tostring(event.reason or "host shutdown"))
      return
    end

    to_frontend({
      kind = "notice",
      level = "warn",
      text = "unhandled event: " .. tostring(kind),
    })
  end,
})
