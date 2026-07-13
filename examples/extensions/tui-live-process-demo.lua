-- Public live process + retained-display mechanism demo. Run in a real
-- terminal; e hands the terminal to a child, z suspends the process group,
-- s suspends one callback, and q exits.
local pi = ...

local function batch(columns, rows, lines)
  return {
    version = pi.tui.display_schema_version,
    viewport = { columns = columns, rows = rows },
    root = 1,
    nodes = {
      {
        id = 1,
        rect = { x = 0, y = 0, width = columns, height = rows },
        clip_children = true,
        content = { kind = "group" },
        children = { 2 },
      },
      {
        id = 2,
        rect = { x = 0, y = 0, width = columns, height = rows },
        clip_children = true,
        content = {
          kind = "text",
          wrap = "grapheme",
          runs = { { text = table.concat(lines, "\n") } },
        },
      },
    },
  }
end

pi.register_command("tui-live-process-demo", {
  description = "Drive live process events, retained display, children, and cleanup",
  handler = function()
    local process = pi.tui.display_process()
    local columns, rows = 80, 24
    local lines = { "Live display process", "Press e for child, z to suspend, s to await, q to exit" }
    local function frame()
      return batch(columns, rows, lines)
    end
    local reason, signal = process:run(function(event)
      if event.type == "input" and event.data == "e" then
        return {
          inherited_process = {
            id = "demo-child",
            program = "sh",
            args = { "-c", "printf 'Inherited child owns the terminal\\n'" },
            message = "Leaving the display for an inherited child...\n",
          },
        }
      end
      if event.type == "inherited_process_result" and event.id == "demo-child" then
        lines[2] = "Inherited child exited with " .. tostring(event.status) .. "; q exits"
        return { display = frame() }
      end
      if event.type == "input" and event.data == "z" then
        return { suspend = true }
      end
      if event.type == "input" and event.data == "s" then
        pi.sleep(2000)
        lines[2] = "Suspended callback completed; press q to exit"
        return { display = frame() }
      end
      if event.type == "input" and (event.data == "q" or event.data == "\3") then
        return { exit = true }
      end
      if event.type == "signal" then
        return { exit = true }
      end
      if event.type == "start" or event.type == "resize" then
        columns, rows = event.columns, event.rows
        lines[1] = string.format("Live display process (%dx%d)", columns, rows)
        return { display = frame(), title = "pi-rs display process" }
      end
      return nil
    end)
    return { reason = reason, signal = signal }
  end,
})
