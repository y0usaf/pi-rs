-- read.ts — the read tool.
--
-- Deferred (carried in PLAN.md): the pi-docs compact-read
-- classification (needs pi's bundled README/docs tree, which pi-rs does
-- not ship yet). The spec's `autoResizeImages` creation option is
-- embedder-only — the coding agent always uses the default (true), so
-- the non-resizing branch is not ported.

-- formatReadLineRange: ":start" or ":start-end" in warning color.
local function format_read_line_range(args, theme)
  if args == nil or (args.offset == nil and args.limit == nil) then return "" end
  local start_line = args.offset or 1
  local end_line = args.limit ~= nil and (start_line + args.limit - 1) or nil
  -- JS renders no suffix for a falsy endLine (including 0).
  local suffix = (end_line ~= nil and end_line ~= 0) and ("-" .. fmt_num(end_line)) or ""
  return theme:fg("warning", ":" .. fmt_num(start_line) .. suffix)
end

local function read_raw_path(args)
  if args == nil then return "" end
  if args.file_path ~= nil then return str(args.file_path) end
  return str(args.path)
end

local function format_read_call(args, theme, base_cwd)
  local path_display = render_tool_path(read_raw_path(args), theme, base_cwd)
  return theme:fg("toolTitle", theme:bold("read")) .. " " .. path_display
    .. format_read_line_range(args, theme)
end

local COMPACT_RESOURCE_FILE_NAMES = {
  ["AGENTS.md"] = true, ["AGENTS.MD"] = true,
  ["CLAUDE.md"] = true, ["CLAUDE.MD"] = true,
}

local function get_compact_read_classification(args, base_cwd)
  local raw_path = read_raw_path(args)
  if raw_path == nil or raw_path == "" then return nil end
  local absolute_path = resolve_to_cwd(raw_path, base_cwd)
  local file_name = pi.path.basename(absolute_path)
  if file_name == "SKILL.md" then
    local label = pi.path.basename(pi.path.dirname(absolute_path))
    if label == "" then label = file_name end
    return { kind = "skill", label = label }
  end
  if COMPACT_RESOURCE_FILE_NAMES[file_name] then
    return { kind = "resource", label = format_path_relative_to_cwd_or_absolute(absolute_path, base_cwd) }
  end
  return nil
end

local function format_compact_read_call(classification, args, theme)
  local expand_hint = theme:fg("dim", " (" .. key_text("app.tools.expand") .. " to expand)")
  if classification.kind == "skill" then
    return theme:fg("customMessageLabel", "\27[1m[skill]\27[22m ")
      .. theme:fg("customMessageText", classification.label)
      .. format_read_line_range(args, theme)
      .. expand_hint
  end
  return theme:fg("toolTitle", theme:bold("read " .. classification.kind))
    .. " " .. theme:fg("accent", classification.label)
    .. format_read_line_range(args, theme)
    .. expand_hint
end

