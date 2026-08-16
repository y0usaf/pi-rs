-- File-backed pi-morph translation (dogfood package).
--
-- Merge partial code edits into existing files via Morph on Vercel AI Gateway.
-- Public surface only: events (on), register_command, register_tool, ctx.{cwd,
-- modelRegistry, ui}, ctx.ui.{setStatus,notify,theme.fg}, pi.http.fetch
-- (timeout_ms contract), pi.fs (stat/read_file/mkdir/rename/unlink/write_file_atomic/
-- exists), pi.path (join/dirname/is_absolute/resolve), pi.env, pi.crypto.random_uuid,
-- pi.module.require("pi.tools.file-mutation-queue","1") for with_file_mutation_queue,
-- and pi.module.require("pi.tools.truncate","1") for format_size.
-- No privileged escape hatch, no long-lived host resources (each tool call is a
-- standalone request; the mutation queue ownership is released on completion/error).
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local format_size = truncate.format_size
local mutation_queue = pi.module.require("pi.tools.file-mutation-queue", "1")
local with_file_mutation_queue = mutation_queue.with_file_mutation_queue

local EXTENSION_SETTINGS_KEY = "morph"
local DEFAULT_BASE_URL = "https://ai-gateway.vercel.sh/v1"
local DEFAULT_MODEL = "morph/morph-v3-large"
local DEFAULT_API_KEY_PROVIDER = "vercel-ai-gateway"
local EXISTING_CODE_MARKER = "// ... existing code ..."
local DEFAULT_MAX_FILE_BYTES = 2 * 1024 * 1024
local DEFAULT_MAX_OUTPUT_BYTES = 2 * 1024 * 1024
local FULL_REPLACEMENT_LINE_LIMIT = 10
local REQUEST_TIMEOUT_MS = 60000
local MORPH_ROUTING_HINT_HEADER = "Pi Morph routing:"

local DEFAULT_SETTINGS = {
  enabled = true,
  model = DEFAULT_MODEL,
  baseUrl = DEFAULT_BASE_URL,
  apiKeyProvider = DEFAULT_API_KEY_PROVIDER,
  maxFileBytes = DEFAULT_MAX_FILE_BYTES,
  maxOutputBytes = DEFAULT_MAX_OUTPUT_BYTES,
  allowFullReplacement = false,
  showStatus = true,
}

local function is_table(value)
  return type(value) == "table"
end

local function parse_positive_integer(value)
  if type(value) ~= "number" then return nil end
  local rounded = math.floor(value)
  return (rounded > 0 and value == value and value <= math.huge) and rounded or nil
end

local function parse_settings(raw)
  if type(raw) == "boolean" then return { enabled = raw } end
  if type(raw) ~= "table" then return {} end
  local function is_array(t)
    for _ in ipairs(t) do return true end
    return false
  end
  local out = {}
  if type(raw.enabled) == "boolean" then out.enabled = raw.enabled end
  if type(raw.model) == "string" and raw.model:gsub("%s", "") ~= "" then out.model = raw.model:gsub("^%s*(.-)%s*$", "%1") end
  if type(raw.baseUrl) == "string" then
    local base = raw.baseUrl:gsub("^%s*(.-)%s*$", "%1"):gsub("/+$", "")
    if base ~= "" then out.baseUrl = base end
  end
  if type(raw.apiKeyProvider) == "string" and raw.apiKeyProvider:gsub("%s", "") ~= "" then
    out.apiKeyProvider = raw.apiKeyProvider:gsub("^%s*(.-)%s*$", "%1")
  end
  if type(raw.allowFullReplacement) == "boolean" then out.allowFullReplacement = raw.allowFullReplacement end
  if type(raw.showStatus) == "boolean" then out.showStatus = raw.showStatus end
  local maxFileBytes = parse_positive_integer(raw.maxFileBytes)
  if maxFileBytes ~= nil then out.maxFileBytes = maxFileBytes end
  local maxOutputBytes = parse_positive_integer(raw.maxOutputBytes)
  if maxOutputBytes ~= nil then out.maxOutputBytes = maxOutputBytes end
  if is_table(raw.provider) and not is_array(raw.provider) then out.provider = raw.provider end
  if is_table(raw.providerOptions) and not is_array(raw.providerOptions) then out.providerOptions = raw.providerOptions end
  return out
end

