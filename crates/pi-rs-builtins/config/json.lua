-- Strict bounded JSON reader for the legacy configuration fallback.
--
-- The host stores and streams JSON, but it never decodes a *product* file for
-- Lua: the legacy `settings.json` resource is a product format, so its reader
-- is ordinary Lua policy. The decoder is deliberately strict — a trailing
-- comma, a control character, or a duplicate key is an error naming its byte
-- offset, because a configuration file that silently loses a key is worse
-- than one that refuses to load.
--
-- `null` decodes to the `json.null` sentinel rather than `nil`, so a present
-- key stays distinguishable from an absent one; the settings mapper drops it.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.config.json",
  version = "1",
  factory = function()
    local NULL = setmetatable({}, { __tostring = function() return "json.null" end })
    local MAX_BYTES = 1024 * 1024
    local MAX_DEPTH = 16

    local decode_value

    local function fail(text, position, message)
      local prefix = string.sub(text, 1, math.max(position - 1, 0))
      local _, newlines = string.gsub(prefix, "\n", "")
      error(string.format("%s at byte %d (line %d)", message, position, newlines + 1), 0)
    end

    local function skip_space(text, position)
      local _, stop = string.find(text, "^[ \t\r\n]*", position)
      return (stop or position - 1) + 1
    end

    local function decode_string(text, position)
      -- position points at the opening quote.
      local index = position + 1
      local parts = {}
      while true do
        local char = string.sub(text, index, index)
        if char == "" then
          fail(text, index, "unterminated string")
        elseif char == '"' then
          return table.concat(parts), index + 1
        elseif char == "\\" then
          local escape = string.sub(text, index + 1, index + 1)
          if escape == "n" then
            parts[#parts + 1] = "\n"
          elseif escape == "t" then
            parts[#parts + 1] = "\t"
          elseif escape == "r" then
            parts[#parts + 1] = "\r"
          elseif escape == "b" then
            parts[#parts + 1] = "\b"
          elseif escape == "f" then
            parts[#parts + 1] = "\f"
          elseif escape == '"' or escape == "\\" or escape == "/" then
            parts[#parts + 1] = escape
          elseif escape == "u" then
            local hex = string.sub(text, index + 2, index + 5)
            local code = tonumber(hex, 16)
            if code == nil or #hex < 4 then
              fail(text, index, "invalid \\u escape")
            end
            parts[#parts + 1] = utf8.char(code)
            index = index + 4
          else
            fail(text, index, "invalid escape")
          end
          index = index + 2
        elseif string.byte(char) < 32 then
          fail(text, index, "raw control character in string")
        else
          parts[#parts + 1] = char
          index = index + 1
        end
      end
    end

    local function decode_number(text, position)
      local literal = string.match(text, "^%-?%d+%.?%d*[eE]?[-+]?%d*", position)
      if literal == nil or literal == "" then
        fail(text, position, "invalid number")
      end
      local value = tonumber(literal)
      if value == nil then
        fail(text, position, "invalid number")
      end
      return value, position + #literal
    end

    local function decode_array(text, position, depth)
      local items = {}
      local index = skip_space(text, position + 1)
      if string.sub(text, index, index) == "]" then
        return items, index + 1
      end
      while true do
        local value
        value, index = decode_value(text, index, depth + 1)
        items[#items + 1] = value
        index = skip_space(text, index)
        local char = string.sub(text, index, index)
        if char == "," then
          index = skip_space(text, index + 1)
          if string.sub(text, index, index) == "]" then
            fail(text, index, "trailing comma")
          end
        elseif char == "]" then
          return items, index + 1
        else
          fail(text, index, "expected ',' or ']'")
        end
      end
    end

    local function decode_object(text, position, depth)
      local object = {}
      local index = skip_space(text, position + 1)
      if string.sub(text, index, index) == "}" then
        return object, index + 1
      end
      while true do
        if string.sub(text, index, index) ~= '"' then
          fail(text, index, "expected object key")
        end
        local key
        key, index = decode_string(text, index)
        if object[key] ~= nil then
          fail(text, index, "duplicate key '" .. key .. "'")
        end
        index = skip_space(text, index)
        if string.sub(text, index, index) ~= ":" then
          fail(text, index, "expected ':'")
        end
        index = skip_space(text, index + 1)
        local value
        value, index = decode_value(text, index, depth + 1)
        object[key] = value
        index = skip_space(text, index)
        local char = string.sub(text, index, index)
        if char == "," then
          index = skip_space(text, index + 1)
          if string.sub(text, index, index) == "}" then
            fail(text, index, "trailing comma")
          end
        elseif char == "}" then
          return object, index + 1
        else
          fail(text, index, "expected ',' or '}'")
        end
      end
    end

    decode_value = function(text, position, depth)
      if depth > MAX_DEPTH then
        fail(text, position, "nesting exceeds depth " .. MAX_DEPTH)
      end
      local index = skip_space(text, position)
      local char = string.sub(text, index, index)
      if char == "" then
        fail(text, index, "unexpected end of input")
      elseif char == "{" then
        return decode_object(text, index, depth)
      elseif char == "[" then
        return decode_array(text, index, depth)
      elseif char == '"' then
        return decode_string(text, index)
      elseif string.sub(text, index, index + 3) == "true" then
        return true, index + 4
      elseif string.sub(text, index, index + 4) == "false" then
        return false, index + 5
      elseif string.sub(text, index, index + 3) == "null" then
        return NULL, index + 4
      else
        return decode_number(text, index)
      end
    end

    --- Decode one complete JSON document. Trailing content is an error.
    local function decode(text)
      if type(text) ~= "string" then
        error("json.decode expects a string", 0)
      end
      if #text > MAX_BYTES then
        error(string.format("document exceeds %d bytes", MAX_BYTES), 0)
      end
      local value, index = decode_value(text, 1, 1)
      index = skip_space(text, index)
      if index <= #text then
        fail(text, index, "trailing content after value")
      end
      return value
    end

    return {
      null = NULL,
      max_bytes = MAX_BYTES,
      max_depth = MAX_DEPTH,
      decode = decode,
    }
  end,
})
