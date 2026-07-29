-- Transcript blocks for the shipped frontend.
--
-- Two halves live in this file and they are deliberately separable:
--
--   1. the bounded entry list built from agent actions (`user`,
--      `assistant_delta`, `tool_start`, `notice`, ...), which is state, and
--   2. the presentation that turns one entry into styled display lines, which
--      is a **declaration**, not a private branch.
--
-- Rust never learns what a "user block" or a "tool block" is: it receives text
-- runs with cell styles like any other package would submit.
--
-- Presentation rhythm (canonical set, `tests/experience/canonical-v1.json`):
-- every entry becomes a block of full-width lines, blocks are separated by one
-- untouched row, text starts at column 1, and a filled block paints its own
-- background across the whole width with one padded row above and below.
--
-- **The block renderer seam.** Each shipped block is declared through the one
-- generic declaration path, `pi.kernel.v1.declare("renderer", definition)`,
-- with `surface = "transcript.block"`. A declaration claims one `entry` kind:
--
--     pi.kernel.v1.declare("renderer", {
--       id      = "my.package.user-block",
--       surface = "transcript.block",
--       entry   = "user",                 -- user|assistant|thinking|tool|notice
--       order   = 10,                     -- shipped blocks declare 0
--       render  = function(entry, context) return { line, line, ... } end,
--     })
--
-- `render` receives the entry and a per-frame `context`:
--
--   | field           | meaning                                            |
--   |-----------------|----------------------------------------------------|
--   | `width`         | block width in cells                               |
--   | `body`          | usable text width (`width - 1`; column 0 indents)  |
--   | `limits`        | a copy of this transcript's bounds                 |
--   | `options`       | a copy of this frame's presentation options        |
--   | `line(runs[, fill])` | one full-width display line                   |
--   | `padded(fill)`  | one full-width row of nothing but `fill`           |
--
-- and returns display lines — `{ runs = { { text = ..., style = ... } } }` —
-- exactly the shape `pi.frontend.view` already consumes.
--
-- Resolution is one bounded host read per frame, not per block: the winner for
-- an entry kind is the **last** matching declaration in registered order, which
-- is `order`, then source, then id. Declaring a positive `order` therefore
-- replaces a shipped block deterministically, without a priority auction and
-- without forking the frontend root. Disposing the declaring package retracts
-- it and the shipped block returns.
--
-- A renderer is ordinary policy, so its output is bounded here: a block is
-- clipped to `max_block_rows + 2` lines and a malformed line is dropped. An
-- entry kind no renderer claims still shows its text unstyled, so removing
-- presentation never silently removes content.

local pi = ...
local kernel = pi.kernel.v1
local module = kernel.module
local text_cells = pi.terminal.v1.text

