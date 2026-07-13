-- Translation of Pi v0.79.0 examples/extensions/structured-output.ts.
local pi = ...

pi.register_tool({
  name = "structured_output",
  label = "Structured Output",
  description = "Return a final structured answer. Use this as your last action when the user asks for structured output or a machine-readable summary.",
  promptSnippet = "Emit a final structured answer as a terminating tool result",
  promptGuidelines = {
    "Use structured_output as your final action when the user asks for structured output, JSON-like output, or a machine-readable summary.",
    "After calling structured_output, do not emit another assistant response in the same turn.",
  },
  parameters = {
    type = "object",
    properties = {
      headline = { type = "string", description = "Short title for the result" },
      summary = { type = "string", description = "One-paragraph summary" },
      actionItems = {
        type = "array",
        items = { type = "string" },
        description = "Concrete next steps or key bullets",
      },
    },
    required = { "headline", "summary", "actionItems" },
  },
  execute = function(_tool_call_id, params)
    return {
      content = { { type = "text", text = "Saved structured output: " .. params.headline } },
      details = {
        headline = params.headline,
        summary = params.summary,
        actionItems = params.actionItems,
      },
      terminate = true,
    }
  end,
  renderResult = function(result, _options, theme)
    local details = result.details
    if not details then
      local text = result.content[1]
      return pi.tui.text(text and text.type == "text" and text.text or "", 0, 0)
    end

    local lines = {
      theme:fg("toolTitle", theme:bold(details.headline)),
      theme:fg("text", details.summary),
      "",
    }
    for index, item in ipairs(details.actionItems) do
      lines[#lines + 1] = theme:fg("muted", index .. ". " .. item)
    end
    return pi.tui.text(table.concat(lines, "\n"), 0, 0)
  end,
})
