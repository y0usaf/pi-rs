-- Structured tool result example; presentation is frontend policy.
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
})
