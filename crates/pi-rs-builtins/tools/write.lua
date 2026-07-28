-- The shipped `write` tool: replace one file's contents under the path lock.
--
-- Mutation policy lives here: the path lock, the create/overwrite decision,
-- the size bound, and the diff a transcript renders. The host contributes a
-- bounded `fs.write` and nothing else.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.write",
  version = "1",
  dependencies = {
    paths = { name = "pi.tools.paths", version = "1" },
    render = { name = "pi.tools.render", version = "1" },
    locks = { name = "pi.tools.locks", version = "1" },
  },
  factory = function(deps)
    local effects = pi.effects.v1
    local DEFAULT_NAME = "write"
    local DEFAULT_MAX_BYTES = 1024 * 1024

    local DESCRIPTION = "Write a UTF-8 text file, creating it when missing. "
      .. "Reports the resulting diff; concurrent writes to one path are serialized."

    local PARAMETERS = {
      type = "object",
      properties = {
        path = { type = "string", description = "workspace file path" },
        content = { type = "string", description = "complete new file contents" },
      },
      required = { "path", "content" },
    }

    local function options_for(options)
      options = options or {}
      return {
        name = options.name or DEFAULT_NAME,
        resolver = options.resolver or deps.paths.resolver(options),
        max_bytes = tonumber(options.max_bytes) or DEFAULT_MAX_BYTES,
        wait_ms = options.wait_ms,
        max_output_bytes = tonumber(options.max_output_bytes)
          or deps.render.default_max_output_bytes,
      }
    end

    -- The public surface has no `stat`, so prior contents (and therefore
    -- existence) come from one bounded read; an unreadable file is treated as
    -- existing with no diff rather than as a missing file.
    local function previous(path, max_bytes)
      local ok, contents = pcall(effects.fs.read, path, max_bytes)
      if ok then
        return true, contents
      end
      if string.find(tostring(contents), "exceeds", 1, true) then
        return true, nil
      end
      return false, nil
    end

    local function execute(call, options)
      local settings = options_for(options)
      local arguments = (call and call.arguments) or {}
      local path, reason = settings.resolver:resolve(arguments.path)
      if path == nil then
        return { output = "write failed: " .. reason, is_error = true, details = { ok = false } }
      end
      local content = arguments.content
      if type(content) ~= "string" then
        return {
          output = "write failed: content must be a string",
          is_error = true,
          details = { ok = false, path = settings.resolver:display(path) },
        }
      end
      local shown = settings.resolver:display(path)
      if #content > settings.max_bytes then
        return {
          output = "write failed: "
            .. tostring(#content)
            .. " bytes exceeds the "
            .. tostring(settings.max_bytes)
            .. " byte limit",
          is_error = true,
          details = { ok = false, path = shown, bytes = #content },
        }
      end

      local result, busy = deps.locks.guard(path, function()
        local existed, before = previous(path, settings.max_bytes)
        local ok, failure = pcall(effects.fs.write, path, content)
        if not ok then
          return {
            output = "write failed: " .. tostring(failure),
            is_error = true,
            details = { ok = false, path = shown },
          }
        end
        deps.locks.bump(path)
        local changes = deps.render.diff(before or "", content, options)
        local summary = (existed and "updated " or "created ") .. shown
        local body = summary
          .. " ("
          .. tostring(#content)
          .. " bytes, +"
          .. tostring(changes.added)
          .. "/-"
          .. tostring(changes.removed)
          .. ")"
        if #changes.text > 0 then
          body = body .. "\n" .. changes.text
        end
        return {
          output = deps.render.clip(body, { max_bytes = settings.max_output_bytes }).text,
          is_error = false,
          details = {
            ok = true,
            path = shown,
            created = not existed,
            bytes = #content,
            added = changes.added,
            removed = changes.removed,
            revision = deps.locks.revision(path),
            diff = changes.rows,
          },
        }
      end, { wait_ms = settings.wait_ms })

      if result == nil then
        return {
          output = "write failed: " .. tostring(busy),
          is_error = true,
          details = { ok = false, path = shown, busy = true },
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
    }
  end,
})
