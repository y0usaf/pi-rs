-- Exerciser for tool renderCall/renderResult on the public surface.
--
-- In pi, tool definitions carry optional renderers
-- (core/extensions/types.ts ToolDefinition.renderCall / renderResult /
-- renderShell); the interactive transcript's ToolExecutionComponent
-- prefers them over its generic fallback. pi-rs keeps the same contract on
-- pi.register_tool: a renderer receives (args, theme, context) — context
-- carrying args/toolCallId/state/cwd/executionStarted/argsComplete/
-- isPartial/expanded/showImages/isError — and returns a component:
-- `function(width) -> lines`. Default-shell renderers are framed by the
-- transcript's status Box; `renderShell = "self"` tools frame themselves.
local pi = ...

pi.register_tool({
  name = "render-demo",
  label = "render-demo",
  description = "Demonstrates custom tool renderers",
  parameters = {
    type = "object",
    properties = {
      target = { type = "string", description = "What to greet" },
    },
    required = { "target" },
  },
  execute = function(_id, params)
    return { content = { { type = "text", text = "greeted " .. params.target } } }
  end,
  renderCall = function(args, theme, context)
    local marker = context.expanded and "[expanded] " or ""
    local text = theme:fg("toolTitle", theme:bold("render-demo"))
      .. " " .. theme:fg("accent", marker .. (args and args.target or "?"))
    return function(width)
      return pi.tui.text_render(text, width, 0, 0)
    end
  end,
  renderResult = function(result, options, theme, _context)
    local first = result.content[1]
    local text = "\n" .. theme:fg("toolOutput", (first and first.text or "")
      .. (options.isPartial and " …" or ""))
    return function(width)
      return pi.tui.text_render(text, width, 0, 0)
    end
  end,
})
