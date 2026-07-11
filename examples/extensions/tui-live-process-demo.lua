-- Exercises the public live process TUI driver. Run in a real terminal; press q to exit.
local pi = ...

pi.register_command("tui-live-process-demo", {
  description = "Drive live process input, rendering, resize, scheduling, and cleanup",
  handler = function()
    local process = pi.tui.process_session(true)
    local frame = { "Live process driver", "Press s to suspend a callback; q still exits" }
    local reason, signal = process:run(function(event)
      if event.type == "input" and event.data == "s" then
        -- This callback suspends, but the process keeps dispatching input/ticks;
        -- q remains usable while this task is waiting.
        pi.sleep(2000)
        frame[2] = "Suspended callback completed; press q to exit"
        return { lines = frame }
      end
      if event.type == "input" and (event.data == "q" or event.data == "\3") then
        return { exit = true }
      end
      if event.type == "signal" then
        return { exit = true }
      end
      if event.type == "start" or event.type == "resize" then
        frame[1] = string.format("Live process driver (%dx%d)", event.columns, event.rows)
        return { lines = frame, force = event.type == "resize", title = "pi-rs process TUI" }
      end
      return nil
    end)
    return { reason = reason, signal = signal }
  end,
})
