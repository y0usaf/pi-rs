-- Translation of Pi v0.79.0 examples/extensions/custom-header.ts.
-- Replaces the built-in header (logo + keybinding hints) with a custom
-- component showing the pi mascot.
local pi = ...

-- --- PI MASCOT --- (based on pi_mascot.ts)
local function get_pi_mascot(theme)
  -- Colors: piBlue = accent, white/black via text/dim
  local function pi_blue(text) return theme:fg("accent", text) end
  local function white(text) return text end
  local function black(text) return theme:fg("dim", text) end

  local BLOCK = "█"
  local PUPIL = "▌"

  local eye = white(BLOCK) .. black(PUPIL)
  local line_eyes = "     " .. eye .. "  " .. eye
  local line_bar = "  " .. pi_blue(string.rep(BLOCK, 14))
  local line_leg = "     " .. pi_blue(string.rep(BLOCK, 2)) .. "    " .. pi_blue(string.rep(BLOCK, 2))

  return { "", line_eyes, line_bar, line_leg, line_leg, line_leg, line_leg, "" }
end

pi.on("session_start", function(_event, ctx)
  if ctx.mode == "tui" then
    ctx.ui.setHeader(function(_tui, theme)
      return {
        render = function(_width)
          local mascot_lines = get_pi_mascot(theme)
          local subtitle = theme:fg("muted", "   shitty coding agent") .. theme:fg("dim", " v0.79.0")
          local lines = {}
          for _, line in ipairs(mascot_lines) do lines[#lines + 1] = line end
          lines[#lines + 1] = subtitle
          return lines
        end,
        invalidate = function() end,
      }
    end)
  end
end)

-- Command to restore built-in header
pi.register_command("builtin-header", {
  description = "Restore built-in header with keybinding hints",
  handler = function(_args, ctx)
    ctx.ui.setHeader(nil)
    ctx.ui.notify("Built-in header restored", "info")
  end,
})