local function format_read_result(args, result, options, theme, show_images, _base_cwd, is_error)
  if not options.expanded and not is_error then return "" end
  local raw_path = read_raw_path(args)
  local output = get_text_output(result, show_images)
  local lang = (raw_path ~= nil and raw_path ~= "") and get_language_from_path(raw_path) or nil
  local rendered_lines = lang and highlight_code(replace_tabs(output), lang, theme) or split(output, "\n")
  local lines = trim_trailing_empty_lines(rendered_lines)
  local max_lines = options.expanded and #lines or 10
  local remaining = #lines - max_lines
  local display = {}
  for i = 1, math.min(max_lines, #lines) do
    local line = lines[i]
    display[i] = lang and replace_tabs(line) or theme:fg("toolOutput", replace_tabs(line))
  end
  local text = "\n" .. table.concat(display, "\n")
  if remaining > 0 then
    text = text .. theme:fg("muted", ("\n... (%d more lines,"):format(remaining))
      .. " " .. key_hint(theme, "app.tools.expand", "to expand") .. theme:fg("muted", ")")
  end
  local truncation = result.details and result.details.truncation
  if truncation and truncation.truncated then
    if truncation.firstLineExceedsLimit then
      text = text .. "\n" .. theme:fg("warning",
        ("[First line exceeds %s limit]"):format(format_size(truncation.maxBytes or DEFAULT_MAX_BYTES)))
    elseif truncation.truncatedBy == "lines" then
      text = text .. "\n" .. theme:fg("warning",
        ("[Truncated: showing %d of %d lines (%d line limit)]"):format(
          truncation.outputLines, truncation.totalLines, truncation.maxLines or DEFAULT_MAX_LINES))
    else
      text = text .. "\n" .. theme:fg("warning",
        ("[Truncated: %d lines shown (%s limit)]"):format(
          truncation.outputLines, format_size(truncation.maxBytes or DEFAULT_MAX_BYTES)))
    end
  end
  return text
end
-- utils/image-resize.ts formatDimensionNote: coordinate-mapping note for
-- resized images (JS `${n}` interpolation; toFixed(2) for the scale).
local function format_dimension_note(resized)
  if not resized.wasResized then return nil end
  local scale = resized.originalWidth / resized.width
  return ("[Image: original %sx%s, displayed at %sx%s. Multiply coordinates by %s to map to original image.]")
    :format(fmt_num(resized.originalWidth), fmt_num(resized.originalHeight),
      fmt_num(resized.width), fmt_num(resized.height), ("%.2f"):format(scale))
end

-- read.ts getNonVisionImageNote.
local function get_non_vision_image_note(model)
  if not model then return nil end
  for _, input in ipairs(model.input or {}) do
    if input == "image" then return nil end
  end
  return "[Current model does not support images. The image will be omitted from this request.]"
end

pi.register_tool({
  name = "read",
  label = "read",
  description = (
    "Read the contents of a file. Supports text files and images (jpg, png, gif, webp)."
    .. " Images are sent as attachments. For text files, output is truncated to %d lines"
    .. " or %dKB (whichever is hit first). Use offset/limit for large files. When you"
    .. " need the full file, continue with offset until complete."
  ):format(DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES // 1024),
  promptSnippet = "Read file contents",
  promptGuidelines = { "Use read to examine files instead of cat or sed." },
  parameters = {
    type = "object",
    properties = {
      path = { type = "string", description = "Path to the file to read (relative or absolute)" },
      offset = { type = "number", description = "Line number to start reading from (1-indexed)" },
      limit = { type = "number", description = "Maximum number of lines to read" },
    },
    required = { "path" },
  },
  execute = function(_tool_call_id, params, signal, _on_update, ctx)
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    local path, offset, limit = params.path, params.offset, params.limit
    local absolute_path = resolve_read_path(path)
    -- Spec ops.access(R_OK): Node-shaped error so callers can match ENOENT.
    if not pi.fs.exists(absolute_path) then
      error(("ENOENT: no such file or directory, access '%s'"):format(absolute_path), 0)
    end
    local buffer = pi.fs.read_bytes(absolute_path)
    local mime_type = detect_supported_image_mime_type(buffer:sub(1, IMAGE_TYPE_SNIFF_BYTES))

    local non_vision_image_note = get_non_vision_image_note(ctx and ctx.model or nil)
    if mime_type then
      -- Resize image if needed before sending it back to the model
      -- (spec: resizeImage — pi's Photon worker, pi-rs's pi.image.resize).
      local resized = pi.image.resize(buffer, mime_type)
      if not resized then
        local text_note = ("Read image file [%s]\n[Image omitted: could not be resized below the inline image size limit.]")
          :format(mime_type)
        if non_vision_image_note then text_note = text_note .. "\n" .. non_vision_image_note end
        return { content = { { type = "text", text = text_note } } }
      end
      local dimension_note = format_dimension_note(resized)
      local text_note = ("Read image file [%s]"):format(resized.mimeType)
      if dimension_note then text_note = text_note .. "\n" .. dimension_note end
      if non_vision_image_note then text_note = text_note .. "\n" .. non_vision_image_note end
      return {
        content = {
          { type = "text", text = text_note },
          { type = "image", data = resized.data, mimeType = resized.mimeType },
        },
      }
    end

    local text_content = utf8_lossy(buffer)
    local all_lines = split(text_content, "\n")
    local total_file_lines = #all_lines
    -- Convert from 1-indexed input to 0-indexed access (spec keeps the
    -- 0-indexed startLine variable; array reads below convert back).
    local start_line = offset and math.max(0, offset - 1) or 0
    local start_line_display = start_line + 1
    if start_line >= total_file_lines then
      error(("Offset %s is beyond end of file (%d lines total)"):format(fmt_num(offset), total_file_lines), 0)
    end

    local selected_content
    local user_limited_lines
    if limit ~= nil then
      local end_line = math.min(start_line + limit, total_file_lines)
      selected_content = table.concat(all_lines, "\n", start_line + 1, end_line)
      user_limited_lines = end_line - start_line
    else
      selected_content = table.concat(all_lines, "\n", start_line + 1, total_file_lines)
    end

    local truncation = truncate_head(selected_content)
    local output_text
    local details
    if truncation.firstLineExceedsLimit then
      -- First line alone exceeds the byte limit: point at a bash fallback.
      local first_line_size = format_size(#all_lines[start_line + 1])
      output_text = ("[Line %d is %s, exceeds %s limit. Use bash: sed -n '%dp' %s | head -c %d]"):format(
        start_line_display,
        first_line_size,
        format_size(DEFAULT_MAX_BYTES),
        start_line_display,
        path,
        DEFAULT_MAX_BYTES
      )
      details = { truncation = truncation }
    elseif truncation.truncated then
      -- Truncation occurred: build an actionable continuation notice.
      local end_line_display = start_line_display + truncation.outputLines - 1
      local next_offset = end_line_display + 1
      output_text = truncation.content
      if truncation.truncatedBy == "lines" then
        output_text = output_text
          .. ("\n\n[Showing lines %d-%d of %d. Use offset=%d to continue.]"):format(
            start_line_display,
            end_line_display,
            total_file_lines,
            next_offset
          )
      else
        output_text = output_text
          .. ("\n\n[Showing lines %d-%d of %d (%s limit). Use offset=%d to continue.]"):format(
            start_line_display,
            end_line_display,
            total_file_lines,
            format_size(DEFAULT_MAX_BYTES),
            next_offset
          )
      end
      details = { truncation = truncation }
    elseif user_limited_lines ~= nil and start_line + user_limited_lines < total_file_lines then
      -- User-specified limit stopped early but the file has more content.
      local remaining = total_file_lines - (start_line + user_limited_lines)
      local next_offset = start_line + user_limited_lines + 1
      output_text = ("%s\n\n[%d more lines in file. Use offset=%d to continue.]"):format(
        truncation.content,
        remaining,
        next_offset
      )
    else
      output_text = truncation.content
    end

    return { content = { { type = "text", text = output_text } }, details = details }
  end,
  renderCall = function(args, theme, context)
    local classification = (not context.expanded) and get_compact_read_classification(args, context.cwd) or nil
    local text = classification and format_compact_read_call(classification, args, theme)
      or format_read_call(args, theme, context.cwd)
    return text_component(text, 0, 0)
  end,
  renderResult = function(result, options, theme, context)
    return text_component(
      format_read_result(context.args, result, options, theme, context.showImages, context.cwd, context.isError),
      0, 0)
  end,
})
