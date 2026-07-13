-- File-backed consumer of the same exact-version helpers used by builtins.
-- Load after the builtin tools package.
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local render = pi.module.require("pi.tools.render", "1")

pi.register_command("module-demo", {
  description = "Exercise public builtin Lua modules",
  handler = function()
    local result = truncate.truncate_head("alpha\nbeta\ngamma", { maxLines = 2 })
    return {
      content = result.content,
      path = render.shorten_path((pi.env.HOME or "") .. "/demo.txt"),
      truncated = result.truncated,
    }
  end,
})
