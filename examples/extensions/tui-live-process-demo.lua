-- Exercises the public live process TUI driver. Run in a real terminal;
-- press e to hand the terminal to an inherited child, z to suspend the
-- process group (resume with `fg`), s to suspend a callback, or q to exit.
local pi = ...

pi.register_command("tui-live-process-demo", {
  description = "Drive live process input, suspension, inherited children, rendering, resize, scheduling, and cleanup",
  handler = function()
    local process = pi.tui.process_session(true)
    local frame = { "Live process driver", "Press e for child, z to suspend, s to await, q to exit" }
    local reason, signal = process:run(function(event)
      if event.type == "input" and event.data == "e" then
        return {
          inheritedProcess = {
            id = "demo-child",
            program = "sh",
            args = { "-c", "printf 'Inherited child owns the terminal\\n'" },
            message = "Leaving the TUI for an inherited child...\n",
          },
        }
      end
      if event.type == "inherited_process_result" and event.id == "demo-child" then
        frame[2] = "Inherited child exited with " .. tostring(event.status) .. "; q exits"
        return { lines = frame, force = true }
      end
      if event.type == "input" and event.data == "z" then
        return { suspend = true }
      end
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