local SURFACE = "transcript.block"

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

    -- Frame-scoped presentation options. Every renderer reads them through
    -- `context.options`, so a block-specific toggle such as collapsed
    -- thinking needs no block-specific host surface and no private branch.
    -- Only scalars are stored and the table is capped, so the per-frame copy
    -- stays bounded.
    local DEFAULT_OPTIONS = {
      thinking_visible = true,
    }
    local MAX_OPTIONS = 32

    -- Product palette. These are the reviewed canonical colors for the
    -- transcript; a replacement renderer may choose any others.
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

    local PALETTE = {
      text = TEXT,
      argument = ARGUMENT,
      muted = MUTED,
      meta = META,
      failure = FAILURE,
      user_fill = USER_FILL,
      tool_pending_fill = TOOL_PENDING_FILL,
      tool_ok_fill = TOOL_OK_FILL,
      tool_failed_fill = TOOL_FAILED_FILL,
    }

    -- One presentation row per block: `fill` paints a background block with
    -- pad rows, `text`/`accent` style the two run kinds inside it.
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

    -- Hiding thinking keeps the block's place in the rhythm and replaces the
    -- reasoning with this placeholder, which is the reviewed canonical row.
    local THINKING_PLACEHOLDER = "Thinking..."

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

    -- One display line: the leading indent and the trailing pad carry the
    -- block fill but no foreground, which is exactly how the canonical frames
    -- record a filled row.
    local function line_of(width, runs, fill)
      local out = { { text = " ", style = fill } }
      local used = 1
      for _, run in ipairs(runs or {}) do
        if type(run) == "table" and type(run.text) == "string" and #run.text > 0 then
          out[#out + 1] = { text = run.text, style = run.style }
          used = used + text_cells.width(run.text)
        end
      end
      if fill and used < width then
        out[#out + 1] = { text = string.rep(" ", width - used), style = fill }
      end
      return { runs = out }
    end

    local function padded_row(width, fill)
      return { runs = { { text = string.rep(" ", width), style = fill } } }
    end

    -- ---------------------------------------------------------------------
    -- Shipped block renderers
    -- ---------------------------------------------------------------------

    -- Every text-shaped block is the same shape: optional pad row, wrapped
    -- rows at the block's own style, optional pad row.
    local function text_block(presentation, value, context)
      local fill = fill_style(presentation)
      local style = styled(presentation, "text")
      local lines = {}
      if fill then
        lines[#lines + 1] = context.padded(fill)
      end
      local rows = text_cells.wrap(value, {
        width = context.body,
        limit = context.limits.max_block_rows,
      })
      if #rows == 0 then
        rows = { "" }
      end
      for _, row in ipairs(rows) do
        lines[#lines + 1] = context.line({ { text = row, style = style } }, fill)
      end
      if fill then
        lines[#lines + 1] = context.padded(fill)
      end
      return lines
    end

    local function render_user(entry, context)
      return text_block(PRESENTATION.user, entry.text, context)
    end

    local function render_assistant(entry, context)
      return text_block(PRESENTATION.assistant, entry.text, context)
    end

    local function render_thinking(entry, context)
      local shown = entry.text
      if (context.options or {}).thinking_visible == false then
        shown = THINKING_PLACEHOLDER
      end
      return text_block(PRESENTATION.thinking, shown, context)
    end

    local function render_notice(entry, context)
      local presentation = PRESENTATION["notice_" .. tostring(entry.level)]
        or PRESENTATION.notice_info
      return text_block(presentation, entry.text, context)
    end

    local function render_tool(entry, context)
      local presentation = PRESENTATION.tool_pending
      if entry.state == "ok" then
        presentation = PRESENTATION.tool_ok
      elseif entry.state == "failed" then
        presentation = PRESENTATION.tool_failed
      end

      local fill = fill_style(presentation)
      local lines = {}
      if fill then
        lines[#lines + 1] = context.padded(fill)
      end

      local name, name_width = text_cells.truncate(entry.name, { width = context.body })
      local runs = { { text = name, style = styled(presentation, "text") } }
      local argument = entry.argument or ""
      if #argument > 0 and name_width + 1 < context.body then
        local shown = text_cells.truncate(argument, { width = context.body - name_width - 1 })
        runs[#runs + 1] = { text = " ", style = fill }
        runs[#runs + 1] = { text = shown, style = styled(presentation, "accent") }
      end
      lines[#lines + 1] = context.line(runs, fill)

      -- A settled call collapses to what was run; only a failure keeps its
      -- output, because that is the part the user has to act on.
      if entry.output and #entry.output > 0 then
        local accent = styled(presentation, "accent")
        local rows = text_cells.wrap(entry.output, { width = context.body, limit = 4 })
        for _, row in ipairs(rows) do
          lines[#lines + 1] = context.line({ { text = row, style = accent } }, fill)
        end
      end

      if fill then
        lines[#lines + 1] = context.padded(fill)
      end
      return lines
    end

    local SHIPPED = {
      { entry = "user", render = render_user },
      { entry = "assistant", render = render_assistant },
      { entry = "thinking", render = render_thinking },
      { entry = "tool", render = render_tool },
      { entry = "notice", render = render_notice },
    }

    --- The shipped block renderers as declaration rows, ready for
    --- `pi.kernel.v1.declare("renderer", row)`. The package file below
    --- declares them, so they belong to its scope like any other package's.
    local function declarations()
      local rows = {}
      for index, shipped in ipairs(SHIPPED) do
        rows[index] = {
          id = "pi.frontend.transcript." .. shipped.entry,
          surface = SURFACE,
          entry = shipped.entry,
          order = 0,
          render = shipped.render,
        }
      end
      return rows
    end

    -- One bounded host read per frame. Later beats earlier, and registered
    -- order is `order`, then source, then id, so a positive `order` replaces
    -- a shipped block deterministically.
    local function resolve()
      local chosen = {}
      for _, declaration in ipairs(kernel.registered("renderer")) do
        if
          declaration.surface == SURFACE
          and type(declaration.entry) == "string"
          and type(declaration.render) == "function"
        then
          chosen[declaration.entry] = declaration.render
        end
      end
      return chosen
    end

    -- An entry kind nothing claims still shows its text: presentation may be
    -- removed, content may not silently vanish with it.
    local function fallback(entry, context)
      local rows = text_cells.wrap(tostring(entry.text or ""), {
        width = context.body,
        limit = context.limits.max_block_rows,
      })
      if #rows == 0 then
        rows = { "" }
      end
      local lines = {}
      for _, row in ipairs(rows) do
        lines[#lines + 1] = context.line({ { text = row } })
      end
      return lines
    end

    -- ---------------------------------------------------------------------
    -- Entry list
    -- ---------------------------------------------------------------------

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
      local options = {}
      for key, value in pairs(DEFAULT_OPTIONS) do
        options[key] = value
      end
      return setmetatable({
        entries = {},
        limits = bounds,
        options = options,
        revision = 0,
        streaming = nil,
        reasoning = nil,
        tools = {},
        statuses = {},
      }, Transcript)
    end

    -- Presentation options are ordinary frontend policy: bounded scalars any
    -- renderer may read. The transcript stores them because it owns the
    -- per-frame renderer context, not because it interprets them.
    function Transcript:set_option(key, value)
      local name = tostring(key)
      local kind = type(value)
      if kind ~= "string" and kind ~= "number" and kind ~= "boolean" then
        return false
      end
      if self.options[name] == nil then
        local count = 0
        for _ in pairs(self.options) do
          count = count + 1
        end
        if count >= MAX_OPTIONS then
          return false
        end
      end
      self.options[name] = value
      self:touch()
      return true
    end

    function Transcript:option(key)
      return self.options[tostring(key)]
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
      -- Any other block closes an open reasoning block, so a later thinking
      -- delta starts a new one instead of reopening a settled row.
      if entry.kind ~= "thinking" then
        self.reasoning = nil
      end
      self.entries[#self.entries + 1] = entry
      while #self.entries > self.limits.max_entries do
        -- A dropped entry is marked, so a keyed reference kept elsewhere
        -- cannot silently rewrite a row that is no longer in the list.
        local dropped = table.remove(self.entries, 1)
        dropped.removed = true
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

    -- Reasoning streams like assistant text: deltas grow one open block and
    -- the completed text closes it, so a whole reasoning turn is one entry
    -- however many events the provider sent.
    function Transcript:thinking_delta(delta)
      local chunk = tostring(delta or "")
      if #chunk == 0 then
        return nil
      end
      if self.reasoning == nil or self.reasoning.removed then
        self.streaming = nil
        self.reasoning = self:push({ kind = "thinking", text = "" })
      end
      self.reasoning.text = clip(self.reasoning.text .. chunk, self.limits.max_entry_bytes)
      self:touch()
      return self.reasoning
    end

    function Transcript:thinking(text)
      self.streaming = nil
      local value = clip(text, self.limits.max_entry_bytes)
      local open = self.reasoning
      self.reasoning = nil
      if open ~= nil and not open.removed then
        if #value > 0 then
          open.text = value
        end
        self:touch()
        return open
      end
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
      entry.output = (not ok) and summary or nil
      self:touch()
      return entry
    end

    local function notice_level(level)
      local name = tostring(level or "info")
      if name ~= "error" and name ~= "warn" then
        name = "info"
      end
      return name
    end

    function Transcript:notice(level, text)
      self.streaming = nil
      return self:push({
        kind = "notice",
        level = notice_level(level),
        text = clip(text, self.limits.max_entry_bytes),
      })
    end

    -- A keyed notice. Re-announcing the same key rewrites the row already in
    -- the transcript instead of appending a second one, so a toggle pressed
    -- twice stays one line of history rather than two.
    function Transcript:status(key, level, text)
      local name = tostring(key)
      local existing = self.statuses[name]
      if existing ~= nil and not existing.removed then
        existing.level = notice_level(level)
        existing.text = clip(text, self.limits.max_entry_bytes)
        self:touch()
        return existing
      end
      local entry = self:notice(level, text)
      self.statuses[name] = entry
      return entry
    end

    -- ---------------------------------------------------------------------
    -- Presentation
    -- ---------------------------------------------------------------------

    --- The per-frame renderer context. Width is constant across the blocks of
    --- one frame, so this is built once per `lines` call, not once per block.
    function Transcript:context(width)
      local columns = math.max(1, math.floor(tonumber(width) or 80))
      local body = math.max(1, columns - 1)
      local limits = {}
      for key, value in pairs(self.limits) do
        limits[key] = value
      end
      local options = {}
      for key, value in pairs(self.options) do
        options[key] = value
      end
      return {
        width = columns,
        body = body,
        limits = limits,
        options = options,
        line = function(runs, fill)
          return line_of(columns, runs, fill)
        end,
        padded = function(fill)
          return padded_row(columns, fill)
        end,
      }
    end

    --- One entry's display lines, through the renderer that claims its kind.
    --- `renderers` is the map `resolve()` returned for this frame; omitting it
    --- resolves the declarations again, which is the convenient path for a
    --- caller rendering one block on its own.
    function Transcript:block(entry, context, renderers)
      local render = (renderers or resolve())[entry.kind]
      local produced = nil
      if render ~= nil then
        produced = render(entry, context)
      end
      if type(produced) ~= "table" then
        produced = fallback(entry, context)
      end

      -- A renderer is ordinary policy: bound what it hands back so one
      -- package cannot make a frame unbounded.
      local ceiling = math.floor(context.limits.max_block_rows) + 2
      local lines = {}
      for _, line in ipairs(produced) do
        if #lines >= ceiling then
          break
        end
        if type(line) == "table" and type(line.runs) == "table" then
          lines[#lines + 1] = line
        end
      end
      return lines
    end

    -- Only the newest `limit` lines are ever built, so presentation cost is
    -- bounded by the viewport rather than by history length.
    function Transcript:lines(width, limit)
      local context = self:context(width)
      local renderers = resolve()
      local budget = math.max(1, math.floor(tonumber(limit) or self.limits.max_block_rows))
      local collected = {}
      local total = 0
      for index = #self.entries, 1, -1 do
        if total >= budget then
          break
        end
        local block = self:block(self.entries[index], context, renderers)
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
      self.reasoning = nil
      self.tools = {}
      self.statuses = {}
      self:touch()
    end

    return {
      new = Transcript.new,
      defaults = DEFAULT_LIMITS,
      default_options = DEFAULT_OPTIONS,
      separator = SEPARATOR,
      surface = SURFACE,
      palette = PALETTE,
      declarations = declarations,
    }
  end,
})

-- Declared by the package file, not by the module factory, so the shipped
-- blocks belong to this package's own source and scope: suppressing the
-- package retracts them exactly like any other declaration.
local transcript = module.require("pi.frontend.transcript", "1")
for _, declaration in ipairs(transcript.declarations()) do
  kernel.declare("renderer", declaration)
end
