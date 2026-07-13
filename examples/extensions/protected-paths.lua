-- Translation of Pi v0.79.0 examples/extensions/protected-paths.ts.
local pi = ...

local protected_paths = { ".env", ".git/", "node_modules/" }

pi.on("tool_call", function(event, ctx)
  if event.toolName ~= "write" and event.toolName ~= "edit" then return nil end

  local path = event.input.path
  for _, protected in ipairs(protected_paths) do
    if path:find(protected, 1, true) then
      if ctx.hasUI then
        ctx.ui.notify("Blocked write to protected path: " .. path, "warning")
      end
      return { block = true, reason = 'Path "' .. path .. '" is protected' }
    end
  end
  return nil
end)
