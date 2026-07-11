-- Exercises the WS6.3 editor mechanism through the public Lua seam.
local pi = ...
pi.register_command("tui-editor-demo", {
  description = "Edit text with pi.tui editor primitives",
  handler = function()
    local editor = pi.tui.editor("hello")
    editor:input("\x1b[D")
    editor:insert("!")
    editor:input("\x7f")
    editor:undo()
    return {
      value = editor:value(), cursor = editor:cursor(),
      kitty = pi.tui.decode_printable("\x1b[97u"),
      key = pi.tui.decode_key("\x1b[112;6u"),
    }
  end,
})
