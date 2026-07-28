-- Walking skeleton middleware package: an ordinary file-backed package that
-- composes one event stage and one render stage around roots it does not own.
--
-- Middleware is mechanism, not privilege: this package holds no reference to
-- the agent or application root, uses no private API, and can be deleted
-- without touching them. Rust owns stage ordering and the per-stage watchdog;
-- the meaning of every payload below stays Lua policy.

local pi = ...
local middleware = pi.roots.v1.middleware

-- Event stage: normalise the key an agent turn sees. The agent root only
-- understands lowercase keys, so typing 'R' would otherwise echo instead of
-- running the effect demo. The stage replaces the event the root receives;
-- it queues no actions and does not stop the chain.
middleware.register({
  kind = "agent",
  phase = "event",
  id = "key-normalize",
  order = 10,
  handler = function(snapshot)
    local event = snapshot.event
    if event.kind ~= "turn" or type(event.key) ~= "string" then
      return nil
    end
    return {
      event = { kind = event.kind, key = event.key:lower() },
    }
  end,
})

-- Render stage: transform the application's settled action list. Any batch
-- that presented a frame gets one extra ANSI action marking row 2, so the
-- composition is observable from outside the process. Returning a table
-- whose `actions` array replaces the list is the only way to change a
-- settled batch; the stage cannot mutate host state.
--
-- Snapshot values are read-only views, so a kept action is rebuilt as a
-- plain table before it crosses back to the host.
middleware.register({
  kind = "application",
  phase = "render",
  id = "frame-marker",
  order = 10,
  handler = function(snapshot)
    local rendered = false
    local next_actions = {}
    for _, action in ipairs(snapshot.actions) do
      if action.kind == "ansi" then
        rendered = true
      end
      local payload = {}
      for key, value in pairs(action.payload) do
        payload[key] = value
      end
      next_actions[#next_actions + 1] = { kind = action.kind, payload = payload }
    end
    if not rendered then
      return nil
    end
    next_actions[#next_actions + 1] = {
      kind = "ansi",
      payload = { data = "\27[2;1H[mw]" },
    }
    return { actions = next_actions }
  end,
})
