-- Translation of Pi v0.79.0 examples/extensions/model-status.ts.
-- Shows model changes in the status bar via the model_select hook.
local pi = ...

pi.on("model_select", function(event, ctx)
  local model, previous_model, source = event.model, event.previousModel, event.source

  -- Format model identifiers
  local next = model.provider .. "/" .. model.id
  local prev = previous_model and (previous_model.provider .. "/" .. previous_model.id) or "none"

  -- Show notification on change
  if source ~= "restore" then
    ctx.ui.notify("Model: " .. next, "info")
  end

  -- Update status bar with current model
  ctx.ui.setStatus("model", "🤖 " .. model.id)
end)