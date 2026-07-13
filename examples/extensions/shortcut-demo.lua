-- Exercises pi.register_shortcut (spec registerShortcut, loader.ts) through
-- the public surface: registration, the resolved registered_shortcuts view,
-- and a handler driven with the frontend's ExtensionContext slice.
local pi = ...

local fired = {}

pi.register_shortcut("ctrl+x", {
  description = "Record a ping and notify",
  handler = function(ctx)
    fired[#fired + 1] = { idle = ctx.isIdle(), cwd = ctx.cwd, mode = ctx.mode }
    ctx.ui.notify("shortcut ping", "info")
  end,
})

-- Re-registration of the same key replaces the handler but keeps its slot.
pi.register_shortcut("CTRL+X", {
  description = "Replacement wins",
  handler = function(ctx)
    fired[#fired + 1] = { replaced = ctx.mode == "tui" and ctx.hasUI == true }
    ctx.ui.notify("replaced ping", "info")
  end,
})

pi.register_command("shortcut-demo", {
  description = "Inspect and invoke the registered shortcut",
  handler = function()
    local shortcuts = pi.registered_shortcuts()
    local notices = {}
    for _, shortcut in ipairs(shortcuts) do
      shortcut.handler({
        cwd = pi.cwd(), mode = "tui", hasUI = true,
        isIdle = function() return true end,
        hasPendingMessages = function() return false end,
        abort = function() end,
        shutdown = function() end,
        ui = { notify = function(message) notices[#notices + 1] = message end },
      })
    end
    local listed = {}
    for index, shortcut in ipairs(shortcuts) do
      listed[index] = { shortcut = shortcut.shortcut, description = shortcut.description }
    end
    return { shortcuts = listed, notices = notices, fired = fired }
  end,
})
