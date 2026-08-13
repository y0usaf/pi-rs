-- Translation of Pi v0.79.0 examples/extensions/custom-footer.ts.
-- Custom footer with git branch and token stats via ctx.ui.setFooter().
local pi = ...

local enabled = false

pi.register_command("footer", {
  description = "Toggle custom footer",
  handler = function(_args, ctx)
    enabled = not enabled

    if enabled then
      ctx.ui.setFooter(function(_tui, theme, footer_data)
        local unsub = footer_data.onBranchChange(function() end)

        return {
          dispose = unsub,
          invalidate = function() end,
          render = function(width)
            -- Compute tokens from ctx (already accessible to extensions)
            local input, output, cost = 0, 0, 0
            local branch_entries = ctx.sessionManager:get_branch()
            for _, e in ipairs(branch_entries) do
              if e.type == "message" and e.message.role == "assistant" then
                local usage = e.message.usage or {}
                input = input + (usage.input or 0)
                output = output + (usage.output or 0)
                cost = cost + ((usage.cost and usage.cost.total) or 0)
              end
            end

            local branch = footer_data.getGitBranch()
            local function fmt(n)
              if n < 1000 then return tostring(n) end
              return string.format("%.1fk", n / 1000)
            end

            local left = theme:fg("dim", "↑" .. fmt(input) .. " ↓" .. fmt(output) .. " $" .. string.format("%.3f", cost))
            local branch_str = branch and (" (" .. branch .. ")") or ""
            local right = theme:fg("dim", (ctx.model and ctx.model.id or "no-model") .. branch_str)

            local pad = string.rep(" ", math.max(1, width - pi.tui.visible_width(left) - pi.tui.visible_width(right)))
            return { pi.tui.truncate(left .. pad .. right, width, "", false) }
          end,
        }
      end)
      ctx.ui.notify("Custom footer enabled", "info")
    else
      ctx.ui.setFooter(nil)
      ctx.ui.notify("Default footer restored", "info")
    end
  end,
})