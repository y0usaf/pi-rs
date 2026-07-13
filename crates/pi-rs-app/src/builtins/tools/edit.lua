-- edit.ts -- exact/fuzzy single-file replacement through the public API.
-- Abort: spec checks signal.aborted inside the mutation queue (never an
-- abort listener, so the queue stays locked until the op settles).
local function edit_raw_path(args)
  if args == nil then return "" end
  if args.file_path ~= nil then return str(args.file_path) end
  return str(args.path)
end

-- getRenderablePreviewInput: a path plus a well-formed edits array (or
-- the legacy oldText/newText pair) that the preview can diff.
local function get_renderable_preview_input(args)
  if args == nil then return nil end
  local path
  if type(args.path) == "string" then path = args.path
  elseif type(args.file_path) == "string" then path = args.file_path
  else return nil end
  if type(args.edits) == "table" and #args.edits > 0 then
    local valid = true
    for _, edit in ipairs(args.edits) do
      if type(edit) ~= "table" or type(edit.oldText) ~= "string" or type(edit.newText) ~= "string" then
        valid = false
        break
      end
    end
    if valid then return { path = path, edits = args.edits } end
  end
  if type(args.oldText) == "string" and type(args.newText) == "string" then
    return { path = path, edits = { { oldText = args.oldText, newText = args.newText } } }
  end
  return nil
end

local function format_edit_call(args, theme, base_cwd)
  local path_display = render_tool_path(edit_raw_path(args), theme, base_cwd)
  return theme:fg("toolTitle", theme:bold("edit")) .. " " .. path_display
end

