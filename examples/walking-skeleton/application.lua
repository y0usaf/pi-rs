-- Walking skeleton application root: the coordinator. It owns no display,
-- no turn logic, and no policy beyond routing: startup frames come from the
-- frontend root, input bytes are parsed by the frontend root, and each key
-- becomes one turn through the agent root. Every cross-root call goes
-- through the public roots.v1.dispatch seam and every action is republished
-- explicitly; nothing crosses roots but immutable snapshots and batches.

local pi = ...
local roots = pi.roots.v1

-- Republish another root's queued actions into this dispatch's batch.
local function forward(batch)
  for _, action in ipairs(batch.actions) do
    roots.action(action.kind, action.payload)
  end
end

roots.register({
  kind = "application",
  id = "walking-skeleton",
  dispatch = function(snapshot)
    local kind = snapshot.event.kind

    if kind == "startup" then
      -- The launcher measures the terminal before the first dispatch and
      -- passes it in the context; later changes arrive as `resize` events.
      local terminal = type(snapshot.context) == "table" and snapshot.context.terminal or nil
      forward(roots.dispatch("frontend", {
        kind = "startup",
        columns = type(terminal) == "table" and terminal.columns or nil,
        rows = type(terminal) == "table" and terminal.rows or nil,
      }))
      return
    end

    if kind == "resize" then
      forward(roots.dispatch("frontend", {
        kind = "resize",
        columns = snapshot.event.columns,
        rows = snapshot.event.rows,
      }))
      return
    end

    if kind == "input" then
      -- The frontend decodes raw bytes into keys.
      local parsed = roots.dispatch("frontend", {
        kind = "input",
        data = snapshot.event.data or "",
      })
      local keys = nil
      for _, action in ipairs(parsed.actions) do
        if action.kind == "keys" then
          keys = action.payload.keys
        end
      end
      if not keys then
        return
      end
      for _, key in ipairs(keys) do
        if key == "q" then
          roots.action("shutdown", { reason = "user quit" })
          return
        end
        -- One agent turn per key; the agent renders through the frontend.
        forward(roots.dispatch("agent", { kind = "turn", key = key }))
      end
      return
    end
  end,
})
