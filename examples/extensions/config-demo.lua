-- Canonical config declaration API, available identically to files and packs.
local pi = ...

pi.register_command("config-demo", {
  description = "Exercise Lua-only configuration declarations",
  handler = function()
    pi.config.keybindings({ ["app.exit"] = { "ctrl+d", "ctrl+q" } })
    pi.config.provider("demo", { baseUrl = "http://127.0.0.1:1" })
    pi.config.model("demo", { id = "demo-model" })
    pi.config.theme("demo", { dark = true, colors = { accent = "#abcdef" } })
    pi.config.enable("extensions", { "demo.lua" })
    local snapshot = pi.config.snapshot()
    return {
      exit = snapshot.keybindings["app.exit"],
      model = snapshot.providers.demo.models[1].id,
      dark = snapshot.themes.demo.dark,
      enabled = snapshot.selectors.extensions.enabled,
    }
  end,
})
