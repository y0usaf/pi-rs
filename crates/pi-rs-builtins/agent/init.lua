-- Shipped agent root. Registration is the only thing this file owns: turn
-- policy lives in `pi.agent.turn`, and any package may replace this root by
-- registering an agent root with a higher priority.

local pi = ...
local roots = pi.roots.v1
local module = pi.kernel.v1.module

local agent = nil

local function instance()
  if agent == nil then
    agent = module.require("pi.agent.turn", "1").new({})
  end
  return agent
end

roots.register({
  kind = "agent",
  id = "pi.builtins.agent",
  active = true,
  priority = 0,
  dispatch = function(snapshot)
    instance():handle(snapshot.event, function(kind, payload)
      roots.action(kind, payload)
    end)
  end,
})
