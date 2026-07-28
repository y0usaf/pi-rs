-- Walking skeleton frontend root: owns the retained display, the bounded
-- input buffer, and every rendered frame.
--
-- The frontend is an independently replaceable root. The application
-- coordinates it through the public roots.v1.dispatch seam; every ANSI frame
-- leaves as a queued action in the frontend's own batch, which the caller
-- explicitly republishes. No private channel, no shared mutable host state.

local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

-- Display and input buffer survive across dispatches via Lua upvalues.
local display = nil
local input = nil

local function ensure_display()
  if not display then
    display = terminal.display()
    input = terminal.input_buffer()
  end
  return display
end

local function render_frame(text, ready)
  local d = ensure_display()
  local label = ready and "pi> " .. text or text
  local frame = d:submit({
    version = terminal.display_schema_version,
    viewport = { columns = 80, rows = 1 },
    root = 1,
    nodes = { {
      id = 1,
      rect = { x = 0, y = 0, width = 80, height = 1 },
      content = {
        kind = "text",
        runs = { { text = label } },
      },
    } },
  })
  if frame.ansi and #frame.ansi > 0 then
    roots.action("ansi", { data = frame.ansi })
  end
  return frame
end

roots.register({
  kind = "frontend",
  id = "walking-skeleton-frontend",
  dispatch = function(snapshot)
    local kind = snapshot.event.kind

    if kind == "startup" then
      render_frame("", true)
      return
    end

    if kind == "render" then
      render_frame(snapshot.event.text or "", snapshot.event.ready == true)
      return
    end

    if kind == "input" then
      -- Parse raw bytes through the bounded stdin buffer and report the
      -- decoded keys to the caller as an ordinary action payload.
      local data = snapshot.event.data or ""
      local keys = {}
      local events = input:feed(data)
      for _, event in ipairs(events) do
        if event.kind == "data" then
          keys[#keys + 1] = event.data
        end
      end
      roots.action("keys", { keys = keys })
      return
    end
  end,
})
