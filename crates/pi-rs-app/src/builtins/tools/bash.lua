-- bash.ts — exact pi v0.79 execute behavior within pi-rs's current surface.
-- commandPrefix/shellPath/spawnHook factory options arrive with their
-- owning frontend/context mechanisms. Abort: the tool signal reaches
-- pi.exec (spec: createLocalBashOperations' killProcessTree-on-abort)
-- and surfaces as "Command aborted" after the partial output.
local BASH_PREVIEW_LINES = 5

local function format_duration(ms)
  return string.format("%.1fs", ms / 1000)
end

local function format_bash_call(args, theme)
  local command = ""
  if args ~= nil then command = str(args.command) end
  local timeout = args ~= nil and args.timeout or nil
  -- JS truthiness: no suffix for a missing or zero timeout.
  local timeout_suffix = (type(timeout) == "number" and timeout ~= 0)
    and theme:fg("muted", " (timeout " .. fmt_num(timeout) .. "s)") or ""
  local command_display
  if command == nil then
    command_display = invalid_arg_text(theme)
  elseif command ~= "" then
    command_display = command
  else
    command_display = theme:fg("toolOutput", "...")
  end
  return theme:fg("toolTitle", theme:bold("$ " .. command_display)) .. timeout_suffix
end

-- rebuildBashResultRenderComponent as a lines-producing component: the
-- Container of Texts plus the width-cached preview renderer, evaluated
-- per render width (pi-rs re-renders pure components each frame, so the
-- spec's per-component caches are unnecessary).
local function bash_result_component(result, options, theme, context, started_at, ended_at)
  return function(width)
    local lines = {}
    local output = js_trim(get_text_output(result, context.showImages))
    local truncation = result.details and result.details.truncation
    local full_output_path = result.details and result.details.fullOutputPath
    if not options.isPartial and truncation and truncation.truncated
      and full_output_path and output:sub(-1) == "]" then
      -- Strip the execute-time footer; the renderer re-adds warnings.
      local footer_start = nil
      local search = 1
      while true do
        local found = output:find("\n\n[", search, true)
        if not found then break end
        footer_start = found
        search = found + 1
      end
      if footer_start and output:sub(footer_start):find(full_output_path, 1, true) then
        output = (output:sub(1, footer_start - 1):gsub("%s+$", ""))
      end
    end

    if output ~= "" then
      local styled_parts = {}
      for _, line in ipairs(split(output, "\n")) do
        styled_parts[#styled_parts + 1] = theme:fg("toolOutput", line)
      end
      local styled_output = table.concat(styled_parts, "\n")
      if options.expanded then
        for _, line in ipairs(pi.tui.text_render("\n" .. styled_output, width, 0, 0)) do
          lines[#lines + 1] = line
        end
      else
        local preview = truncate_to_visual_lines(styled_output, BASH_PREVIEW_LINES, width)
        if preview.skippedCount > 0 then
          local hint = theme:fg("muted", ("... (%d earlier lines,"):format(preview.skippedCount))
            .. " " .. key_hint(theme, "app.tools.expand", "to expand") .. theme:fg("muted", ")")
          lines[#lines + 1] = ""
          lines[#lines + 1] = pi.tui.truncate(hint, width, "...", false)
        else
          lines[#lines + 1] = ""
        end
        for _, line in ipairs(preview.visualLines) do lines[#lines + 1] = line end
      end
    end

    if (truncation and truncation.truncated) or full_output_path then
      local warnings = {}
      if full_output_path then
        warnings[#warnings + 1] = "Full output: " .. full_output_path
      end
      if truncation and truncation.truncated then
        if truncation.truncatedBy == "lines" then
          warnings[#warnings + 1] = ("Truncated: showing %d of %d lines"):format(
            truncation.outputLines, truncation.totalLines)
        else
          warnings[#warnings + 1] = ("Truncated: %d lines shown (%s limit)"):format(
            truncation.outputLines, format_size(truncation.maxBytes or DEFAULT_MAX_BYTES))
        end
      end
      local warning_text = "\n" .. theme:fg("warning", "[" .. table.concat(warnings, ". ") .. "]")
      for _, line in ipairs(pi.tui.text_render(warning_text, width, 0, 0)) do
        lines[#lines + 1] = line
      end
    end

    if started_at ~= nil then
      local label = options.isPartial and "Elapsed" or "Took"
      local end_time = ended_at or context.now_ms()
      local duration_text = "\n" .. theme:fg("muted", label .. " " .. format_duration(end_time - started_at))
      for _, line in ipairs(pi.tui.text_render(duration_text, width, 0, 0)) do
        lines[#lines + 1] = line
      end
    end
    return lines
  end
end

pi.register_tool({
  name = "bash",
  label = "bash",
  description = "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
  promptSnippet = "Execute bash commands (ls, grep, find, etc.)",
  parameters = {
    type = "object",
    properties = {
      command = { type = "string", description = "Bash command to execute" },
      timeout = { type = "number", description = "Timeout in seconds (optional, no default timeout)" },
    },
    required = { "command" },
  },
  execute = function(tool_call_id, params, signal, on_update)
    if not pi.fs.exists(cwd) then
      error("Working directory does not exist: " .. cwd .. "\nCannot execute bash commands.")
    end
    local safe = tostring(tool_call_id):gsub("[^%w_.-]", "-")
    local full_path = pi.fs.create_temp_file("pi-bash-" .. safe .. "-", "")
    local output = new_output_accumulator(full_path)
    local last_update_ms, dirty = -math.huge, false
    local function update(force)
      if not on_update or not dirty then return end
      local now = pi.monotonic_ms()
      if not force and now - last_update_ms < 100 then return end
      local snapshot = output.snapshot()
      on_update({
        content = { { type = "text", text = snapshot.content or "" } },
        details = snapshot.truncation.truncated and {
          truncation = snapshot.truncation,
          fullOutputPath = snapshot.fullOutputPath,
        } or nil,
      })
      dirty, last_update_ms = false, now
    end
    if on_update then on_update({ content = {} }) end
    local shell, args = shell_config()
    args[#args + 1] = params.command
    local timeout = type(params.timeout) == "number" and params.timeout > 0
      and params.timeout * 1000 or nil
    local result
    -- Spec ops.exec: signal already aborted before spawn throws "aborted"
    -- without spawning; mid-run aborts kill the process tree.
    if not (signal and signal:is_aborted()) then
      result = pi.exec(shell, args, {
        cwd = cwd,
        timeout = timeout,
        signal = signal,
        onData = function(data)
          pi.fs.append_file(full_path, data)
          output.append(data)
          dirty = true
          update(false)
        end,
      })
    end
    output.finish()
    update(true)
    local snapshot = output.snapshot()
    local truncation = snapshot.truncation
    if not truncation.truncated then pi.fs.remove_file(full_path) end
    local text = snapshot.content ~= "" and snapshot.content or "(no output)"
    local details
    if truncation.truncated then
      details = { truncation = truncation, fullOutputPath = snapshot.fullOutputPath }
      local first = truncation.totalLines - truncation.outputLines + 1
      local last = truncation.totalLines
      if truncation.lastLinePartial then
        text = text .. "\n\n[Showing last " .. format_size(truncation.outputBytes) .. " of line " .. last
          .. " (line is " .. format_size(output.last_line_bytes()) .. "). Full output: " .. snapshot.fullOutputPath .. "]"
      elseif truncation.truncatedBy == "lines" then
        text = text .. "\n\n[Showing lines " .. first .. "-" .. last .. " of " .. truncation.totalLines
          .. ". Full output: " .. snapshot.fullOutputPath .. "]"
      else
        text = text .. "\n\n[Showing lines " .. first .. "-" .. last .. " of " .. truncation.totalLines
          .. " (" .. format_size(DEFAULT_MAX_BYTES) .. " limit). Full output: " .. snapshot.fullOutputPath .. "]"
      end
    end
    -- Spec appendStatus over formatOutput: the abort/timeout paths
    -- format with emptyText "" (status alone when nothing was printed),
    -- while the exit-code path keeps the "(no output)" default text.
    local function fail(base, status)
      local prefix = base ~= "" and (base .. "\n\n") or ""
      error(prefix .. status)
    end
    local error_text = snapshot.content ~= "" and text or ""
    -- Spec order: aborted wins over the timeout's killed flag.
    if signal and signal:is_aborted() then fail(error_text, "Command aborted") end
    if result.killed then fail(error_text, "Command timed out after " .. fmt_num(params.timeout) .. " seconds") end
    if result.code ~= 0 then fail(text, "Command exited with code " .. result.code) end
    return { content = { { type = "text", text = text } }, details = details }
  end,
  renderCall = function(args, theme, context)
    local state = context.state
    if context.executionStarted and state.startedAt == nil then
      state.startedAt = context.now_ms()
      state.endedAt = nil
    end
    return text_component(format_bash_call(args, theme), 0, 0)
  end,
  renderResult = function(result, options, theme, context)
    local state = context.state
    -- The spec's per-second invalidate interval is unnecessary: the
    -- frontend re-renders streaming frames on tick.
    if not options.isPartial or context.isError then
      if state.endedAt == nil then state.endedAt = context.now_ms() end
    end
    return bash_result_component(result, options, theme, context, state.startedAt, state.endedAt)
  end,
})