local function pick_settings(parsed)
  local extensionSettings = parsed.extensionSettings
  if type(extensionSettings) ~= "table" then return nil end
  return extensionSettings[EXTENSION_SETTINGS_KEY] or extensionSettings["pi-morph"]
end

local function read_settings_file(path)
  if not pi.fs.exists(path) then return {} end
  local ok, parsed = pcall(pi.json.decode, pi.fs.read_file(path))
  if not ok then return {} end
  if type(parsed) ~= "table" then return {} end
  return parse_settings(pick_settings(parsed))
end

local function agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE or "."
  return pi.path.join(home, ".pi", "agent")
end

local function merge(base, extra)
  local out = {}
  for k, v in pairs(base) do out[k] = v end
  for k, v in pairs(extra) do out[k] = v end
  return out
end

local function load_settings(cwd)
  return merge(merge(DEFAULT_SETTINGS, read_settings_file(pi.path.join(agent_dir(), "settings.json"))),
    read_settings_file(pi.path.join(cwd, ".pi", "settings.json")))
end

-- ---- paths ----
local function expand_path(file_path)
  local normalized = file_path:sub(1, 1) == "@" and file_path:sub(2) or file_path
  if normalized == "~" then return pi.env.HOME or normalized end
  if normalized:sub(1, 2) == "~/" then return (pi.env.HOME or "~") .. normalized:sub(2) end
  return normalized
end

local function resolve_to_cwd(file_path, cwd)
  local expanded = expand_path(file_path)
  return pi.path.is_absolute(expanded) and expanded or pi.path.resolve(cwd, expanded)
end

-- ---- utils ----
local function throw_if_aborted(signal)
  if signal and signal:is_aborted() then error("Aborted", 0) end
end

-- ---- api ----
local function resolve_api_key(ctx, settings)
  local registry = ctx.modelRegistry
  if registry and registry.getApiKeyForProvider then
    local ok, key = pcall(registry.getApiKeyForProvider, settings.apiKeyProvider)
    if ok and key then return key end
  end
  if settings.apiKeyProvider == DEFAULT_API_KEY_PROVIDER then
    return pi.env.AI_GATEWAY_API_KEY
  end
  return pi.env[settings.apiKeyProvider]
end

local function build_prompt(filepath, original_code, code_edit, instructions)
  return table.concat({
    "<filepath>" .. filepath .. "</filepath>",
    "",
    "<code>",
    original_code,
    "</code>",
    "",
    "<update>",
    code_edit,
    "</update>",
    "",
    "<instruction>",
    instructions,
    "",
    "Merge the update into the original file.",
    "Return only the complete merged file content.",
    "Do not return markdown fences, XML tags, explanations, or a diff.",
    "Preserve existing style and indentation.",
    "</instruction>",
  }, "\n")
end

