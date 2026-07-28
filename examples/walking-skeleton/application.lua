-- Walking skeleton: minimal application root proving the interactive loop.
--
-- This package demonstrates the generic product loop end to end: startup
-- renders an input-ready frame, typed input echoes through a retained
-- display, and a shutdown action exits cleanly.

local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

-- Shared display and input buffer survive across dispatches via the
-- package's Lua upvalues (standard Lua closure semantics).
local display = nil
local input = nil

local function ensure_display(columns, rows)
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
  kind = "application",
  id = "walking-skeleton",
  dispatch = function(snapshot)
    local kind = snapshot.event.kind

    if kind == "startup" then
      render_frame("", true)
      return
    end

    if kind == "input" then
      local data = snapshot.event.data or ""
      -- Parse input events through the bounded stdin buffer.
      local events = input:feed(data)
      for _, event in ipairs(events) do
        if event.kind == "data" then
          -- Echo the input back through the display.
          render_frame(event.data, true)
          -- Quit on 'q'.
          if event.data == "q" then
            roots.action("shutdown", { reason = "user quit" })
            return
          end
        end
      end
      return
    end
  end,
})
