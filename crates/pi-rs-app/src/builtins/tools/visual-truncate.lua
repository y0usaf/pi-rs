-- modes/interactive/components/visual-truncate.ts — truncate text to the
-- last N visual lines, accounting for wrapping at the render width, via
-- the pi-tui Text mechanism.
local function truncate_to_visual_lines(text, max_visual_lines, width, padding_x)
  if text == "" then
    return { visualLines = {}, skippedCount = 0 }
  end
  local all = pi.tui.text_render(text, width, padding_x or 0, 0)
  if #all <= max_visual_lines then
    return { visualLines = all, skippedCount = 0 }
  end
  local out = {}
  for i = #all - max_visual_lines + 1, #all do out[#out + 1] = all[i] end
  return { visualLines = out, skippedCount = #all - max_visual_lines }
end

-- Public module consumed by the interactive builtin and ordinary packages.
pi.module.define({
  name = "pi.tui.visual-truncate",
  version = "1",
  dependencies = {},
  factory = function()
    return { truncate_to_visual_lines = truncate_to_visual_lines }
  end,
})
