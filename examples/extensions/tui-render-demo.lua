-- File-backed exerciser for the versioned retained-display batch boundary.
local pi = ...

local function batch(text, cursor_visible)
  return {
    version = pi.tui.display_schema_version,
    viewport = { columns = 12, rows = 3 },
    root = 1,
    nodes = {
      {
        id = 1,
        rect = { x = 0, y = 0, width = 12, height = 3 },
        clip_children = true,
        content = { kind = "group" },
        children = { 2 },
      },
      {
        id = 2,
        rect = { x = 1, y = 1, width = 10, height = 1 },
        clip_children = true,
        focusable = true,
        content = {
          kind = "text",
          wrap = "clip",
          runs = { { text = text, style = { bold = true } } },
        },
      },
    },
    focused = 2,
    cursor = {
      node = 2,
      row = 0,
      column = 3,
      shape = "bar",
      visible = cursor_visible,
    },
  }
end

pi.register_command("tui-render-demo", {
  description = "Submit bounded retained display batches",
  handler = function()
    local display = pi.tui.display()
    local first = display:submit(batch("A界", true))
    local unchanged = display:submit(batch("A界", true))
    local changed = display:submit(batch("A好", true))
    local revision_before_error = display:revision()
    local malformed_ok, malformed_error = pcall(function()
      display:submit(batch("bad\27[2J", true))
    end)
    local revision_after_error = display:revision()
    display:reset_presentation()
    local redrawn = display:submit(batch("A好", false))
    return {
      schema_version = pi.tui.display_schema_version,
      first = first,
      unchanged = unchanged,
      changed = changed,
      malformed_ok = malformed_ok,
      malformed_error = tostring(malformed_error),
      revision_before_error = revision_before_error,
      revision_after_error = revision_after_error,
      redrawn = redrawn,
    }
  end,
})
