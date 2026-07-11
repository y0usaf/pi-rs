-- Exercises the WS6.7 terminal-image mechanisms through the public Lua seam.
local pi = ...
pi.register_command("tui-image-demo", {
  description = "Render deterministic Kitty image protocol output",
  handler = function()
    local rendered = pi.tui.image_render("kitty", "AAAA", { width_px = 20, height_px = 20 }, { max_width_cells = 2, image_id = 42, move_cursor = false })
    return {
      rows = rendered.rows,
      image = pi.tui.is_image_line(rendered.sequence),
      fallback = pi.tui.image_fallback("image/png", 20, 20, "demo.png"),
      hyperlink = pi.tui.hyperlink("pi", "https://pi.dev"),
      deleted = pi.tui.delete_kitty_image(42),
    }
  end,
})
