-- Path policy for the shipped core tools. The kernel exposes bounded
-- filesystem effects and nothing else: which paths a tool may touch, how a
-- relative path is resolved, and what a rejection says are ordinary Lua
-- policy that any replacement tool package may ignore or redefine.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.tools.paths",
  version = "1",
  factory = function()
    local function segments(path)
      local parts = {}
      for segment in string.gmatch(path, "[^/]+") do
        parts[#parts + 1] = segment
      end
      return parts
    end

    -- Lexical normalization only: the public effect surface exposes no
    -- realpath, so `..` is resolved textually and an escape above the root is
    -- a rejection rather than a resolved path outside the workspace.
    local function normalize(path)
      local absolute = string.sub(path, 1, 1) == "/"
      local stack = {}
      for _, segment in ipairs(segments(path)) do
        if segment == ".." then
          if #stack == 0 then
            return nil, "path escapes the workspace root"
          end
          table.remove(stack)
        elseif segment ~= "." then
          stack[#stack + 1] = segment
        end
      end
      local joined = table.concat(stack, "/")
      if absolute then
        return "/" .. joined
      end
      return joined
    end

    local Resolver = {}
    Resolver.__index = Resolver

    -- `root` is optional: without it, relative paths resolve against the host
    -- working directory and absolute paths are refused unless the owner opts
    -- in. With it, every accepted path stays inside that subtree.
    function Resolver.new(options)
      options = options or {}
      local root = nil
      if type(options.root) == "string" and #options.root > 0 then
        root = normalize(options.root)
      end
      return setmetatable({
        root = root,
        allow_absolute = options.allow_absolute == true,
      }, Resolver)
    end

    function Resolver:resolve(path)
      if type(path) ~= "string" then
        return nil, "path must be a string"
      end
      if #path == 0 then
        return nil, "path must not be empty"
      end
      if string.find(path, "\0", 1, true) then
        return nil, "path must not contain a NUL byte"
      end
      local absolute = string.sub(path, 1, 1) == "/"
      if absolute and not (self.allow_absolute or self.root ~= nil) then
        return nil, "absolute paths are not allowed: " .. path
      end
      local candidate = path
      if not absolute and self.root ~= nil then
        candidate = self.root .. "/" .. path
      end
      local normalized, reason = normalize(candidate)
      if normalized == nil then
        return nil, reason .. ": " .. path
      end
      if #normalized == 0 or normalized == "/" then
        return nil, "path must name a file: " .. path
      end
      if self.root ~= nil then
        local prefix = self.root .. "/"
        if normalized ~= self.root and string.sub(normalized, 1, #prefix) ~= prefix then
          return nil, "path escapes the workspace root: " .. path
        end
      end
      return normalized
    end

    -- Render-facing form: results quote the shortest path a reader can act
    -- on, never a temporary absolute prefix.
    function Resolver:display(path)
      if self.root == nil then
        return path
      end
      local prefix = self.root .. "/"
      if string.sub(path, 1, #prefix) == prefix then
        return string.sub(path, #prefix + 1)
      end
      return path
    end

    return {
      resolver = Resolver.new,
      normalize = normalize,
    }
  end,
})
