-- Translation of Pi v0.79.0 examples/extensions/send-user-message.ts.
-- Demonstrates pi.sendUserMessage() for sending user messages from
-- extensions: always triggers a turn when idle, or steers/queues while
-- streaming.
local pi = ...

local function trimmed_args(args)
  return args:gsub("^%s+", ""):gsub("%s+$", "")
end

-- Simple command that sends a user message
pi.register_command("ask", {
  description = "Send a user message to the agent",
  handler = function(args, ctx)
    if trimmed_args(args) == "" then
      ctx.ui.notify("Usage: /ask <message>", "warning")
      return
    end

    if not ctx.isIdle() then
      ctx.ui.notify("Agent is busy. Use /steer or /followup instead.", "warning")
      return
    end

    pi.sendUserMessage(args)
  end,
})

-- Command that steers the agent mid-conversation
pi.register_command("steer", {
  description = "Send a steering message (interrupts current processing)",
  handler = function(args, ctx)
    if trimmed_args(args) == "" then
      ctx.ui.notify("Usage: /steer <message>", "warning")
      return
    end

    if ctx.isIdle() then
      pi.sendUserMessage(args)
    else
      pi.sendUserMessage(args, { deliverAs = "steer" })
    end
  end,
})

-- Command that queues a follow-up message
pi.register_command("followup", {
  description = "Queue a follow-up message (waits for current processing)",
  handler = function(args, ctx)
    if trimmed_args(args) == "" then
      ctx.ui.notify("Usage: /followup <message>", "warning")
      return
    end

    if ctx.isIdle() then
      pi.sendUserMessage(args)
    else
      pi.sendUserMessage(args, { deliverAs = "followUp" })
      ctx.ui.notify("Follow-up queued", "info")
    end
  end,
})

-- Example with content array (text + images would go here)
pi.register_command("askwith", {
  description = "Send a user message with structured content",
  handler = function(args, ctx)
    if trimmed_args(args) == "" then
      ctx.ui.notify("Usage: /askwith <message>", "warning")
      return
    end

    if not ctx.isIdle() then
      ctx.ui.notify("Agent is busy", "warning")
      return
    end

    pi.sendUserMessage({
      { type = "text", text = "User request: " .. args },
      { type = "text", text = "Please respond concisely." },
    })
  end,
})