-- The shipped `edit` tool: one exact-match replacement inside a file.
--
-- Match policy (unique by default), the optional revision guard, the path
-- lock, and the rendered diff are all Lua. The host contributes bounded
-- `fs.read`/`fs.write` and never inspects the edit.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.edit",
  version = "1",
  dependencies = {
    paths = { name = "pi.tools.paths", version = "1" },
    render = { name = "pi.tools.render", version = "1" },
    locks = { name = "pi.tools.locks", version = "1" },
  },
  factory = function(deps)
    local effects = pi.effects.v1
    local DEFAULT_NAME = "edit"
    local DEFAULT_MAX_BYTES = 1024 * 1024

    local DESCRIPTION = "Replace an exact text span in a file. The span must be "
      .. "unique unless replace_all is set; edits to one path are serialized."

    local PARAMETERS = {
      type = "object",
      properties = {
        path = { type = "string", description = "workspace file path" },
        old_text = { type = "string", description = "exact text to replace" },
        new_text = { type = "string", description = "replacement text" },
        replace_all = { type = "boolean", description = "replace every occurrence" },
        expected_revision = {
          type = "integer",
          description = "revision returned by an earlier write/edit of this path",
        },
      },
      required = { "path", "old_text", "new_text" },
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

    -- Plain (non-pattern) scan: tool arguments are literal text, never Lua
    -- patterns, so a `%` or `-` in source code cannot change the match.
    local function occurrences(haystack, needle)
      local found = {}
      local position = 1
      while true do
        local start, stop = string.find(haystack, needle, position, true)
        if start == nil then
          return found
        end
        found[#found + 1] = { start = start, stop = stop }
        position = stop + 1
      end
    end

    local function apply(contents, spans, replacement)
      local pieces = {}
      local cursor = 1
      for _, span in ipairs(spans) do
        pieces[#pieces + 1] = string.sub(contents, cursor, span.start - 1)
        pieces[#pieces + 1] = replacement
        cursor = span.stop + 1
      end
      pieces[#pieces + 1] = string.sub(contents, cursor)
      return table.concat(pieces)
    end

    local function failure(message, path)
      return {
        output = "edit failed: " .. message,
        is_error = true,
        details = { ok = false, path = path },
      }
    end

    local function execute(call, options)
      local settings = options_for(options)
      local arguments = (call and call.arguments) or {}
      local path, reason = settings.resolver:resolve(arguments.path)
      if path == nil then
        return failure(reason, nil)
      end
      local shown = settings.resolver:display(path)
      local old_text = arguments.old_text
      local new_text = arguments.new_text
      if type(old_text) ~= "string" or #old_text == 0 then
        return failure("old_text must be a non-empty string", shown)
      end
      if type(new_text) ~= "string" then
        return failure("new_text must be a string", shown)
      end

      local result, busy = deps.locks.guard(path, function()
        local expected = tonumber(arguments.expected_revision)
        if expected ~= nil and expected ~= deps.locks.revision(path) then
          return failure(
            "stale revision "
              .. tostring(expected)
              .. "; "
              .. shown
              .. " is at revision "
              .. tostring(deps.locks.revision(path)),
            shown
          )
        end
        local ok, contents = pcall(effects.fs.read, path, settings.max_bytes)
        if not ok then
          return failure(tostring(contents), shown)
        end
        local spans = occurrences(contents, old_text)
        if #spans == 0 then
          return failure("old_text was not found in " .. shown, shown)
        end
        if #spans > 1 and arguments.replace_all ~= true then
          return failure(
            tostring(#spans) .. " matches in " .. shown .. "; pass replace_all or add context",
            shown
          )
        end
        if arguments.replace_all ~= true then
          spans = { spans[1] }
        end
        local updated = apply(contents, spans, new_text)
        if #updated > settings.max_bytes then
          return failure(
            "result of " .. tostring(#updated) .. " bytes exceeds the write limit",
            shown
          )
        end
        local written, write_error = pcall(effects.fs.write, path, updated)
        if not written then
          return failure(tostring(write_error), shown)
        end
        deps.locks.bump(path)
        local changes = deps.render.diff(contents, updated, options)
        local body = "edited "
          .. shown
          .. " ("
          .. tostring(#spans)
          .. " replacement"
          .. (#spans == 1 and "" or "s")
          .. ", +"
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
            replacements = #spans,
            added = changes.added,
            removed = changes.removed,
            bytes = #updated,
            revision = deps.locks.revision(path),
            diff = changes.rows,
          },
        }
      end, { wait_ms = settings.wait_ms })

      if result == nil then
        local rejected = failure(tostring(busy), shown)
        rejected.details.busy = true
        return rejected
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
