-- Shipped core-tool package. This file owns declaration only: each tool's
-- behaviour lives in its own module, and every tool is declared through the
-- one tool declaration path (`pi.agent.tools`). A package loaded from disk may
-- suppress one tool (`suite.unregister(registry, "bash")`) or replace it by
-- unregistering and declaring its own, without disturbing the others.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.suite",
  version = "1",
  dependencies = {
    read = { name = "pi.tools.read", version = "1" },
    write = { name = "pi.tools.write", version = "1" },
    edit = { name = "pi.tools.edit", version = "1" },
    bash = { name = "pi.tools.bash", version = "1" },
  },
  factory = function(deps)
    local ORDER = { "read", "write", "edit", "bash" }

    local function names()
      local list = {}
      for index, name in ipairs(ORDER) do
        list[index] = name
      end
      return list
    end

    -- `options.suppress` drops tools; `options.tools[name]` overrides one
    -- tool's settings; anything else applies to all of them.
    local function declare(registry, options)
      options = options or {}
      local suppress = options.suppress or {}
      local overrides = options.tools or {}
      local declared = {}
      for _, name in ipairs(ORDER) do
        if suppress[name] ~= true then
          local settings = overrides[name]
          if settings == nil then
            settings = options.shared
          end
          declared[#declared + 1] = deps[name].declare(registry, settings)
        end
      end
      return declared
    end

    local function unregister(registry, name)
      if name == nil then
        local removed = 0
        for _, entry in ipairs(ORDER) do
          if deps[entry].unregister(registry) then
            removed = removed + 1
          end
        end
        return removed
      end
      local tool = deps[name]
      if tool == nil then
        return false
      end
      return tool.unregister(registry)
    end

    return {
      names = names,
      declare = declare,
      unregister = unregister,
      tools = deps,
    }
  end,
})

local suite = module.require("pi.tools.suite", "1")
local registry = module.require("pi.agent.tools", "1")

suite.declare(registry)
