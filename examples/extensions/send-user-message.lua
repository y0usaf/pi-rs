-- send-user-message: translation of the spec's send-user-message.ts —
-- send_user_message with steer/followUp delivery while streaming and a
-- plain send when idle (PLAN 9.4).
local pi = ...

pi.register_command("ask", {
  description = "Send a user message to the agent",
  handler = function(args, ctx)
    local text = args:match("^%s*(.-)%s*$") or ""
    if text == "" then return { error = "empty" } end
    if (type(ctx.isIdle) == "function" and not ctx.isIdle()) or ctx.isIdle == false then return { error = "busy" } end
    pi.send_user_message(text)
    return { sent = text, idle = ctx.isIdle() }
  end,
})

pi.register_command("steer", {
  description = "Send a steering message (interrupts current processing)",
  handler = function(args, ctx)
    local text = args:match("^%s*(.-)%s*$") or ""
    if text == "" then return { error = "empty" } end
    if (type(ctx.isIdle) ~= "function") or ctx.isIdle() then
      pi.send_user_message(text)
      return { sent = text, deliverAs = "default" }
    end
    pi.send_user_message(text, { deliverAs = "steer" })
    return { sent = text, deliverAs = "steer" }
  end,
})

pi.register_command("followup", {
  description = "Queue a follow-up message (waits for current processing)",
  handler = function(args, ctx)
    local text = args:match("^%s*(.-)%s*$") or ""
    if text == "" then return { error = "empty" } end
    if (type(ctx.isIdle) ~= "function") or ctx.isIdle() then
      pi.send_user_message(text)
      return { sent = text, deliverAs = "default" }
    end
    pi.send_user_message(text, { deliverAs = "followUp" })
    return { sent = text, deliverAs = "followUp" }
  end,
})

pi.register_command("askwith", {
  description = "Send a user message with structured content",
  handler = function(args, ctx)
    if (type(ctx.isIdle) == "function" and not ctx.isIdle()) or ctx.isIdle == false then return { error = "busy" } end
    pi.send_user_message({
      { type = "text", text = "User request: " .. args },
      { type = "text", text = "Please respond concisely." },
    })
    return { sent = "structured" }
  end,
})
