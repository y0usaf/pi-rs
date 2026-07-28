-- Distribution defaults: the only policy the shipped manifest adds on top of
-- the agent, tool, and frontend packages.
--
-- The shipped application root configures a model when the startup event
-- carries one. Nothing in Rust invents that model: this package is ordinary
-- Lua that picks the first catalog model from a declared candidate list and
-- injects it into the startup event through public application event
-- middleware. A package loaded later may replace this stage (same id, same
-- kind/phase) or simply configure another model.
--
-- Credentials are never read here. `pi.models.v1.stream` resolves the
-- provider's supported credential itself, so a correct key is the only thing
-- a live run needs; a missing or rejected one settles as an agent error and
-- the frontend renders its credential guidance.
--
-- This package also decides the one thing the shipped tool package leaves
-- open: which directory the shipped tools treat as the workspace. The
-- distribution answer is the launcher root, published on every application
-- dispatch as `snapshot.context.root`.

local pi = ...
local models = pi.models.v1
local module = pi.kernel.v1.module
local middleware = pi.roots.v1.middleware

-- Ordered product preference. The first candidate present in the model
-- catalog wins; an empty catalog leaves the startup event untouched and the
-- frontend shows its "no model selected" guidance.
local CANDIDATES = {
  { provider = "anthropic", id = "claude-sonnet-4-5" },
  { provider = "openai", id = "gpt-5.1" },
  { provider = "openrouter", id = "anthropic/claude-sonnet-4.5" },
}

-- Snapshot payloads are read-only views: anything sent onward is copied into
-- a plain table first.
local function clone(value)
  if type(value) ~= "table" then
    return value
  end
  local copy = {}
  for key, item in pairs(value) do
    copy[key] = clone(item)
  end
  return copy
end

local function default_model()
  for _, candidate in ipairs(CANDIDATES) do
    local model = models.find(candidate.provider, candidate.id)
    if type(model) == "table" then
      return model
    end
  end
  return nil
end

middleware.register({
  kind = "application",
  phase = "event",
  id = "pi.builtins.defaults.model",
  order = -100,
  handler = function(snapshot)
    local event = snapshot.event
    if type(event) ~= "table" then
      return nil
    end
    if event.kind ~= "startup" or event.model ~= nil then
      return nil
    end
    local model = default_model()
    if model == nil then
      return nil
    end
    local next_event = clone(event)
    next_event.model = model
    return { event = next_event }
  end,
})

-- Workspace root for the shipped tools. The tool package declares its tools
-- at load time, before any launcher context exists; the distribution
-- re-declares them once the first application dispatch publishes the root, so
-- relative tool paths resolve against the launcher root instead of whatever
-- directory the process happens to sit in. A distribution without the tool
-- package (or with a replacement suite) is left untouched.
local tool_root = nil

local function apply_tool_root(root)
  if type(root) ~= "string" or #root == 0 or root == tool_root then
    return
  end
  local found, suite = pcall(module.require, "pi.tools.suite", "1")
  if not found or type(suite) ~= "table" then
    return
  end
  local declared, registry = pcall(module.require, "pi.agent.tools", "1")
  if not declared or type(registry) ~= "table" then
    return
  end
  suite.unregister(registry)
  suite.declare(registry, { shared = { root = root } })
  tool_root = root
end

middleware.register({
  kind = "application",
  phase = "event",
  id = "pi.builtins.defaults.tool-root",
  order = -99,
  handler = function(snapshot)
    local context = snapshot.context
    if type(context) == "table" then
      apply_tool_root(context.root)
    end
    return nil
  end,
})
