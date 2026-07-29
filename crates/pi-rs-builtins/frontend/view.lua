-- Retained component tree for the shipped frontend.
--
-- One pure function turns frontend state into one display batch: header,
-- transcript, guidance, editor, and footer are components with stable node
-- identities, so an unchanged region is retained rather than repainted. Rust
-- performs the diff and emits ANSI; the tree, the layout, and every string in
-- it are Lua.

local pi = ...
local module = pi.kernel.v1.module
local terminal = pi.terminal.v1

module.define({
  name = "pi.frontend.view",
  version = "1",
  factory = function()
    -- Stable identities: a retained node keeps its id across frames.
    local NODE_ROOT = 1
    local NODE_HEADER = 2
    local NODE_TRANSCRIPT = 3
    local NODE_EDITOR = 4
    local NODE_FOOTER = 5
    local NODE_GUIDANCE = 6
    local NODE_TRANSCRIPT_ROW = 100
    local NODE_EDITOR_ROW = 200

    local MIN_COLUMNS = 20
    local MIN_ROWS = 4
    local MAX_EDITOR_ROWS = 6
    local PROMPT = "> "
    local CONTINUATION = "  "

    local function runs_node(id, x, y, width, runs)
      return {
        id = id,
        rect = { x = x, y = y, width = width, height = 1 },
        content = {
          kind = "text",
          wrap = "clip",
          runs = runs,
        },
      }
    end

    local function text_node(id, x, y, width, text, style)
      return {
        id = id,
        rect = { x = x, y = y, width = width, height = 1 },
        content = {
          kind = "text",
          wrap = "clip",
          runs = { { text = text, style = style } },
        },
      }
    end

    local function group_node(id, x, y, width, height, children, focusable)
      return {
        id = id,
        rect = { x = x, y = y, width = width, height = height },
        clip_children = true,
        focusable = focusable == true,
        content = { kind = "group" },
        children = children,
      }
    end

    local function clamp(value, low, high)
      if value < low then
        return low
      end
      if value > high then
        return high
      end
      return value
    end

    -- Layout is deliberately simple and fully recomputed per frame: resize is
    -- just a different state, not a special code path.
    local function build(state)
      local columns = clamp(tonumber(state.columns) or 80, MIN_COLUMNS, 1000)
      local rows = clamp(tonumber(state.rows) or 24, MIN_ROWS, 1000)

      local editor_lines = state.editor_lines
      if type(editor_lines) ~= "table" or #editor_lines == 0 then
        editor_lines = { "" }
      end
      local guidance = state.guidance
      local guidance_height = guidance and 1 or 0

      local editor_height = clamp(#editor_lines, 1, MAX_EDITOR_ROWS)
      local transcript_height = rows - 2 - guidance_height - editor_height
      if transcript_height < 1 then
        editor_height = clamp(editor_height + transcript_height - 1, 1, MAX_EDITOR_ROWS)
        transcript_height = rows - 2 - guidance_height - editor_height
      end
      if transcript_height < 1 then
        transcript_height = 1
      end

      local transcript_y = 1
      local guidance_y = transcript_y + transcript_height
      local editor_y = guidance_y + guidance_height
      local footer_y = rows - 1

      local nodes = {}
      local root_children = { NODE_HEADER, NODE_TRANSCRIPT }

      nodes[#nodes + 1] = text_node(
        NODE_HEADER,
        0,
        0,
        columns,
        state.header or "pi",
        { bold = true }
      )

      -- The transcript is bottom anchored: the newest block sits against the
      -- prompt and older lines scroll off the top, so a growing conversation
      -- moves upward instead of jumping. History stays bounded in the
      -- transcript module and the frame only ever holds what fits.
      local transcript = state.transcript or {}
      local first = #transcript - transcript_height + 1
      if first < 1 then
        first = 1
      end
      local transcript_children = {}
      local slot = math.max(0, transcript_height - #transcript)
      for index = first, #transcript do
        local line = transcript[index]
        local runs = line.runs or {}
        -- A block separator has no runs: it stays an untouched row rather
        -- than a painted blank one.
        if #runs > 0 then
          local id = NODE_TRANSCRIPT_ROW + slot
          nodes[#nodes + 1] = runs_node(id, 0, slot, columns, runs)
          transcript_children[#transcript_children + 1] = id
        end
        slot = slot + 1
      end
      nodes[#nodes + 1] = group_node(
        NODE_TRANSCRIPT,
        0,
        transcript_y,
        columns,
        transcript_height,
        transcript_children
      )

      if guidance then
        root_children[#root_children + 1] = NODE_GUIDANCE
        nodes[#nodes + 1] = text_node(
          NODE_GUIDANCE,
          0,
          guidance_y,
          columns,
          "! " .. guidance,
          { bold = true }
        )
      end

      local editor_children = {}
      local cursor_row = clamp(tonumber(state.cursor and state.cursor.row) or 1, 1, #editor_lines)
      local first_editor = clamp(cursor_row - editor_height + 1, 1, math.max(1, #editor_lines))
      local cursor_node = NODE_EDITOR_ROW
      slot = 0
      for index = first_editor, math.min(#editor_lines, first_editor + editor_height - 1) do
        local id = NODE_EDITOR_ROW + slot
        local prefix = index == 1 and PROMPT or CONTINUATION
        nodes[#nodes + 1] = text_node(id, 0, slot, columns, prefix .. (editor_lines[index] or ""))
        editor_children[#editor_children + 1] = id
        if index == cursor_row then
          cursor_node = id
        end
        slot = slot + 1
      end
      root_children[#root_children + 1] = NODE_EDITOR
      -- The editor is the only focusable component today; focus routing in
      -- the frontend root decides who receives keys.
      nodes[#nodes + 1] =
        group_node(NODE_EDITOR, 0, editor_y, columns, editor_height, editor_children, true)

      root_children[#root_children + 1] = NODE_FOOTER
      nodes[#nodes + 1] =
        text_node(NODE_FOOTER, 0, footer_y, columns, state.footer or "", { dim = true })

      nodes[#nodes + 1] = group_node(NODE_ROOT, 0, 0, columns, rows, root_children)

      local prefix_width = cursor_row == 1 and #PROMPT or #CONTINUATION
      local cursor_column = clamp(
        prefix_width + (tonumber(state.cursor and state.cursor.column) or 1) - 1,
        0,
        columns - 1
      )

      return {
        version = terminal.display_schema_version,
        viewport = { columns = columns, rows = rows },
        root = NODE_ROOT,
        nodes = nodes,
        focused = NODE_EDITOR,
        -- The cursor row node already carries its own offset inside the
        -- editor group, so the cursor is at row 0 of that node.
        cursor = {
          node = cursor_node,
          row = 0,
          column = cursor_column,
          shape = "bar",
          visible = true,
        },
      }
    end

    return {
      build = build,
      nodes = {
        root = NODE_ROOT,
        header = NODE_HEADER,
        transcript = NODE_TRANSCRIPT,
        editor = NODE_EDITOR,
        footer = NODE_FOOTER,
        guidance = NODE_GUIDANCE,
      },
      prompt = PROMPT,
    }
  end,
})
