-- Translation of Pi v0.79.0 examples/extensions/shutdown-command.ts.
local pi = ...

pi.register_command("quit", {
  description = "Exit pi cleanly",
  handler = function(_args, ctx)
    ctx.shutdown()
  end,
})

pi.register_tool({
  name = "finish_and_exit",
  label = "Finish and Exit",
  description = "Complete a task and exit pi",
  parameters = { type = "object", properties = {}, required = pi.json.decode("[]") },
  execute = function(_tool_call_id, _params, _signal, _on_update, ctx)
    ctx.shutdown()
    return {
      content = { { type = "text", text = "Shutdown requested. Exiting after this response." } },
      details = {},
    }
  end,
})

pi.register_tool({
  name = "deploy_and_exit",
  label = "Deploy and Exit",
  description = "Deploy the application and exit pi",
  parameters = {
    type = "object",
    properties = {
      environment = {
        type = "string",
        description = "Target environment (e.g., production, staging)",
      },
    },
    required = { "environment" },
  },
  execute = function(_tool_call_id, params, _signal, on_update, ctx)
    if on_update then
      on_update({
        content = { { type = "text", text = "Deploying to " .. params.environment .. "..." } },
        details = {},
      })
      on_update({
        content = { { type = "text", text = "Deployment complete, exiting..." } },
        details = {},
      })
    end
    ctx.shutdown()
    return {
      content = { { type = "text", text = "Done! Shutdown requested." } },
      details = { environment = params.environment },
    }
  end,
})