local function extract_assistant_text(payload)
  if type(payload) ~= "table" then error("AI Gateway returned a non-object response.", 0) end
  local choices = payload.choices
  if type(choices) ~= "table" or #choices == 0 then error("AI Gateway returned no choices.", 0) end
  local first = choices[1]
  if type(first) ~= "table" then error("AI Gateway returned a malformed choice.", 0) end
  local message = first.message
  if type(message) ~= "table" then error("AI Gateway returned a choice without a message.", 0) end
  local content = message.content
  if type(content) == "string" then return content end
  if type(content) == "table" then
    local parts = {}
    for _, part in ipairs(content) do
      if type(part) == "table" and type(part.text) == "string" then parts[#parts + 1] = part.text end
    end
    return table.concat(parts)
  end
  error("AI Gateway returned a message without text content.", 0)
end

local function strip_outer_code_fence(text)
  local trimmed = text:gsub("^%s*(.-)%s*$", "%1")
  local start, last = trimmed:find("\n[^\n]*$")
  local lines = {}
  for line in (trimmed .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = line end
  if #lines < 3 then return text end
  local first_line = lines[1]
  local last_line = lines[#lines]
  if first_line and last_line and first_line:match("^```[%w-]*$") and last_line == "```" then
    return table.concat({ table.unpack(lines, 2, #lines - 1) }, "\n")
  end
  return text
end

local function call_ai_gateway(settings, api_key, prompt, signal)
  throw_if_aborted(signal)
  local body = {
    model = settings.model,
    messages = { { role = "user", content = prompt } },
    stream = false,
  }
  if settings.provider ~= nil then body.provider = settings.provider end
  if settings.providerOptions ~= nil then body.providerOptions = settings.providerOptions end

  local headers = {
    Authorization = "Bearer " .. api_key,
    ["Content-Type"] = "application/json",
  }
  local response = pi.http.fetch(settings.baseUrl .. "/chat/completions", {
    method = "POST",
    headers = headers,
    body = pi.json.encode(body),
    timeout_ms = REQUEST_TIMEOUT_MS,
  })
  local text = response.body or ""
  if not response.ok then
    error(("AI Gateway request failed (%d %s): %s"):format(
      response.status, tostring(response.statusText or ""), text:sub(1, 1000)), 0)
  end
  local ok, json = pcall(pi.json.decode, text)
  if not ok then
    error("AI Gateway returned invalid JSON: " .. text:sub(1, 500), 0)
  end
  return strip_outer_code_fence(extract_assistant_text(json))
end


-- ---- text ----
local function normalize_code_edit_input(code_edit)
  local trimmed = code_edit:gsub("^%s*(.-)%s*$", "%1")
  local lines = {}
  for line in (trimmed .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = line end
  if #lines < 3 then return code_edit end
  local first_line = lines[1]
  local last_line = lines[#lines]
  if first_line and last_line and first_line:match("^```[%w-]*$") and last_line == "```" then
    local inner = {}
    for i = 2, #lines - 1 do inner[#inner + 1] = lines[i] end
    return table.concat(inner, "\n")
  end
  return code_edit
end

local function detect_line_ending(text)
  local crlf = 0
  for _ in text:gmatch("\r\n") do crlf = crlf + 1 end
  local lf = 0
  for _ in text:gmatch("(^|[^\r])\n") do lf = lf + 1 end
  if text:sub(1, 1) ~= "\r" then
    -- Count standalone \n not preceded by \r.
  end
  lf = 0
  local pos = 1
  while true do
    local idx = text:find("\n", pos, true)
    if not idx then break end
    if idx == 1 or text:sub(idx - 1, idx - 1) ~= "\r" then lf = lf + 1 end
    pos = idx + 1
  end
  return (crlf > lf) and "\r\n" or "\n"
end

local function normalize_line_endings(text, eol)
  local lf = text:gsub("\r\n", "\n")
  if eol == "\n" then return lf end
  return lf:gsub("\n", "\r\n")
end

local function byte_length(text)
  return pi.buffer.byte_length(text)
end

local function split_lines(text)
  if text == "" then return {} end
  local lines = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = line end
  return lines
end

local function summarize_change(original, merged)
  local orig = original
  if orig == merged then
    local line_count = #split_lines(orig)
    return { text = "No changes detected.", changed = false, oldLines = line_count, newLines = line_count }
  end
  local old_lines = split_lines(orig)
  local new_lines = split_lines(merged)
  local prefix = 0
  while prefix < #old_lines and prefix < #new_lines and old_lines[prefix + 1] == new_lines[prefix + 1] do
    prefix = prefix + 1
  end
  local old_suffix = #old_lines - 1
  local new_suffix = #new_lines - 1
  while old_suffix >= prefix and new_suffix >= prefix and old_lines[old_suffix + 1] == new_lines[new_suffix + 1] do
    old_suffix = old_suffix - 1
    new_suffix = new_suffix - 1
  end
  local removed = math.max(0, old_suffix - prefix + 1)
  local added = math.max(0, new_suffix - prefix + 1)
  local start_line = prefix + 1

  local old_preview = {}
  local new_preview = {}
  local old_limit = math.min(old_suffix + 1, prefix + 12)
  for i = prefix + 1, old_limit do old_preview[#old_preview + 1] = old_lines[i] end
  local new_limit = math.min(new_suffix + 1, prefix + 12)
  for i = prefix + 1, new_limit do new_preview[#new_preview + 1] = new_lines[i] end

  local parts = {
    ("Changed around line %d: +%d -%d lines in changed window"):format(start_line, added, removed),
    "",
    "```diff",
  }
  for _, line in ipairs(old_preview) do parts[#parts + 1] = "-" .. line end
  if removed > #old_preview then parts[#parts + 1] = "-... (removed preview truncated)" end
  for _, line in ipairs(new_preview) do parts[#parts + 1] = "+" .. line end
  if added > #new_preview then parts[#parts + 1] = "+... (added preview truncated)" end
  parts[#parts + 1] = "```"

  return { text = table.concat(parts, "\n"), changed = true, oldLines = #old_lines, newLines = #new_lines }
end

local function validate_merged_output(original, merged, code_edit, settings)
  local has_markers = code_edit:find(EXISTING_CODE_MARKER, 1, true) ~= nil
  local original_had_marker = original:find(EXISTING_CODE_MARKER, 1, true) ~= nil
  if has_markers and not original_had_marker and merged:find(EXISTING_CODE_MARKER, 1, true) ~= nil then
    error(('Morph output still contains %s. No file changes were written. Retry with more concrete context or use edit.'):format(EXISTING_CODE_MARKER), 0)
  end
  local output_bytes = byte_length(merged)
  if output_bytes > settings.maxOutputBytes then
    error(("Morph output is %s, over maxOutputBytes=%s. No file changes were written."):format(
      format_size(output_bytes), format_size(settings.maxOutputBytes)), 0)
  end
  if has_markers and #original > 0 then
    local original_line_count = #split_lines(original)
    local merged_line_count = #split_lines(merged)
    local char_loss = (#original - #merged) / #original
    local line_loss = (original_line_count - merged_line_count) / original_line_count
    if char_loss > 0.6 and line_loss > 0.5 then
      error(("Morph output looks destructively truncated (%d%% chars, %d%% lines lost). No file changes were written."):format(
        math.round(char_loss * 100), math.round(line_loss * 100)), 0)
    end
  end
end

-- math.round (not standard)
if not math.round then
  function math.round(x) return math.floor(x + 0.5) end
end

-- ---- apply ----
local function write_text_atomically(path, content, mode)
  local dir = pi.path.dirname(path)
  local temp_path = pi.path.join(dir, ".pi-morph-" .. pi.crypto.random_uuid() .. ".tmp")
  local ok_write = pcall(pi.fs.write_file_atomic, temp_path, content)
  if not ok_write then
    pcall(pi.fs.unlink, temp_path)
    error(mode .. " write failed", 0)
  end
  local ok_chmod
  if type(mode) == "number" then
    ok_chmod = pcall(pi.fs.chmod, temp_path, string.format("%o", mode & 0xFFF))
  end
  local ok_rename = pcall(pi.fs.rename, temp_path, path)
  if not ok_rename then
    pcall(pi.fs.unlink, temp_path)
    error("rename failed", 0)
  end
end

local function apply_morph_edit(params, settings, signal, ctx)
  local target_path = resolve_to_cwd(params.target_filepath, ctx.cwd)
  local normalized_code_edit = normalize_code_edit_input(params.code_edit)

  return with_file_mutation_queue(target_path, function()
    throw_if_aborted(signal)

    local file_stat
    do
      local ok, stat = pcall(pi.fs.stat, target_path)
      if not ok then
        error(("File not found: %s. Use write for new files; morph_edit edits existing files."):format(params.target_filepath), 0)
      end
      file_stat = stat
    end
    if file_stat.type ~= "file" then error(("Not a regular file: %s"):format(params.target_filepath), 0) end
    if file_stat.size > settings.maxFileBytes then
      error(("Refusing to send %s (%s) to Morph; maxFileBytes=%s."):format(
        params.target_filepath, format_size(file_stat.size), format_size(settings.maxFileBytes)), 0)
    end

    local original = pi.fs.read_file(target_path)
    local has_markers = normalized_code_edit:find(EXISTING_CODE_MARKER, 1, true) ~= nil
    local original_line_count = #split_lines(original)
    if not has_markers and not settings.allowFullReplacement and original_line_count > FULL_REPLACEMENT_LINE_LIMIT then
      error(("Missing %s markers. Without markers, Morph may replace the whole %d-line file. Use markers or set allowFullReplacement=true."):format(
        EXISTING_CODE_MARKER, original_line_count), 0)
    end

    local api_key = resolve_api_key(ctx, settings)
    if not api_key then
      error(("No Vercel AI Gateway API key found for provider %s. Set AI_GATEWAY_API_KEY or store a key via Pi /login for Vercel AI Gateway."):format(
        settings.apiKeyProvider), 0)
    end

    local prompt = build_prompt(params.target_filepath, original, normalized_code_edit, params.instructions)
    local eol = detect_line_ending(original)
    local merged_raw = call_ai_gateway(settings, api_key, prompt, signal)
    throw_if_aborted(signal)

    local merged = normalize_line_endings(merged_raw, eol)
    validate_merged_output(original, merged, normalized_code_edit, settings)

    local summary = summarize_change(original, merged)
    if summary.changed then
      pi.fs.mkdir(pi.path.dirname(target_path))
      write_text_atomically(target_path, merged, file_stat.mode)
    end

    local original_bytes = byte_length(original)
    local merged_bytes = byte_length(merged)
    local text = table.concat({
      (summary.changed and "Applied" or "No-op") .. " Morph edit to " .. params.target_filepath,
      ("%d → %d lines, %s → %s"):format(summary.oldLines, summary.newLines, format_size(original_bytes), format_size(merged_bytes)),
      "",
      summary.text,
    }, "\n")

    return {
      content = { { type = "text", text = text } },
      details = {
        path = target_path,
        model = settings.model,
        changed = summary.changed,
        oldLines = summary.oldLines,
        newLines = summary.newLines,
        oldBytes = original_bytes,
        newBytes = merged_bytes,
      },
    }
  end)
end

-- ---- routing ----
local function build_morph_routing_hint(settings, api_key_available)
  if not settings.enabled then
    return table.concat({
      MORPH_ROUTING_HINT_HEADER,
      "- pi-morph is disabled by extensionSettings.morph.enabled=false; do not call morph_edit.",
      "- Use edit for exact existing-file changes and write for new files/full rewrites.",
    }, "\n")
  end
  if not api_key_available then
    return table.concat({
      MORPH_ROUTING_HINT_HEADER,
      ("- morph_edit is unavailable because no API key was found for %s; do not call morph_edit unless credentials become available."):format(settings.apiKeyProvider),
      "- Use edit for exact existing-file changes and write for new files/full rewrites.",
    }, "\n")
  end
  return table.concat({
    MORPH_ROUTING_HINT_HEADER,
    "- Use morph_edit for large existing files, multiple scattered edits, whitespace-sensitive edits, repetitive changes, or ambiguous/structural rewrites inside one existing file.",
    "- Use edit for small exact anchor-based replacements, single-line/few-line changes, and deterministic patches.",
    "- Use write for new files or intentional full-file rewrites.",
    ("- morph_edit requires %s markers around unchanged sections, ideally with 1-2 unique context lines around each changed region."):format(EXISTING_CODE_MARKER),
    "- If morph_edit fails, retry with more concrete context or fall back to edit/write.",
  }, "\n")
end

local function serialize(value) return pi.json.encode(value) end

local function append_text_content(content, hint)
  if type(content) == "string" then
    if content:find(MORPH_ROUTING_HINT_HEADER, 1, true) then return content end
    return content .. "\n\n" .. hint
  end
  if type(content) == "table" then
    local is_array = false
    for _ in ipairs(content) do is_array = true break end
    if is_array then
      local serialized = pi.json.encode(content)
      if serialized:find(MORPH_ROUTING_HINT_HEADER, 1, true) then return content end
      local out = {}
      for i, v in ipairs(content) do out[i] = v end
      out[#out + 1] = { type = "text", text = hint }
      return out
    end
  end
  return content
end

local function append_morph_routing_hint(payload, hint)
  if type(payload) ~= "table" then return nil end
  if type(payload.system) == "string" or type(payload.system) == "table" then
    payload.system = append_text_content(payload.system, hint)
    return payload
  end
  local messages = payload.messages
  if type(messages) ~= "table" then return nil end
  local system_message
  for _, message in ipairs(messages) do
    if type(message) == "table" and message.role == "system" then system_message = message break end
  end
  if type(system_message) == "table" then
    system_message.content = append_text_content(system_message.content, hint)
    return payload
  end
  table.insert(messages, 1, { role = "system", content = hint })
  return payload
end

-- ---- status ----
local function update_status(ctx)
  local settings = load_settings(ctx.cwd)
  if not settings.showStatus then
    ctx.ui.setStatus("morph", nil)
    return
  end
  if not settings.enabled then
    ctx.ui.setStatus("morph", ctx.ui.theme:fg("dim", "morph:off"))
    return
  end
  local ok, key = pcall(resolve_api_key, ctx, settings)
  local has_key = ok and key ~= nil and key ~= ""
  ctx.ui.setStatus("morph", has_key and ctx.ui.theme:fg("accent", "morph") or ctx.ui.theme:fg("warning", "morph:no-key"))
end

-- ---- index ----
local morph_edit_schema = {
  type = "object",
  properties = {
    target_filepath = { type = "string", description = "Path of the existing file to modify" },
    instructions = { type = "string", description = "Brief first-person description of the intended edit, e.g. 'I am adding request logging to the middleware setup.'" },
    code_edit = { type = "string", description = "Partial code edit using " .. EXISTING_CODE_MARKER .. " markers for unchanged sections. Include unique context around each changed region." },
  },
  required = { "target_filepath", "instructions", "code_edit" },
  additionalProperties = false,
}

pi.on("session_start", function(_event, ctx)
  update_status(ctx)
end)

pi.on("model_select", function(_event, ctx)
  update_status(ctx)
end)

pi.on("before_provider_request", function(event, ctx)
  local settings = load_settings(ctx.cwd)
  local ok, key = pcall(resolve_api_key, ctx, settings)
  local has_key = ok and key ~= nil and key ~= ""
  return append_morph_routing_hint(event.payload, build_morph_routing_hint(settings, has_key))
end)

pi.register_command("morph-status", {
  description = "Show pi-morph configuration and Vercel AI Gateway key status",
  handler = function(_args, ctx)
    local settings = load_settings(ctx.cwd)
    local ok, key = pcall(resolve_api_key, ctx, settings)
    local has_key = ok and key ~= nil and key ~= ""
    local lines = {
      "pi-morph: " .. (settings.enabled and "enabled" or "disabled"),
      "model: " .. settings.model,
      "baseUrl: " .. settings.baseUrl,
      "apiKeyProvider: " .. settings.apiKeyProvider,
      "key: " .. (has_key and "available" or "missing"),
      "maxFileBytes: " .. format_size(settings.maxFileBytes),
      "maxOutputBytes: " .. format_size(settings.maxOutputBytes),
      "allowFullReplacement: " .. tostring(settings.allowFullReplacement),
      "config: ~/.pi/agent/settings.json#extensionSettings.morph, .pi/settings.json#extensionSettings.morph",
    }
    ctx.ui.notify(table.concat(lines, "\n"), (has_key and settings.enabled) and "info" or "warning")
    update_status(ctx)
  end,
})

pi.register_tool({
  name = "morph_edit",
  label = "Morph Edit",
  description = table.concat({
    ("Edit an existing UTF-8 file using Morph via Vercel AI Gateway (%s by default)."):format(DEFAULT_MODEL),
    ("Provide a partial code_edit with %s markers for unchanged sections; Morph merges it into the full file."):format(EXISTING_CODE_MARKER),
    "Best for large files, multiple scattered changes, repetitive structures, or ambiguous exact replacements.",
    "Use Pi's regular edit for small exact changes and write for new files.",
    "The tool validates marker leakage, destructive truncation, and configured output size before writing.",
    "Credentials use Pi's normal Vercel AI Gateway provider lookup (AI_GATEWAY_API_KEY or auth.json provider vercel-ai-gateway).",
  }, "\n"),
  promptSnippet = "Merge partial code edits into existing files via Morph on Vercel AI Gateway",
  promptGuidelines = {
    "Use morph_edit for large, scattered, whitespace-sensitive, repetitive, or ambiguous edits inside an existing file.",
    "Use morph_edit with code_edit wrapped by // ... existing code ... markers at both start and end so unchanged code is preserved.",
    "Use morph_edit with 1-2 unique context lines around each edited region to disambiguate repeated patterns.",
    "Use regular edit for small exact replacements and write for new files instead of morph_edit.",
    "If morph_edit fails, retry with more concrete context or fall back to regular edit.",
  },
  parameters = morph_edit_schema,

  renderCall = function(args, theme, _context)
    local text = theme:fg("toolTitle", theme:bold("morph_edit")) .. " " .. theme:fg("accent", args.target_filepath or "...")
    return { text = text }
  end,

  renderResult = function(result, options, theme, _context)
    if options and options.isPartial then
      return { text = theme:fg("warning", "Morph merging...") }
    end
    local body = ""
    if result and result.content then
      local parts = {}
      for _, entry in ipairs(result.content) do
        if entry and entry.type == "text" and entry.text then parts[#parts + 1] = entry.text end
      end
      body = table.concat(parts, "\n")
    end
    return { text = body }
  end,

  execute = function(_tool_call_id, params, signal, on_update, ctx)
    local settings = load_settings(ctx.cwd)
    if not settings.enabled then
      error("pi-morph is disabled by extensionSettings.morph.enabled=false.", 0)
    end
    if on_update then
      on_update({ content = { { type = "text", text = "Morph merging " .. params.target_filepath .. "..." } } })
    end
    return apply_morph_edit(params, settings, signal, ctx)
  end,
})
