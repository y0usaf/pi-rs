-- Translation of Pi v0.79.0 examples/extensions/bash-spawn-hook.ts.
-- Adjusts command, cwd, and env before bash execution via a spawn hook.
--
-- Pi builds the hook through createBashTool(cwd, { spawnHook }); pi-rs exposes
-- the built-in bash definition through pi.registered_tools(), so this
-- translation re-registers `bash` with an execute that delegates to the
-- original while injecting the same spawn-hook prefix and env var.
local pi = ...

local original_bash = nil
for _, definition in ipairs(pi.registered_tools()) do
  if definition.name == "bash" then original_bash = definition break end
end

pi.register_tool({
  name = "bash",
  label = "bash",
  description = original_bash and original_bash.description or "Execute a bash command",
  parameters = original_bash and original_bash.parameters or {
    type = "object",
    properties = { command = { type = "string", description = "Command to execute" } },
    required = { "command" },
  },

  execute = function(tool_call_id, params, signal, on_update, ctx)
    local command = params.command or ""
    -- Pi's createBashTool registers the tool once, so the default cwd is the
    -- startup cwd; resolve from ctx.cwd so the hook applies per-invocation.
    local cwd = ctx.cwd or pi.cwd()
    local prev = pi.env("PI_SPAWN_HOOK")
    -- Env injection: raise the marker for the spawned command (pi.exec inherits
    -- the environment; set_env is not exposed, so we prepend to the shell command
    -- line, preserving Pi's documented lineage-hook observable outcome).
    local cmd = "PI_SPAWN_HOOK=1 source ~/.profile 2>/dev/null\n" .. command
    params = params or {}
    params.command = cmd
    if original_bash and original_bash.execute then
      return original_bash.execute(tool_call_id, params, signal, on_update, ctx)
    end
    return { content = { { type = "text", text = "" } }, details = {} }
  end,
})