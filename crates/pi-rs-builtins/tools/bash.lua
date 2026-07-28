-- The shipped `bash` tool: one bounded shell command with process-tree
-- cancellation.
--
-- The host process effect kills the command it spawned; anything that command
-- backgrounded is its own business. Keeping the whole tree bounded is tool
-- policy, so this module runs the command as a job in its own process group,
-- learns that group id from a marker on the command's own output, and kills
-- the group whenever the run ends killed (cancelled or timed out). No
-- privileged builtin executor and no host-side shell knowledge are involved.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.bash",
  version = "1",
  dependencies = {
    paths = { name = "pi.tools.paths", version = "1" },
    render = { name = "pi.tools.render", version = "1" },
    locks = { name = "pi.tools.locks", version = "1" },
  },
  factory = function(deps)
    local effects = pi.effects.v1
    local roots = pi.roots.v1
    local DEFAULT_NAME = "bash"
    local DEFAULT_SHELL = "bash"
    local DEFAULT_TIMEOUT_MS = 120000
    local DEFAULT_MAX_OUTPUT_BYTES = 64 * 1024
    local DEFAULT_PROCESS_OUTPUT_BYTES = 1024 * 1024
    local MARKER_PATTERN = "\1pi%-pgid:(%d+)\1"
    local MARKER_SCAN_BYTES = 512
    -- Shell commands mutate the workspace, so they share one serialization
    -- slot instead of a per-path lock: the command's paths are unknown.
    local MUTATION_LOCK = "pi.tools.bash/workspace"

    local DESCRIPTION = "Run one shell command in the workspace. Output is bounded, "
      .. "the command runs in its own process group, and cancelling kills the whole tree."

    local PARAMETERS = {
      type = "object",
      properties = {
        command = { type = "string", description = "shell command to run" },
        cwd = { type = "string", description = "optional workspace directory" },
        timeout_ms = { type = "integer", description = "optional timeout in milliseconds" },
      },
      required = { "command" },
    }

    local function options_for(options)
      options = options or {}
      return {
        name = options.name or DEFAULT_NAME,
        resolver = options.resolver or deps.paths.resolver(options),
        shell = options.shell or DEFAULT_SHELL,
        timeout_ms = tonumber(options.timeout_ms) or DEFAULT_TIMEOUT_MS,
        max_output_bytes = tonumber(options.max_output_bytes) or DEFAULT_MAX_OUTPUT_BYTES,
        -- Two separate bounds: the mechanism refuses a run that produces more
        -- than this, while the rendered result is clipped much earlier.
        process_max_output_bytes = tonumber(options.process_max_output_bytes)
          or DEFAULT_PROCESS_OUTPUT_BYTES,
        wait_ms = options.wait_ms,
        cancelled = options.cancelled,
        serialize = options.serialize ~= false,
      }
    end

    -- `set -m` gives the job its own process group; the marker publishes that
    -- group id to the caller before the command's own output matters.
    local function wrapper(command)
      return table.concat({
        "set -m",
        "{",
        command,
        "} &",
        "__pi_job=$!",
        "printf '\\1pi-pgid:%s\\1\\n' \"$__pi_job\"",
        'wait "$__pi_job"',
        "exit $?",
      }, "\n")
    end

    -- Default cancellation source: the kernel's dispatch cancellation handle.
    -- A caller may supply its own predicate (an agent interrupt, say).
    local function cancellation_probe(settings)
      if type(settings.cancelled) == "function" then
        return settings.cancelled
      end
      local ok, handle = pcall(roots.cancellation)
      if ok and handle ~= nil then
        return function()
          return handle:is_cancelled()
        end
      end
      return function()
        return false
      end
    end

    local function kill_group(settings, pgid)
      local script = "kill -TERM -"
        .. pgid
        .. " 2>/dev/null; sleep 0.05; kill -KILL -"
        .. pgid
        .. " 2>/dev/null; exit 0"
      pcall(effects.process.run, settings.shell, { "-c", script }, {
        timeout_ms = 5000,
        max_output_bytes = 1024,
      })
    end

    local function failure(message)
      return {
        output = "bash failed: " .. message,
        is_error = true,
        details = { ok = false },
      }
    end

    local function run(call, settings, options)
      local arguments = (call and call.arguments) or {}
      local command = arguments.command or arguments.input
      if type(command) ~= "string" or #command == 0 then
        return failure("command must be a non-empty string")
      end
      local cwd = nil
      if arguments.cwd ~= nil then
        local resolved, reason = settings.resolver:resolve(arguments.cwd)
        if resolved == nil then
          return failure(reason)
        end
        cwd = resolved
      end
      local timeout = tonumber(arguments.timeout_ms) or settings.timeout_ms

      local is_cancelled = cancellation_probe(settings)
      local signal = effects.cancellation.new()
      local pgid = nil
      local head = ""
      local cancelled = false

      local function observe(chunk)
        if pgid == nil and #head < MARKER_SCAN_BYTES then
          head = head .. chunk
          pgid = string.match(head, MARKER_PATTERN)
        end
        if not cancelled and is_cancelled() then
          cancelled = true
          signal:abort()
        end
      end

      if is_cancelled() then
        return {
          output = "bash cancelled before start",
          is_error = true,
          details = { ok = false, cancelled = true, killed = false },
        }
      end

      local ok, result = pcall(effects.process.run, settings.shell, { "-c", wrapper(command) }, {
        timeout_ms = timeout,
        max_output_bytes = settings.process_max_output_bytes,
        cwd = cwd,
        signal = signal,
        onData = observe,
      })

      if not ok then
        if pgid ~= nil then
          kill_group(settings, pgid)
        end
        local reason = tostring(result)
        if string.find(reason, "output exceeded", 1, true) then
          reason = "command produced more than "
            .. tostring(settings.process_max_output_bytes)
            .. " bytes of output"
        end
        return failure(reason)
      end

      local killed = result.killed == true
      if killed and pgid ~= nil then
        kill_group(settings, pgid)
      end

      local stdout = string.gsub(result.stdout or "", MARKER_PATTERN .. "\n?", "", 1)
      local stderr = result.stderr or ""
      local sections = {}
      if #stdout > 0 then
        sections[#sections + 1] = stdout
      end
      if #stderr > 0 then
        sections[#sections + 1] = "[stderr]\n" .. stderr
      end
      local code = tonumber(result.code) or 0
      if killed then
        sections[#sections + 1] = cancelled and "[cancelled after " .. tostring(#stdout) .. " bytes]"
          or "[killed: timed out after " .. tostring(timeout) .. "ms]"
      elseif code ~= 0 then
        sections[#sections + 1] = "[exit " .. tostring(code) .. "]"
      end
      if #sections == 0 then
        sections[1] = "[no output, exit 0]"
      end
      local bounded = deps.render.clip(table.concat(sections, "\n"), {
        max_bytes = settings.max_output_bytes,
      })
      return {
        output = bounded.text,
        is_error = killed or code ~= 0,
        details = {
          ok = not killed and code == 0,
          code = code,
          killed = killed,
          cancelled = cancelled,
          truncated = bounded.truncated,
          stdout_bytes = #stdout,
          stderr_bytes = #stderr,
          process_group = pgid,
          command = command,
        },
      }
    end

    local function execute(call, options)
      local settings = options_for(options)
      if not settings.serialize then
        return run(call, settings, options)
      end
      local result, busy = deps.locks.guard(MUTATION_LOCK, function()
        return run(call, settings, options)
      end, { wait_ms = settings.wait_ms })
      if result == nil then
        return {
          output = "bash failed: " .. tostring(busy),
          is_error = true,
          details = { ok = false, busy = true },
        }
      end
      return result
    end

    local function declare(registry, options)
      local settings = options_for(options)
      return registry.register({
        name = settings.name,
        description = DESCRIPTION,
        parameters = PARAMETERS,
        owner = "pi.builtins.tools",
        serialize = true,
        execute = function(call)
          return execute(call, options)
        end,
      })
    end

    local function unregister(registry, name)
      return registry.unregister(name or DEFAULT_NAME)
    end

    return {
      name = DEFAULT_NAME,
      description = DESCRIPTION,
      parameters = PARAMETERS,
      execute = execute,
      declare = declare,
      unregister = unregister,
      wrapper = wrapper,
    }
  end,
})
