-- Multiline prompt editor for the shipped frontend.
--
-- The buffer, the cursor, and every editing rule are Lua policy: Rust owns
-- byte decoding and cell measurement, not what a keystroke means. Bounds are
-- declared here so a stuck paste cannot grow unbounded product state.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.frontend.editor",
  version = "1",
  factory = function()
    local DEFAULT_LIMITS = {
      max_lines = 64,
      max_line_bytes = 4096,
    }

    local function char_count(text)
      return utf8.len(text) or #text
    end

    -- 1-based character index -> byte offset, clamped to the line end.
    local function byte_offset(text, column)
      if column <= 1 then
        return 1
      end
      local offset = utf8.offset(text, column)
      if offset == nil then
        return #text + 1
      end
      return offset
    end

    -- Control bytes never enter the buffer: they are keys, not content.
    local function sanitize(text)
      return (text:gsub("[%z\1-\8\11\12\14-\31\127]", ""))
    end

    local Editor = {}
    Editor.__index = Editor

    function Editor.new(limits)
      local bounds = {}
      for key, value in pairs(DEFAULT_LIMITS) do
        bounds[key] = value
      end
      for key, value in pairs(limits or {}) do
        if DEFAULT_LIMITS[key] ~= nil and tonumber(value) then
          bounds[key] = tonumber(value)
        end
      end
      return setmetatable({
        rows = { "" },
        row = 1,
        column = 1,
        limits = bounds,
      }, Editor)
    end

    function Editor:line()
      return self.rows[self.row] or ""
    end

    function Editor:insert(text)
      if type(text) ~= "string" then
        return false
      end
      local clean = sanitize(text)
      if #clean == 0 then
        return false
      end
      local line = self:line()
      if #line + #clean > self.limits.max_line_bytes then
        return false
      end
      local at = byte_offset(line, self.column)
      self.rows[self.row] = line:sub(1, at - 1) .. clean .. line:sub(at)
      self.column = self.column + char_count(clean)
      return true
    end

    function Editor:newline()
      if #self.rows >= self.limits.max_lines then
        return false
      end
      local line = self:line()
      local at = byte_offset(line, self.column)
      self.rows[self.row] = line:sub(1, at - 1)
      table.insert(self.rows, self.row + 1, line:sub(at))
      self.row = self.row + 1
      self.column = 1
      return true
    end

    function Editor:backspace()
      if self.column > 1 then
        local line = self:line()
        local from = byte_offset(line, self.column - 1)
        local to = byte_offset(line, self.column)
        self.rows[self.row] = line:sub(1, from - 1) .. line:sub(to)
        self.column = self.column - 1
        return true
      end
      if self.row == 1 then
        return false
      end
      local removed = table.remove(self.rows, self.row)
      self.row = self.row - 1
      local previous = self:line()
      self.column = char_count(previous) + 1
      self.rows[self.row] = previous .. removed
      return true
    end

    function Editor:delete()
      local line = self:line()
      if self.column <= char_count(line) then
        local from = byte_offset(line, self.column)
        local to = byte_offset(line, self.column + 1)
        self.rows[self.row] = line:sub(1, from - 1) .. line:sub(to)
        return true
      end
      if self.row >= #self.rows then
        return false
      end
      local following = table.remove(self.rows, self.row + 1)
      self.rows[self.row] = line .. following
      return true
    end

    function Editor:move(direction)
      if direction == "left" then
        if self.column > 1 then
          self.column = self.column - 1
        elseif self.row > 1 then
          self.row = self.row - 1
          self.column = char_count(self:line()) + 1
        end
      elseif direction == "right" then
        if self.column <= char_count(self:line()) then
          self.column = self.column + 1
        elseif self.row < #self.rows then
          self.row = self.row + 1
          self.column = 1
        end
      elseif direction == "up" then
        if self.row > 1 then
          self.row = self.row - 1
        end
      elseif direction == "down" then
        if self.row < #self.rows then
          self.row = self.row + 1
        end
      elseif direction == "home" then
        self.column = 1
      elseif direction == "end" then
        self.column = char_count(self:line()) + 1
      else
        return false
      end
      local width = char_count(self:line()) + 1
      if self.column > width then
        self.column = width
      end
      return true
    end

    function Editor:clear_line()
      self.rows[self.row] = ""
      self.column = 1
      return true
    end

    function Editor:clear()
      self.rows = { "" }
      self.row = 1
      self.column = 1
    end

    function Editor:text()
      return table.concat(self.rows, "\n")
    end

    function Editor:is_empty()
      return self:text():match("^%s*$") ~= nil
    end

    function Editor:lines()
      local copy = {}
      for index, line in ipairs(self.rows) do
        copy[index] = line
      end
      return copy
    end

    function Editor:cursor()
      return { row = self.row, column = self.column }
    end

    return {
      new = Editor.new,
      defaults = DEFAULT_LIMITS,
    }
  end,
})
