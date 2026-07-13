-- Translation-style exerciser for Pi's status/widget/header/footer/dialog/custom UI.
local pi = ...

pi.register_ui_slot("status", {
  name = "ui-showcase-status", order = -100,
  render = function(_, context, next_render)
    local label = pi.tui.slice_by_column("Showcase status slot", 0, context.width)
    local lines = { context.theme:fg("dim", label) }
    for _, line in ipairs(next_render()) do lines[#lines + 1] = line end
    return lines
  end,
})

pi.register_command("ui-showcase", {
  description = "Exercise the composable extension UI surface",
  handler = function(_, ctx)
    ctx.ui.setStatus("showcase", ctx.ui.theme:fg("accent", "UI showcase"))
    ctx.ui.setWidget("showcase-above", { "Above editor widget" })
    ctx.ui.setWidget("showcase-below", { "Below editor widget" }, { placement = "belowEditor" })
    ctx.ui.setHeader(function(_, theme)
      return { render = function() return { theme:bold(theme:fg("accent", "Custom header")) } end }
    end)
    ctx.ui.setFooter(function(_, theme, data)
      return { render = function() return { theme:fg("dim", "Custom footer " .. table.concat(data.getExtensionStatuses(), " ")) } end }
    end)
    ctx.ui.setTitle("pi · showcase")
    ctx.ui.setWorkingMessage("Building showcase...")
    ctx.ui.setWorkingIndicator({ frames = { "●" }, intervalMs = 100 })
    ctx.ui.setHiddenThinkingLabel("Reasoning hidden")
    ctx.ui.setEditorText("seed")
    ctx.ui.pasteToEditor(" + paste")
    ctx.ui.setToolsExpanded(true)

    local name = ctx.ui.input("Your name", "Ada", { timeout = 5000 })
    if not name then ctx.ui.notify("Input cancelled", "warning"); return end
    local notes = ctx.ui.editor("Notes", "hello")
    if not notes then ctx.ui.notify("Editor cancelled", "warning"); return end
    local accepted = ctx.ui.custom(function(_, theme, _, done)
      return {
        render = function(_, width)
          return pi.tui.text_render(theme:fg("accent", "Hello " .. name .. ": " .. notes)
            .. "\n" .. theme:fg("dim", "Enter to close"), width, 1, 0)
        end,
        handle_input = function(_, data)
          if data == "\r" or data == "\n" then done(true) end
        end,
        dispose = function() end,
      }
    end)
    ctx.ui.notify(accepted and "Showcase complete" or "Showcase closed", "info")
  end,
})
