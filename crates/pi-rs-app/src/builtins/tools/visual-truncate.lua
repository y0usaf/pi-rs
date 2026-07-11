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

-- Cross-pack export: components/bash-execution.ts (interactive pack,
-- PLAN 7.1) shares the spec's truncateToVisualLines.
visual_truncate_lib = { truncate_to_visual_lines = truncate_to_visual_lines }
