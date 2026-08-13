-- File-backed consumer of the same exact-version helpers used by builtins.
-- Load after the builtin tools package. The builtin tools pack defines these
-- public modules; this on-disk package imports them through the same
-- non-privileged dependency mechanism (pi.module.require) with no hidden
-- native modules, load-order globals, or JS runtime.
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local render = pi.module.require("pi.tools.render", "1")
local mutation_queue = pi.module.require("pi.tools.file-mutation-queue", "1")
local shell = pi.module.require("pi.tools.shell", "1")

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

pi.register_command("module-demo-mutation", {
  description = "Exercise the mutation-queue module from a file-backed package",
  handler = function()
    local resolved = pi.path.resolve((pi.env.HOME or "") .. "/tmp/mutate.txt")
    local key = mutation_queue.mutation_queue_key(resolved)
    local executed = false
    mutation_queue.with_file_mutation_queue(resolved, function()
      executed = true
    end)
    return {
      key = key,
      executed = executed,
      resolved = resolved,
    }
  end,
})

pi.register_command("module-demo-shell", {
  description = "Exercise the shell module from a file-backed package",
  handler = function()
    local binary, args = shell.shell_config()
    return {
      binary = binary,
      argCount = #args,
    }
  end,
})
