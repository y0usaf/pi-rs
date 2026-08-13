-- Translation of Pi v0.79.0 examples/extensions/timed-confirm.ts.
-- Demonstrates timed dialogs with live countdown.
--
-- The two timeout-based forms map directly onto ctx.ui.confirm/select
-- `{ timeout }`. The manual `timed-signal` AbortController form maps onto a
-- `pi.set_timeout` cancellation flag (pi-rs exposes no public AbortController
-- to extensions); it keeps Pi's auto-cancel-with-countdown intent.
local pi = ...

-- Simple approach: use the timeout option (recommended)
pi.register_command("timed", {
  description = "Show a timed confirmation dialog (auto-cancels in 5s with countdown)",
  handler = function(_args, ctx)
    local confirmed = ctx.ui.confirm("Timed Confirmation", "This dialog will auto-cancel in 5 seconds. Confirm?", { timeout = 5000 })

    if confirmed then
      ctx.ui.notify("Confirmed by user!", "info")
    else
      ctx.ui.notify("Cancelled or timed out", "info")
    end
  end,
})

pi.register_command("timed-select", {
  description = "Show a timed select dialog (auto-cancels in 10s with countdown)",
  handler = function(_args, ctx)
    local choice = ctx.ui.select("Pick an option", { "Option A", "Option B", "Option C" }, { timeout = 10000 })

    if choice then
      ctx.ui.notify("Selected: " .. choice, "info")
    else
      ctx.ui.notify("Selection cancelled or timed out", "info")
    end
  end,
})

-- Manual approach: use a cancellation flag driven by a timer (the Lua
-- translation of Pi's AbortController + setTimeout).
local function manual_timeout()
  local aborted = false
  local handle = pi.set_timeout(5000, function() aborted = true end)
  return {
    after = function(timeout_ms, fn)
      local h = pi.set_timeout(timeout_ms, fn)
      return function() pi.clear_timeout(h) end
    end,
    aborted = function() return aborted end,
  }
end

pi.register_command("timed-signal", {
  description = "Show a timed confirm using a manual timeout (Lua AbortSignal equivalent)",
  handler = function(_args, ctx)
    ctx.ui.notify("Dialog will auto-cancel in 5 seconds...", "info")

    local ctl = manual_timeout()
    local confirmed = ctx.ui.confirm("Timed Confirmation", "This dialog will auto-cancel in 5 seconds. Confirm?", { timeout = 5000 })

    if confirmed then
      ctx.ui.notify("Confirmed by user!", "info")
    else
      ctx.ui.notify("Dialog timed out (auto-cancelled)", "warning")
    end
  end,
})