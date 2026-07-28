-- The shipped `read` tool: one bounded file read with line numbers.
--
-- Execution, bounds, path rules, and the rendered result are policy in this
-- module. The host contributes only `pi.effects.v1.fs.read`, and any package
-- may suppress or replace this single tool without touching the others.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.read",
  version = "1",
  dependencies = {
    paths = { name = "pi.tools.paths", version = "1" },
    render = { name = "pi.tools.render", version = "1" },
  },
  factory = function(deps)
    local effects = pi.effects.v1
    local DEFAULT_NAME = "read"
    local DEFAULT_MAX_BYTES = 512 * 1024

    local DESCRIPTION = "Read a UTF-8 text file. Returns 1-based numbered lines "
      .. "and reports truncation instead of silently dropping content."

    local PARAMETERS = {
      type = "object",
      properties = {
        path = { type = "string", description = "workspace file path" },
        offset = { type = "integer", description = "1-based first line to return" },
        limit = { type = "integer", description = "maximum number of lines to return" },
      },
      required = { "path" },
    }

    local function options_for(options)
      options = options or {}
      return {
        name = options.name or DEFAULT_NAME,
        resolver = options.resolver or deps.paths.resolver(options),
        max_bytes = tonumber(options.max_bytes) or DEFAULT_MAX_BYTES,
        max_lines = tonumber(options.max_lines) or deps.render.default_max_lines,
        max_output_bytes = tonumber(options.max_output_bytes)
          or deps.render.default_max_output_bytes,
      }
    end

    local function execute(call, options)
      local settings = options_for(options)
      local arguments = (call and call.arguments) or {}
      local path, reason = settings.resolver:resolve(arguments.path)
      if path == nil then
        return { output = "read failed: " .. reason, is_error = true, details = { ok = false } }
      end
      local shown = settings.resolver:display(path)
      local ok, contents = pcall(effects.fs.read, path, settings.max_bytes)
      if not ok then
        return {
          output = "read failed: " .. tostring(contents),
          is_error = true,
          details = { ok = false, path = shown },
        }
      end
      local window = deps.render.number_lines(contents, {
        offset = arguments.offset,
        limit = arguments.limit or settings.max_lines,
        max_bytes = settings.max_output_bytes,
      })
      return {
        output = window.text,
        is_error = false,
        details = {
          ok = true,
          path = shown,
          bytes = #contents,
          lines = window.total,
          shown = window.shown,
          first_line = window.first,
          last_line = window.last,
          truncated = window.truncated,
        },
      }
    end

    local function declare(registry, options)
      local settings = options_for(options)
      return registry.register({
        name = settings.name,
        description = DESCRIPTION,
        parameters = PARAMETERS,
        owner = "pi.builtins.tools",
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
    }
  end,
})
