-- Translation of Pi v0.79.0 examples/extensions/truncated-tool.ts.
-- Wraps ripgrep with proper output truncation (50KB / 2000 lines) and
-- custom rendering, writing full output to a temp file when truncated.
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local mutation_queue = pi.module.require("pi.tools.file-mutation-queue", "1")

local rg_tool = nil
for _, definition in ipairs(pi.registered_tools()) do
  if definition.name == "grep" then rg_tool = definition break end
end

local function grep_args_string(pattern, search_path, glob)
  local a = { "rg", "--line-number", "--color=never" }
  if glob then a[#a + 1] = "--glob"; a[#a + 1] = glob end
  a[#a + 1] = pattern
  a[#a + 1] = search_path or "."
  return a
end

pi.register_tool({
  name = "rg",
  label = "ripgrep",
  description = "Search file contents using ripgrep. Output is truncated to " .. truncate.DEFAULT_MAX_LINES .. " lines or " .. truncate.format_size(truncate.DEFAULT_MAX_BYTES) .. " (whichever is hit first). If truncated, full output is saved to a temp file.",
  parameters = {
    type = "object",
    properties = {
      pattern = { type = "string", description = "Search pattern (regex)" },
      path = { type = "string", description = "Directory to search (default: current directory)" },
      glob = { type = "string", description = "File glob pattern, e.g. '*.ts'" },
    },
    required = { "pattern" },
  },

  execute = function(_tool_call_id, params, _signal, _on_update, ctx)
    local pattern, search_path, glob = params.pattern, params.path, params.glob

    -- Build and run the ripgrep command via the shared shell; ripgrep exits
    -- with 1 when there are no matches.
    local ok, result = pcall(pi.exec, "rg", grep_args_string(pattern, search_path, glob), { cwd = ctx.cwd })
    if not ok then
      error("ripgrep failed: " .. tostring(result), 0)
    end
    if result.code == 1 then
      return {
        content = { { type = "text", text = "No matches found" } },
        details = { pattern = pattern, path = search_path, glob = glob, matchCount = 0 },
      }
    end
    if result.code ~= 0 then
      error("ripgrep failed: " .. (result.stderr ~= "" and result.stderr or "exit " .. result.code), 0)
    end

    local output = result.stdout
    if output:gsub("%s+", "") == "" then
      return {
        content = { { type = "text", text = "No matches found" } },
        details = { pattern = pattern, path = search_path, glob = glob, matchCount = 0 },
      }
    end

    -- Apply truncation using the built-in utilities
    local truncation = truncate.truncate_head(output, {
      maxLines = truncate.DEFAULT_MAX_LINES,
      maxBytes = truncate.DEFAULT_MAX_BYTES,
    })

    -- Count matches (each non-empty line with a match)
    local match_count = 0
    for line in output:gmatch("[^\n]+") do
      if line:gsub("%s+", "") ~= "" then match_count = match_count + 1 end
    end

    local details = { pattern = pattern, path = search_path, glob = glob, matchCount = match_count }

    local result_text = truncation.content

    if truncation.truncated then
      -- Save full output to a temp file so the LLM can access it if needed
      local temp_dir = pi.fs.mkdtemp("pi-rg-")
      local temp_file = pi.path.join(temp_dir, "output.txt")
      mutation_queue.with_file_mutation_queue(temp_file, function()
        pi.fs.write_file(temp_file, output)
      end)

      details.truncation = truncation
      details.fullOutputPath = temp_file

      local truncated_lines = truncation.totalLines - truncation.outputLines
      local truncated_bytes = truncation.totalBytes - truncation.outputBytes

      result_text = result_text .. "\n\n[Output truncated: showing " .. truncation.outputLines .. " of " .. truncation.totalLines .. " lines"
      result_text = result_text .. " (" .. truncate.format_size(truncation.outputBytes) .. " of " .. truncate.format_size(truncation.totalBytes) .. ")."
      result_text = result_text .. " " .. truncated_lines .. " lines (" .. truncate.format_size(truncated_bytes) .. ") omitted."
      result_text = result_text .. " Full output saved to: " .. temp_file .. "]"
    end

    return { content = { { type = "text", text = result_text } }, details = details }
  end,

  renderCall = function(args, theme, _context)
    local text = theme:fg("toolTitle", theme:bold("rg ")) .. theme:fg("accent", "\"" .. args.pattern .. "\"")
    if args.path then text = text .. theme:fg("muted", " in " .. args.path) end
    if args.glob then text = text .. theme:fg("dim", " --glob " .. args.glob) end
    return pi.tui.text(text, 0, 0)
  end,

  renderResult = function(result, options, theme, _context)
    local details = result.details
    if not details then
      local content = result.content[1]
      return pi.tui.text(content and content.type == "text" and content.text or "", 0, 0)
    end

    if details.matchCount == 0 then
      return pi.tui.text(theme:fg("dim", "No matches found"), 0, 0)
    end

    local text = theme:fg("success", details.matchCount .. " matches")
    if details.truncation and details.truncation.truncated then
      text = text .. theme:fg("warning", " (truncated)")
    end

    if options.expanded then
      local content = result.content[1]
      if content and content.type == "text" then
        local i = 0
        for line in content.text:gmatch("[^\n]+") do
          i = i + 1
          if i > 20 then break end
          text = text .. "\n" .. theme:fg("dim", line)
        end
        if i > 20 then text = text .. "\n" .. theme:fg("muted", "... (use read tool to see full output)") end
      end
      if details.fullOutputPath then
        text = text .. "\n" .. theme:fg("dim", "Full output: " .. details.fullOutputPath)
      end
    end

    return pi.tui.text(text, 0, 0)
  end,
})