local function format_edit_result(args, preview, result, theme, is_error)
  local raw_path = edit_raw_path(args)
  local preview_diff = preview and preview.error == nil and preview.diff or nil
  local preview_error = preview and preview.error or nil
  if is_error then
    local parts = {}
    for _, block in ipairs(result.content or {}) do
      if block.type == "text" then parts[#parts + 1] = block.text or "" end
    end
    local error_text = table.concat(parts, "\n")
    if error_text == "" or error_text == preview_error then return nil end
    return theme:fg("error", error_text)
  end
  local result_diff = result.details and result.details.diff
  if result_diff and result_diff ~= preview_diff then
    return render_diff(result_diff, theme, { filePath = raw_path })
  end
  return nil
end

local function get_edit_header_bg(preview, settled_error, theme)
  if preview then
    if preview.error ~= nil then
      return function(text) return theme:bg("toolErrorBg", text) end
    end
    return function(text) return theme:bg("toolSuccessBg", text) end
  end
  if settled_error then
    return function(text) return theme:bg("toolErrorBg", text) end
  end
  return function(text) return theme:bg("toolPendingBg", text) end
end

-- buildEditCallComponent: Box(1,1,headerBg){ call, [Spacer, preview] }.
-- The state is read when the component renders, so a result settled in
-- the same pass (spec: renderResult rebuilding callComponent) is
-- reflected without an explicit rebuild.
local function build_edit_call_component(state, args, theme, base_cwd)
  return function(width)
    local children = { text_component(format_edit_call(args, theme, base_cwd), 0, 0) }
    if state.preview then
      local body = state.preview.error ~= nil and theme:fg("error", state.preview.error)
        or render_diff(state.preview.diff, theme)
      children[#children + 1] = spacer_component(1)
      children[#children + 1] = text_component(body, 0, 0)
    end
    local bg = get_edit_header_bg(state.preview, state.settledError, theme)
    return box_component(children, 1, 1, bg)(width)
  end
end

local function edit_preview_args_key(preview_input)
  if preview_input == nil then return nil end
  return pi.json.encode({ path = preview_input.path, edits = preview_input.edits })
end

pi.register_tool({
  name = "edit", label = "edit",
  active_by_default = true,
  description = "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.",
  promptSnippet = "Make precise file edits with exact text replacement, including multiple disjoint edits in one call",
  promptGuidelines = {
    "Use edit for precise changes (edits[].oldText must match exactly)",
    "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls",
    "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.",
    "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.",
  },
  parameters = { type = "object", properties = {
    path = { type = "string", description = "Path to the file to edit (relative or absolute)" },
    edits = { type = "array", items = { type = "object", properties = {
      oldText = { type = "string", description = "Exact text for one targeted replacement." },
      newText = { type = "string", description = "Replacement text for this targeted edit." },
    }, required = { "oldText", "newText" }, additionalProperties = false } },
  }, required = { "path", "edits" }, additionalProperties = false },
  renderShell = "self",
  prepare_arguments = function(params)
    if type(params) ~= "table" then return params end
    if type(params.edits) == "string" then
      local ok, parsed = pcall(pi.json.decode, params.edits)
      if ok and type(parsed) == "table" then params.edits = parsed end
    end
    if type(params.oldText) == "string" and type(params.newText) == "string" then
      local edits = type(params.edits) == "table" and params.edits or {}
      edits[#edits + 1] = { oldText = params.oldText, newText = params.newText }
      params.oldText, params.newText, params.edits = nil, nil, edits
    end
    return params
  end,
  execute = function(_tool_call_id, params, signal)
    local path, edits = params.path, params.edits
    if type(edits) ~= "table" or #edits == 0 then error("Edit tool input is invalid. edits must contain at least one replacement.", 0) end
    local absolute_path = resolve_to_cwd(path)
    return with_file_mutation_queue(absolute_path, function()
      if signal and signal:is_aborted() then error("Operation aborted", 0) end
      if not pi.fs.exists(absolute_path) then error("Could not edit file: " .. path .. ". Error code: ENOENT.", 0) end
      local raw = utf8_lossy(pi.fs.read_bytes(absolute_path))
      local bom = ""
      if raw:sub(1, 3) == "\239\187\191" then bom, raw = "\239\187\191", raw:sub(4) end
      local ending = detect_line_ending(raw)
      local base, changed = apply_edits_to_normalized_content(normalize_to_lf(raw), edits, path)
      pi.fs.write_file(absolute_path, bom .. restore_line_endings(changed, ending))
      local diff, first = generate_diff_string(base, changed)
      return {
        content = { { type = "text", text = ("Successfully replaced %d block(s) in %s."):format(#edits, path) } },
        details = { diff = diff, patch = generate_unified_patch(path, base, changed), firstChangedLine = first },
      }
    end)
  end,
  renderCall = function(args, theme, context)
    local state = context.state
    local preview_input = get_renderable_preview_input(args)
    local args_key = edit_preview_args_key(preview_input)
    if state.previewArgsKey ~= args_key then
      state.preview = nil
      state.previewArgsKey = args_key
      state.settledError = false
    end
    if context.argsComplete and preview_input and state.preview == nil then
      -- Spec computes the preview asynchronously and invalidates; the Lua
      -- port awaits the same file read inline within the render pass.
      state.preview = compute_edits_diff(preview_input.path, preview_input.edits, context.cwd)
    end
    return build_edit_call_component(state, args, theme, context.cwd)
  end,
  renderResult = function(result, _options, theme, context)
    local state = context.state
    local preview_input = get_renderable_preview_input(context.args)
    local args_key = edit_preview_args_key(preview_input)
    local result_diff = not context.isError and result.details and result.details.diff or nil
    if type(result_diff) == "string" then
      state.preview = { diff = result_diff, firstChangedLine = result.details.firstChangedLine }
      state.previewArgsKey = args_key
    end
    state.settledError = context.isError
    local output = format_edit_result(context.args, state.preview, result, theme, context.isError)
    if not output then
      return function() return {} end
    end
    return function(width)
      local lines = { "" }
      for _, line in ipairs(pi.tui.text_render(output, width, 1, 0)) do
        lines[#lines + 1] = line
      end
      return lines
    end
  end,
})
