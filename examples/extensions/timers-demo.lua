-- Exerciser for the scoped timer mechanisms: pi.set_timeout / pi.set_interval
-- / pi.clear_timeout / pi.clear_interval. Translations from Node
-- `setTimeout`/`setInterval`/`clearTimeout`/`clearInterval` that the dogfood
-- suite (compact, context-janitor, pomodoro, working-indicator, rlm, vcc)
-- relies on. Timers are scoped to the current dispatch: a cleared timer never
-- fires, and a pending timer drops with its dispatch.
local pi = ...

pi.register_command("timers-demo", {
  description = "Exercise setTimeout/setInterval/clear lifecycle",
  handler = function()
    local order = {}
    -- setTimeout fires once.
    pi.set_timeout(20, function()
      order[#order + 1] = "timeout"
    end)
    -- setTimeout that fires after the caller checks (should not fire here).
    pi.set_timeout(200, function()
      order[#order + 1] = "late-timeout"
    end)
    -- setInterval fires repeatedly.
    local ticks = 0
    local timer = pi.set_interval(15, function()
      ticks = ticks + 1
      order[#order + 1] = "tick"
      if ticks >= 3 then
        pi.clear_interval(timer)
      end
    end)
    -- A timeout cleared before it fires must never fire.
    local cleared = pi.set_timeout(10, function()
      order[#order + 1] = "should-never-fire"
    end)
    pi.clear_timeout(cleared)
    -- Give the timers time to run; the handler awaits so the task set advances.
    pi.sleep(120)
    return { order = order, ticks = ticks }
  end,
})