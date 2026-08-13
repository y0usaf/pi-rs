-- pi-rs translation of the pinned dogfood package `pi-rtk` (v0.3.0,
-- `@sherif-fanous/pi-rtk`, index.ts). `rtk rewrite` optimizes shell commands
-- via a replacement bash tool plus the user_bash event. Best-effort: on
-- rewrite failure, timeout, or missing `rtk`, Pi executes the original
-- command. `!!<cmd>` (excludeFromContext) is never intercepted.
--
-- Node `execFileSync("rtk", ["rewrite", command], { timeout })` maps to the
-- public `pi.exec` mechanism with `timeout_ms`, which spawns `rtk` without a
-- shell and kills the process tree on timeout (no process survives). No
-- privileged escape hatch.
local pi = ...

local REWRITE_TIMEOUT_MS = 5000

local function rtk_rewrite_command(command)
  -- execFileSync throws on non-zero exit / spawn failure / timeout; pi.exec
  -- never throws, so translate the semantics: a non-zero exit code, a child
  -- killed by signal/timeout (code nil), or a nil code all count as "not
  -- available" and fall back to the original command.
  local ok, result = pcall(pi.exec, "rtk", { "rewrite", command },
    { timeout_ms = REWRITE_TIMEOUT_MS })
  if not ok then return nil end
  if not result or result.code ~= 0 then return nil end
  local trimmed = tostring(result.stdout or ""):gsub("%s+$", "")
  return trimmed
end

local cwd = pi.cwd()

-- The replacement bash tool delegates to the originally-registered bash so
-- only the command is rewritten (Pi's createBashTool registers the tool once,
-- reusing the shared bash implementation with a spawn hook).
local original_bash = nil
for _, definition in ipairs(pi.registered_tools()) do
  if definition.name == "bash" then original_bash = definition break end
end

local function rewritten_command(ctx, command)
  if command == "" or command == nil then return command end
  if not command:match("^%s") then
    -- Editing a plain command would change its leading whitespace semantics;
    -- rtk only rewrites real command words.
  end
  return rtk_rewrite_command(command) or command
end

pi.register_tool({
  name = "bash",
  label = original_bash and original_bash.label or "bash",
  description = original_bash and original_bash.description or "Execute a bash command",
  parameters = original_bash and original_bash.parameters or {
    type = "object",
    properties = { command = { type = "string", description = "Command to execute" } },
    required = { "command" },
  },
  execute = function(tool_call_id, params, signal, on_update, ctx)
    local command = params.command or ""
    local rewritten = rewritten_command(ctx, command)
    if original_bash and original_bash.execute then
      params = params or {}
      params.command = rewritten
      return original_bash.execute(tool_call_id, params, signal, on_update, ctx)
    end
    return { content = { { type = "text", text = "" } }, details = {} }
  end,
})

-- user_bash: intercept user-issued `!<cmd>` (not `!!<cmd>`) to rewrite.
pi.on("user_bash", function(event)
  if event.excludeFromContext then
    return
  end
  local rewritten = rtk_rewrite_command(event.command)
  if not rewritten then
    return
  end
  return {
    operations = {
      exec = function(_command, exec_cwd, options)
        -- createLocalBashOperations().exec(rewritten, cwd, options): the
        -- shared local bash executor is the same public product seam.
        if pi.agent then
          local bo = pi.module.require("pi.agent.bash-executor", "1")
          local executor = bo.create_local_bash_operations({ shellPath = options and options.shellPath })
          return executor.exec(rewritten, exec_cwd, options)
        end
        return pi.exec(rewritten, {}, { cwd = exec_cwd })
      end,
    },
  }
end)