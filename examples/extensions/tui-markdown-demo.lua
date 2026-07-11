-- Exercises the markdown mechanism through the public Lua seam: plain
-- rendering, theme style functions, default text style, and marker options.
local pi = ...
pi.register_command("tui-markdown-demo", {
  description = "Render markdown cells",
  handler = function()
    local plain = pi.tui.markdown_render("# Ready\n\n- exact\n- deterministic", 32, 1, 0)
    local themed = pi.tui.markdown_render("**bold** `code`\n\n3) kept", 32, 0, 0, {
      theme = {
        bold = function(text) return "\27[1m" .. text .. "\27[22m" end,
        code = function(text) return "\27[36m" .. text .. "\27[39m" end,
      },
      color = function(text) return "\27[38;5;250m" .. text .. "\27[39m" end,
      preserve_ordered_list_markers = true,
    })
    return { plain = plain, themed = themed, args_json = pi.json.encode({ path = "a.txt" }, true) }
  end,
})
