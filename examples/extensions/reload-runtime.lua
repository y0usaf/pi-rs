-- Translation of Pi v0.79.0 examples/extensions/reload-runtime.ts.
-- Demonstrates ctx.reload() from ExtensionCommandContext and an LLM-callable
-- tool that queues a follow-up command to trigger reload.
local pi = ...

-- Command entrypoint for reload.
pi.register_command("reload-runtime", {
  description = "Reload extensions, skills, prompts, and themes",
  handler = function(_args, ctx)
    ctx.reload()
  end,
})

-- LLM-callable tool. Tools get ExtensionContext, so they cannot call
-- ctx.reload() directly; queue a follow-up user command instead.
pi.register_tool({
  name = "reload_runtime",
  label = "Reload Runtime",
  description = "Reload extensions, skills, prompts, and themes",
  parameters = { type = "object", properties = {}, required = {} },
  execute = function()
    pi.sendUserMessage("/reload-runtime", { deliverAs = "followUp" })
    return {
      content = { { type = "text", text = "Queued /reload-runtime as a follow-up command." } },
      details = {},
    }
  end,
})