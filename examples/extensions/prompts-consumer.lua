-- File-backed consumer of the public prompt-template module
-- (pi.interactive.prompts). Load after the interactive builtin pack. Uses the
-- same exact-version dependency mechanism as builtins — no hidden native
-- module or JS runtime.
local pi = ...

local prompts = pi.module.require("pi.interactive.prompts", "1")

pi.register_command("prompts-consumer", {
  description = "Exercise the public prompt-template module from a file-backed package",
  handler = function(args)
    local input = pi.json.decode(args or "{}")
    local argv = input.args or {}
    return {
      tokens = prompts.parse_command_args(table.concat(argv, " ")),
      substituted = prompts.substitute_args("$@", argv),
      expanded = prompts.expand_prompt_template(
        "/plan " .. argv[1],
        { { name = "plan", content = "Plan: $1" } }
      ),
    }
  end,
})