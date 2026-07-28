-- Transcript rows for the shipped frontend.
--
-- The transcript is an ordinary bounded list of rows built from agent
-- actions. Rust never learns what a "user row" or a "tool row" is; a
-- replacement frontend may keep an entirely different shape.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.frontend.transcript",
  version = "1",
  factory = function()
    local DEFAULT_LIMITS = {
      max_rows = 200,
      max_row_bytes = 4096,
      max_tool_output = 120,
    }

    local function first_line(text)
      local line = tostring(text or ""):match("^[^\r\n]*") or ""
      return line
    end

    local Transcript = {}
    Transcript.__index = Transcript

    function Transcript.new(limits)
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
        entries = {},
        limits = bounds,
        revision = 0,
        streaming = nil,
        tools = {},
      }, Transcript)
    end

    function Transcript:touch()
      self.revision = self.revision + 1
    end

    -- Rows are bounded: the oldest row is dropped once the history limit is
    -- reached, so a long session cannot grow product state without bound.
    function Transcript:push(kind, text)
      local row = { kind = kind, text = first_line(text) }
      self.entries[#self.entries + 1] = row
      while #self.entries > self.limits.max_rows do
        table.remove(self.entries, 1)
      end
      self:touch()
      return row
    end

    function Transcript:user(text)
      self.streaming = nil
      self.tools = {}
      local prompt = tostring(text or "")
      local row = nil
      for line in (prompt .. "\n"):gmatch("([^\n]*)\n") do
        row = self:push("user", line)
      end
      return row
    end

    function Transcript:assistant_delta(delta)
      local text = tostring(delta or "")
      if #text == 0 then
        return nil
      end
      if self.streaming == nil then
        self.streaming = self:push("assistant", "")
      end
      local merged = self.streaming.text .. first_line(text)
      if #merged > self.limits.max_row_bytes then
        merged = merged:sub(1, self.limits.max_row_bytes)
      end
      self.streaming.text = merged
      self:touch()
      return self.streaming
    end

    function Transcript:assistant_done(text)
      local final = first_line(text)
      if self.streaming ~= nil then
        if #self.streaming.text == 0 and #final > 0 then
          self.streaming.text = final
        end
        self.streaming = nil
        self:touch()
        return
      end
      if #final > 0 then
        self:push("assistant", final)
      end
    end

    function Transcript:tool_start(id, name)
      self.streaming = nil
      local row = self:push("tool", "· " .. tostring(name or "tool"))
      self.tools[tostring(id)] = row
      return row
    end

    function Transcript:tool_result(id, name, ok, output)
      local row = self.tools[tostring(id)]
      local marker = ok and "+" or "!"
      local summary = first_line(output)
      if #summary > self.limits.max_tool_output then
        summary = summary:sub(1, self.limits.max_tool_output) .. "…"
      end
      local text = marker .. " " .. tostring(name or "tool") .. " → " .. summary
      if row == nil then
        return self:push("tool", text)
      end
      row.text = text
      self:touch()
      return row
    end

    function Transcript:notice(level, text)
      self.streaming = nil
      return self:push("notice", "[" .. tostring(level or "info") .. "] " .. tostring(text or ""))
    end

    function Transcript:rows()
      local copy = {}
      for index, row in ipairs(self.entries) do
        copy[index] = { kind = row.kind, text = row.text }
      end
      return copy
    end

    function Transcript:len()
      return #self.entries
    end

    function Transcript:clear()
      self.entries = {}
      self.streaming = nil
      self.tools = {}
      self:touch()
    end

    return {
      new = Transcript.new,
      defaults = DEFAULT_LIMITS,
    }
  end,
})
