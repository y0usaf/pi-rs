-- Tool declaration registry for the shipped agent.
--
-- Tool packages (shipped or file-backed) require this module and declare
-- their tools through one path. The registry is ordinary Lua policy: it
-- carries no host privilege, and a replacement agent may define its own.
-- Settlement mode is declared here (`serialize = true` for tools that mutate
-- shared state) and honoured by `pi.agent.turn`.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.agent.tools",
  version = "1",
  factory = function()
    local order = {}
    local by_name = {}

    local function copy_schema(value)
      if type(value) ~= "table" then
        return value
      end
      local copy = {}
      for key, item in pairs(value) do
        copy[key] = copy_schema(item)
      end
      return copy
    end

    local function register(declaration)
      if type(declaration) ~= "table" then
        error("tool declaration must be a table", 0)
      end
      local name = declaration.name
      if type(name) ~= "string" or #name == 0 then
        error("tool name must be a non-empty string", 0)
      end
      if type(declaration.execute) ~= "function" then
        error("tool " .. name .. " requires an execute function", 0)
      end
      local existing = by_name[name]
      if existing then
        error("tool " .. name .. " is already declared by " .. tostring(existing.owner), 0)
      end
      local entry = {
        name = name,
        description = declaration.description or "",
        parameters = copy_schema(declaration.parameters)
          or { type = "object", properties = { input = { type = "string" } } },
        execute = declaration.execute,
        serialize = declaration.serialize == true,
        owner = declaration.owner or name,
      }
      by_name[name] = entry
      order[#order + 1] = entry
      return entry
    end

    local function unregister(name)
      if by_name[name] == nil then
        return false
      end
      by_name[name] = nil
      for index, entry in ipairs(order) do
        if entry.name == name then
          table.remove(order, index)
          break
        end
      end
      return true
    end

    local function find(name)
      if type(name) ~= "string" then
        return nil
      end
      return by_name[name]
    end

    local function list()
      local items = {}
      for index, entry in ipairs(order) do
        items[index] = entry
      end
      return items
    end

    -- Provider-facing declarations: exactly the wire fields, in declaration
    -- order, with the executor and settlement mode kept private to Lua.
    local function declarations()
      local items = {}
      for index, entry in ipairs(order) do
        items[index] = {
          name = entry.name,
          description = entry.description,
          parameters = copy_schema(entry.parameters),
        }
      end
      return items
    end

    return {
      register = register,
      unregister = unregister,
      find = find,
      list = list,
      declarations = declarations,
    }
  end,
})
