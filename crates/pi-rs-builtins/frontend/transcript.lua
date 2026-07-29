-- Transcript blocks for the shipped frontend.
--
-- The transcript is an ordinary bounded list of entries built from agent
-- actions, plus the presentation policy that turns them into styled display
-- lines. Rust never learns what a "user block" or a "tool block" is: it
-- receives text runs with cell styles like any other package would submit.
--
-- Presentation rhythm (canonical set, `tests/experience/canonical-v1.json`):
-- every entry becomes a block of full-width lines, blocks are separated by one
-- untouched row, text starts at column 1, and a filled block paints its own
-- background across the whole width with one padded row above and below.

local pi = ...
local module = pi.kernel.v1.module
local text_cells = pi.terminal.v1.text

module.define({
  name = "pi.frontend.transcript",
  version = "1",
  factory = function()
    local DEFAULT_LIMITS = {
      max_entries = 200,
      max_entry_bytes = 4096,
      max_argument = 120,
      max_output = 120,
      max_block_rows = 64,
    }

    -- Product palette. These are the reviewed canonical colors for the
    -- transcript; a replacement frontend may choose any others.
    local function rgb(value)
      return {
        red = (value // 0x10000) % 0x100,
        green = (value // 0x100) % 0x100,
        blue = value % 0x100,
      }
    end

    local TEXT = rgb(0xd4d4d4)
    local ARGUMENT = rgb(0x8abeb7)
    local MUTED = rgb(0x808080)
    local META = rgb(0x666666)
    local FAILURE = rgb(0xcc6666)

    local USER_FILL = rgb(0x343541)
    local TOOL_PENDING_FILL = rgb(0x282832)
    local TOOL_OK_FILL = rgb(0x283228)
    local TOOL_FAILED_FILL = rgb(0x3c2828)

    -- One presentation row per entry kind: `fill` paints a background block
    -- with pad rows, `text`/`accent` style the two run kinds inside it.
    local PRESENTATION = {
      user = { fill = USER_FILL, text = TEXT },
      assistant = { text = nil },
      thinking = { text = MUTED, italic = true },
      tool_pending = { fill = TOOL_PENDING_FILL, text = TEXT, bold = true, accent = ARGUMENT },
      tool_ok = { fill = TOOL_OK_FILL, text = TEXT, bold = true, accent = ARGUMENT },
      tool_failed = { fill = TOOL_FAILED_FILL, text = TEXT, bold = true, accent = MUTED },
      notice_error = { text = FAILURE },
      notice_warn = { text = META },
      notice_info = { text = META },
    }

    local SEPARATOR = { runs = {} }

    local function styled(presentation, kind)
      local style = {}
      if presentation.fill then
        style.background = presentation.fill
      end
      if kind == "accent" then
        style.foreground = presentation.accent or presentation.text
      elseif kind == "text" then
        style.foreground = presentation.text
        style.bold = presentation.bold == true
        style.italic = presentation.italic == true
      end
      return style
    end

    local function fill_style(presentation)
      if not presentation.fill then
        return nil
      end
      return { background = presentation.fill }
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

    local function clip(text, budget)
      local value = tostring(text or "")
      if #value > budget then
        return value:sub(1, budget)
      end
      return value
    end

    -- Entries are bounded: the oldest is dropped once the history limit is
    -- reached, so a long session cannot grow product state without bound.
    function Transcript:push(entry)
      self.entries[#self.entries + 1] = entry
      while #self.entries > self.limits.max_entries do
        table.remove(self.entries, 1)
      end
      self:touch()
      return entry
    end

    function Transcript:user(text)
      self.streaming = nil
      self.tools = {}
      return self:push({
        kind = "user",
        text = clip(text, self.limits.max_entry_bytes),
      })
    end

    function Transcript:assistant_delta(delta)
      local chunk = tostring(delta or "")
      if #chunk == 0 then
        return nil
      end
      if self.streaming == nil then
        self.streaming = self:push({ kind = "assistant", text = "" })
      end
      self.streaming.text = clip(self.streaming.text .. chunk, self.limits.max_entry_bytes)
      self:touch()
      return self.streaming
    end

    function Transcript:assistant_done(text)
      local final = clip(text, self.limits.max_entry_bytes)
      if self.streaming ~= nil then
        if #self.streaming.text == 0 and #final > 0 then
          self.streaming.text = final
        end
        self.streaming = nil
        self:touch()
        return
      end
      if #final > 0 then
        self:push({ kind = "assistant", text = final })
      end
    end

    function Transcript:thinking(text)
      self.streaming = nil
      local value = clip(text, self.limits.max_entry_bytes)
      if #value == 0 then
        return nil
      end
      return self:push({ kind = "thinking", text = value })
    end

    -- A call is summarised as its name plus its scalar arguments in key order,
    -- so the block reads like the command that was run and stays deterministic
    -- whatever order the provider serialised the arguments in.
    function Transcript:argument_summary(arguments)
      if type(arguments) ~= "table" then
        return ""
      end
      local keys = {}
      for key, value in pairs(arguments) do
        local kind = type(value)
        if kind == "string" or kind == "number" or kind == "boolean" then
          keys[#keys + 1] = tostring(key)
        end
      end
      table.sort(keys)
      local parts = {}
      for _, key in ipairs(keys) do
        parts[#parts + 1] = tostring(arguments[key])
      end
      return clip(table.concat(parts, " "), self.limits.max_argument)
    end

    function Transcript:tool_start(id, name, arguments)
      self.streaming = nil
      local entry = self:push({
        kind = "tool",
        state = "pending",
        name = tostring(name or "tool"),
        argument = self:argument_summary(arguments),
      })
      self.tools[tostring(id)] = entry
      return entry
    end

    function Transcript:tool_result(id, name, ok, output)
      local entry = self.tools[tostring(id)]
      local summary = clip(output, self.limits.max_output)
      if entry == nil then
        entry = self:push({
          kind = "tool",
          state = "pending",
          name = tostring(name or "tool"),
          argument = "",
        })
      end
      entry.state = ok and "ok" or "failed"
      -- A settled call collapses to what was run; only a failure keeps its
      -- output, because that is the part the user has to act on.
      entry.output = (not ok) and summary or nil
      self:touch()
      return entry
    end

    function Transcript:notice(level, text)
      self.streaming = nil
      local name = tostring(level or "info")
      if name ~= "error" and name ~= "warn" then
        name = "info"
      end
      return self:push({
        kind = "notice",
        level = name,
        text = clip(text, self.limits.max_entry_bytes),
      })
    end

    local function presentation_for(entry)
      if entry.kind == "tool" then
        if entry.state == "ok" then
          return PRESENTATION.tool_ok
        end
        if entry.state == "failed" then
          return PRESENTATION.tool_failed
        end
        return PRESENTATION.tool_pending
      end
      if entry.kind == "notice" then
        return PRESENTATION["notice_" .. entry.level] or PRESENTATION.notice_info
      end
      return PRESENTATION[entry.kind] or PRESENTATION.assistant
    end

    -- One display line: the leading indent and the trailing pad carry the
    -- block fill but no foreground, which is exactly how the canonical frames
    -- record a filled row.
    local function line_of(presentation, width, runs)
      local fill = fill_style(presentation)
      local out = { { text = " ", style = fill } }
      local used = 1
      for _, run in ipairs(runs) do
        if #run.text > 0 then
          out[#out + 1] = { text = run.text, style = run.style }
          used = used + text_cells.width(run.text)
        end
      end
      if fill and used < width then
        out[#out + 1] = { text = string.rep(" ", width - used), style = fill }
      end
      return { runs = out }
    end

    function Transcript:block(entry, width)
      local presentation = presentation_for(entry)
      local body = math.max(1, width - 1)
      local lines = {}

      if presentation.fill then
        lines[#lines + 1] = {
          runs = { { text = string.rep(" ", width), style = fill_style(presentation) } },
        }
      end

      if entry.kind == "tool" then
        local name, name_width = text_cells.truncate(entry.name, { width = body })
        local runs = { { text = name, style = styled(presentation, "text") } }
        local argument = entry.argument or ""
        if #argument > 0 and name_width + 1 < body then
          local shown = text_cells.truncate(argument, { width = body - name_width - 1 })
          runs[#runs + 1] = { text = " ", style = fill_style(presentation) }
          runs[#runs + 1] = { text = shown, style = styled(presentation, "accent") }
        end
        lines[#lines + 1] = line_of(presentation, width, runs)
        if entry.output and #entry.output > 0 then
          local rows = text_cells.wrap(entry.output, { width = body, limit = 4 })
          for _, row in ipairs(rows) do
            lines[#lines + 1] = line_of(presentation, width, {
              { text = row, style = styled(presentation, "accent") },
            })
          end
        end
      else
        local rows = text_cells.wrap(entry.text, {
          width = body,
          limit = self.limits.max_block_rows,
        })
        if #rows == 0 then
          rows = { "" }
        end
        for _, row in ipairs(rows) do
          lines[#lines + 1] = line_of(presentation, width, {
            { text = row, style = styled(presentation, "text") },
          })
        end
      end

      if presentation.fill then
        lines[#lines + 1] = {
          runs = { { text = string.rep(" ", width), style = fill_style(presentation) } },
        }
      end
      return lines
    end

    -- Only the newest `limit` lines are ever built, so presentation cost is
    -- bounded by the viewport rather than by history length.
    function Transcript:lines(width, limit)
      local columns = math.max(1, math.floor(tonumber(width) or 80))
      local budget = math.max(1, math.floor(tonumber(limit) or self.limits.max_block_rows))
      local collected = {}
      local total = 0
      for index = #self.entries, 1, -1 do
        if total >= budget then
          break
        end
        local block = self:block(self.entries[index], columns)
        if #collected > 0 then
          total = total + 1
        end
        total = total + #block
        collected[#collected + 1] = block
      end

      local lines = {}
      for index = #collected, 1, -1 do
        if #lines > 0 then
          lines[#lines + 1] = SEPARATOR
        end
        for _, line in ipairs(collected[index]) do
          lines[#lines + 1] = line
        end
      end
      while #lines > budget do
        table.remove(lines, 1)
      end
      return lines
    end

    function Transcript:rows()
      local copy = {}
      for index, entry in ipairs(self.entries) do
        copy[index] = {
          kind = entry.kind,
          text = entry.text,
          name = entry.name,
          state = entry.state,
          level = entry.level,
        }
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
      separator = SEPARATOR,
    }
  end,
})
