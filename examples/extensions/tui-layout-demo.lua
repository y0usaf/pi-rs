-- Exercises every WS6.8 component userdata through the public Lua seam.
local pi = ...

pi.register_command("tui-layout-demo", {
  description = "Compose and interact with TUI layout components",
  handler = function()
    local text = pi.tui.text("heading", 0, 0)
    text:set_text("title")

    local input = pi.tui.input("ab")
    input:set_value("xy")
    input:set_focused(true)
    local changed = input:input("z")
    local submitted = input:input("\r")

    local settings = pi.tui.settings_list({
      { id = "theme", label = "Theme", description = "Color theme", current_value = "dark", values = { "dark", "light" } },
      { id = "mode", label = "Mode", current_value = "safe", values = { "safe", "fast" } },
    }, 2, true)
    settings:set_query("mode")
    settings:move_down()
    settings:move_up()
    local selected = settings:selected()
    local action = settings:activate()
    local settings_changed = settings:input("\r")
    local settings_cancelled = settings:input("\27")
    settings:update_value("mode", "safe")
    local settings_lines = settings:render(32)

    local spacer = pi.tui.spacer(1)
    spacer:set_lines(2)
    local truncated = pi.tui.truncated_text("abcdefghijkl", 1, 0)
    truncated:set_text("abcdefghijk")

    local box = pi.tui.box(1, 1, function(line) return "[" .. line .. "]" end)
    box:add(text)
    box:add(spacer)
    box:add(truncated)
    box:remove(spacer)
    box:add(input)
    local lines = box:render(12)
    box:clear()

    return {
      lines = lines,
      empty = box:render(12),
      input_value = input:value(),
      changed = changed,
      submitted = submitted,
      selected = selected,
      action = action,
      settings_changed = settings_changed,
      settings_cancelled = settings_cancelled,
      settings_lines = settings_lines,
      spacer_lines = spacer:render(12),
      truncated_lines = truncated:render(8),
      text_lines = text:render(8),
    }
  end,
})
