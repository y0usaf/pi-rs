-- File-backed pi-context-janitor translation (dogfood package).
-- Public surface only: pi.register_message_renderer, pi.appendEntry,
-- pi.sendMessage, pi.register_command, pi.on, pi.ai.complete (the
-- completeSimple streaming-LLM helper), ctx.modelRegistry.{find,getAvailable,
-- getApiKeyForProvider}, ctx.sessionManager.getBranch, ctx.hasUI,
-- ctx.ui.{setStatus,notify,custom}, pi.crypto.sha256, pi.json.{decode,encode},
-- pi.fs.{exists,read_file,mkdir,write_file_atomic}, pi.path.join, pi.env,
-- pi.set_timeout/clear_timeout + pi.set_interval/clear_interval for the status
-- spinner. Cleanup: pending timer + spinner interval cleared, abort handler
-- dropped, and status cleared on session_shutdown / session replacement.
local pi = ...

-- ── Constants ──────────────────────────────────────────────────────
local INDEX_CUSTOM_TYPE = "context-janitor-index"
local RESTORE_CUSTOM_TYPE = "context-janitor-restore"
local SUMMARY_CUSTOM_TYPE = "context-janitor-summary"
local NOTICE_CUSTOM_TYPE = "context-janitor-notice"
local STATUS_KEY = "context-janitor"
local CONTEXT_HIDDEN_TEXT = "\u{200B}"
local JANITOR_CUSTOM_TYPES = {}
for _, t in ipairs({ INDEX_CUSTOM_TYPE, RESTORE_CUSTOM_TYPE, SUMMARY_CUSTOM_TYPE, NOTICE_CUSTOM_TYPE }) do
  JANITOR_CUSTOM_TYPES[t] = true
end
local DEBOUNCE_MS = 900
local HYSTERESIS_MIN_TOOL_CALLS = 6
local HYSTERESIS_MIN_RAW_CHARS = 16000
local HYSTERESIS_MAX_AGE_MS = 60000
local HYSTERESIS_RECHECK_MS = 5000
local MAX_DECIDER_INPUT_CHARS = 60000
local MAX_RECORDS_PER_PASS = 24
local MAX_DECIDER_TOKENS = 1000
local STATUS_ENABLED_IDLE = "janitor ⣿"
local STATUS_DISABLED = "janitor"
local STATUS_SPINNER_FRAMES = { "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏" }
local STATUS_SPINNER_MS = 120

local DECIDER_SYSTEM_PROMPT = "You are Context Janitor, a conservative background context cleaner for a coding agent.\n\nYou receive JSON objects representing completed tool results. Each object has an id and a hash. Decide which tool-result outputs are safe to replace with a hidden placeholder in future model context.\n\nOutput JSON only:\n{\"actions\":[{\"target\":{\"id\":\"...\",\"hash\":\"...\"},\"action\":\"truncate|keep\",\"reason\":\"...\"}]}\n\nPolicy:\n- Truncate only operational clutter: duplicate/noisy output, progress logs, stale failed attempts that were corrected, typo commands, irrelevant exploration, or huge output with no durable fact.\n- Keep unresolved errors, the latest test/build/lint result, file contents/snippets likely needed, command outputs with side effects, permission/network failures, and anything uncertain.\n- Be conservative. If unsure, keep.\n- Never invent ids or hashes. Use only the provided id/hash pairs."

local DEFAULT_SETTINGS = { enabled = true }

local AUTO_MODEL_CANDIDATES = {
  { provider = "openai", modelId = "gpt-5.4-mini" },
  { provider = "anthropic", modelId = "claude-haiku-4-5" },
  { provider = "vercel-ai-gateway", modelId = "openai/gpt-5-nano" },
}

-- ── Settings dir/path ──────────────────────────────────────────────
local function get_agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE
  if home then return pi.path.join(home, ".pi", "agent") end
  return pi.path.join(".pi", "agent")
end
local SETTINGS_DIR = pi.path.join(get_agent_dir(), "context-janitor")
local SETTINGS_PATH = pi.path.join(SETTINGS_DIR, "settings.json")

-- ── Utils ──────────────────────────────────────────────────────────
local function is_record(value)
  if type(value) ~= "table" then return false end
  -- A record is a non-array object (hash part or mixed); JSON arrays decode to
  -- integer-keyed sequences, so treat those as non-records.
  local is_array = next(value) == nil and false
  if not is_array then
    is_array = true
    for k in pairs(value) do
      if type(k) ~= "number" or k < 1 or math.floor(k) ~= k then is_array = false; break end
    end
  end
  return not is_array
end

local function text_from_content(content)
  if type(content) == "string" then return content end
  if type(content) ~= "table" then return "" end
  local parts = {}
  for i = 1, #content do
    local part = content[i]
    if type(part) == "table" then
      if part.type == "text" and type(part.text) == "string" then parts[#parts + 1] = part.text
      elseif part.type == "image" then parts[#parts + 1] = "[image]"
      elseif part.type == "thinking" and type(part.thinking) == "string" then parts[#parts + 1] = part.thinking
      elseif part.type == "toolCall" then parts[#parts + 1] = "[toolCall " .. tostring(part.name or "") .. "]" end
    end
  end
  return table.concat(parts, "\n")
end

local function truncate_middle(text, max_chars)
  if #text <= max_chars then return text end
  if max_chars <= 1 then return string.sub("…", 1, math.max(0, max_chars)) end
  local marker = "\n...[truncated " .. (#text - max_chars) .. " chars]...\n"
  if #marker >= max_chars then return string.sub(text, 1, max_chars - 1) .. "…" end
  local head = math.max(0, math.floor((max_chars - #marker) * 0.58))
  local tail = math.max(0, max_chars - #marker - head)
  return string.sub(text, 1, head) .. marker .. string.sub(text, #text - tail + 1)
end

local function safe_json(value, max_chars)
  max_chars = max_chars or 4000
  local ok, text = pcall(pi.json.encode, value, true)
  if not ok then text = tostring(value) end
  return truncate_middle(text, max_chars)
end

local function stable_json(value)
  if value == nil then return "null" end
  if type(value) ~= "table" then return tostring(value) end
  if next(value) == nil then
    return "[]"
  end
  -- Detect array: integer ordered keys.
  local is_array = true
  local count = 0
  for k in pairs(value) do
    count = count + 1
    if type(k) ~= "number" or k < 1 or math.floor(k) ~= k then is_array = false end
  end
  if is_array then
    local parts = {}
    for i = 1, count do parts[i] = stable_json(value[i]) end
    return "[" .. table.concat(parts, ",") .. "]"
  else
    local keys = {}
    for k in pairs(value) do keys[#keys + 1] = tostring(k) end
    table.sort(keys)
    local parts = {}
    for _, k in ipairs(keys) do
      parts[#parts + 1] = pi.json.encode(k) .. ":" .. stable_json(value[k])
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end
end

local function format_count(value)
  if value >= 1000000 then return string.format("%.1fM", value / 1000000) end
  if value >= 1000 then return string.format("%.1fk", value / 1000) end
  return tostring(value)
end

local function format_chars(value) return format_count(value) .. "ch" end

-- ── Settings I/O ───────────────────────────────────────────────────
local function parse_settings(raw)
  if type(raw) == "boolean" then return { enabled = raw } end
  if type(raw) ~= "table" then return {} end
  if type(raw.enabled) == "boolean" then return { enabled = raw.enabled } end
  return {}
end

local function merge_settings(base, extra)
  local out = {}
  for k, v in pairs(base) do out[k] = v end
  for k, v in pairs(extra) do out[k] = v end
  return out
end

local function load_settings()
  if not pi.fs.exists(SETTINGS_PATH) then return { settings = merge_settings(DEFAULT_SETTINGS, {}) } end
  local ok, raw = pcall(pi.fs.read_file, SETTINGS_PATH)
  if not ok then
    return { settings = merge_settings(DEFAULT_SETTINGS, {}), error = "Failed to read " .. SETTINGS_PATH .. ": " .. tostring(raw) }
  end
  local ok2, parsed = pcall(pi.json.decode, raw)
  if not ok2 then
    return { settings = merge_settings(DEFAULT_SETTINGS, {}), error = "Failed to parse " .. SETTINGS_PATH .. ": " .. tostring(parsed) }
  end
  return { settings = merge_settings(DEFAULT_SETTINGS, parse_settings(parsed)) }
end

local function save_settings(settings)
  pi.fs.mkdir(SETTINGS_DIR, true)
  pi.fs.write_file_atomic(SETTINGS_PATH, pi.json.encode(settings, true) .. "\n")
end

-- ── Capture ────────────────────────────────────────────────────────
local function assistant_tool_args(message)
  local out = {}
  if type(message) ~= "table" or message.role ~= "assistant" or type(message.content) ~= "table" then return out end
  for i = 1, #message.content do
    local part = message.content[i]
    if type(part) == "table" and part.type == "toolCall" and type(part.id) == "string" then
      out[part.id] = part.arguments
    end
  end
  return out
end

local function capture_batch(turn_index, message, tool_results, indexed)
  if type(tool_results) ~= "table" or #tool_results == 0 then return nil end
  local args_by_id = assistant_tool_args(message)
  local tool_calls = {}
  local raw_chars = 0
  for i = 1, #tool_results do
    local result = tool_results[i]
    if not (result and type(result) == "table" and result.toolCallId) then
      if not (result and type(result) == "table" and result.tool_call_id) then
      end
    end
    local tc_id = result and (result.toolCallId or result.tool_call_id)
    if not tc_id then
      if result and type(result) == "table" then
        -- tool results may be keyed tuples; fall back to scanning for id keys.
        if not result.message_tool_calls then
          for k, v in pairs(result) do if k == "toolCallId" or k == "tool_call_id" then tc_id = v end end
        end
      end
    end
    if tc_id and not indexed[tc_id] then
      local result_text = text_from_content(result.content)
      if result_text:gsub("%s", "") ~= "" then
        local tool_name = result.toolName or result.tool_name
        local is_error = result.isError or result.is_error
        local ts = result.timestamp
        if type(ts) ~= "number" then ts = pi.now_ms() end
        tool_calls[#tool_calls + 1] = {
          toolCallId = tc_id,
          toolName = tool_name,
          args = args_by_id[tc_id],
          resultText = result_text,
          isError = is_error == true,
          turnIndex = turn_index,
          timestamp = ts,
        }
        raw_chars = raw_chars + #result_text
      end
    end
  end
  if #tool_calls == 0 then return nil end
  return { turnIndex = turn_index, toolCalls = tool_calls, rawChars = raw_chars, capturedAt = pi.now_ms() }
end

local function pending_totals(batches)
  local tool_calls, raw_chars = 0, 0
  for _, b in ipairs(batches) do
    tool_calls = tool_calls + #b.toolCalls
    raw_chars = raw_chars + b.rawChars
  end
  return tool_calls, raw_chars
end

local function pending_hysteresis(batches)
  local tool_calls, raw_chars = pending_totals(batches)
  local now = pi.now_ms()
  local oldest = nil
  for _, b in ipairs(batches) do
    if oldest == nil or b.capturedAt < oldest then oldest = b.capturedAt end
  end
  local age = oldest and math.max(0, now - oldest) or 0
  if tool_calls >= HYSTERESIS_MIN_TOOL_CALLS then return true, DEBOUNCE_MS, "tool-count" end
  if raw_chars >= HYSTERESIS_MIN_RAW_CHARS then return true, DEBOUNCE_MS, "raw-size" end
  if age >= HYSTERESIS_MAX_AGE_MS then return true, DEBOUNCE_MS, "age" end
  local next_delay = math.max(DEBOUNCE_MS, math.min(HYSTERESIS_RECHECK_MS, HYSTERESIS_MAX_AGE_MS - age))
  return false, next_delay, "warming"
end

local function batch_from_records(records)
  if #records == 0 then return nil end
  local min_turn = nil
  local raw = 0
  for _, r in ipairs(records) do
    if min_turn == nil or r.turnIndex < min_turn then min_turn = r.turnIndex end
    raw = raw + #r.resultText
  end
  return { turnIndex = min_turn, toolCalls = records, rawChars = raw, capturedAt = pi.now_ms() }
end

-- ── Index store ────────────────────────────────────────────────────
local function make_id(prefix)
  local rand = tostring(math.random(0, 999999))
  return prefix .. "-" .. pi.now_ms() .. "-" .. rand
end

local function projection_text() return CONTEXT_HIDDEN_TEXT end

local function entry_from_run(summary_id, reason, records, result)
  local tool_calls = {}
  for _, r in ipairs(records) do
    local copy = {}
    for k, v in pairs(r) do copy[k] = v end
    copy.summaryId = summary_id
    tool_calls[#tool_calls + 1] = copy
  end
  local raw = 0
  for _, r in ipairs(tool_calls) do raw = raw + #r.resultText end
  return {
    version = 1, summaryId = summary_id, createdAt = os.date("!%Y-%m-%dT%H:%M:%SZ"),
    reason = reason, rawChars = raw,
    projectedChars = #tool_calls * #projection_text(),
    deciderModel = result.modelLabel, usage = result.usage, toolCalls = tool_calls,
  }
end

local function apply_index_entry(entry_tbl, index, entries)
  entries[entry_tbl.summaryId] = entry_tbl
  for _, record in ipairs(entry_tbl.toolCalls) do
    if record.toolCallId and type(record.resultText) == "string" then
      index[record.toolCallId] = record
    end
  end
end

local function parse_index_entry(raw)
  if type(raw) ~= "table" or raw.version ~= 1 or type(raw.summaryId) ~= "string" or type(raw.toolCalls) ~= "table" then return nil end
  local tool_calls = {}
  for _, item in ipairs(raw.toolCalls) do
    if type(item) == "table" and type(item.toolCallId) == "string" and type(item.toolName) == "string" and type(item.resultText) == "string" then
      tool_calls[#tool_calls + 1] = {
        toolCallId = item.toolCallId, toolName = item.toolName, args = item.args,
        resultText = item.resultText, isError = item.isError == true,
        turnIndex = type(item.turnIndex) == "number" and item.turnIndex or 0,
        timestamp = type(item.timestamp) == "number" and item.timestamp or pi.now_ms(),
        summaryId = type(item.summaryId) == "string" and item.summaryId or raw.summaryId,
        hash = type(item.hash) == "string" and item.hash or nil,
        janitorReason = type(item.janitorReason) == "string" and item.janitorReason or nil,
      }
    end
  end
  if #tool_calls == 0 then return nil end
  for _, record in ipairs(tool_calls) do if type(record.hash) ~= "string" then return nil end end
  local raw_total = 0
  for _, r in ipairs(tool_calls) do raw_total = raw_total + #r.resultText end
  return {
    version = 1, summaryId = raw.summaryId,
    createdAt = type(raw.createdAt) == "string" and raw.createdAt or os.date("!%Y-%m-%dT%H:%M:%SZ"),
    reason = type(raw.reason) == "string" and raw.reason or "reconstruct",
    rawChars = type(raw.rawChars) == "number" and raw.rawChars or raw_total,
    projectedChars = #tool_calls * #projection_text(),
    deciderModel = type(raw.deciderModel) == "string" and raw.deciderModel or "unknown",
    usage = type(raw.usage) == "table" and raw.usage or nil, toolCalls = tool_calls,
  }
end

local function parse_restore_entry(raw)
  if type(raw) ~= "table" or raw.version ~= 1 or type(raw.restoreId) ~= "string" or type(raw.summaryIds) ~= "table" then return nil end
  local ids = {}
  local seen = {}
  for _, id in ipairs(raw.summaryIds) do
    if type(id) == "string" and id:gsub("%s", "") ~= "" then
      local trimmed = id:gsub("^%s+", ""):gsub("%s+$", "")
      if not seen[trimmed] then seen[trimmed] = true; ids[#ids + 1] = trimmed end
    end
  end
  if #ids == 0 then return nil end
  return {
    version = 1, restoreId = raw.restoreId,
    createdAt = type(raw.createdAt) == "string" and raw.createdAt or os.date("!%Y-%m-%dT%H:%M:%SZ"),
    reason = type(raw.reason) == "string" and raw.reason or "restore", summaryIds = ids,
  }
end

-- ── Decider (LLM via pi.ai.complete) ───────────────────────────────
local function hash_object(value) return pi.crypto.sha256(stable_json(value)):sub(1, 16) end

local function find_auto_candidate(ctx, provider, model_id)
  local model = ctx.modelRegistry.find(provider, model_id)
  if not model then return nil end
  local available = ctx.modelRegistry.getAvailable() or {}
  for _, item in ipairs(available) do
    if string.lower(tostring(item.provider) .. "/" .. tostring(item.id)) == string.lower(provider .. "/" .. model_id) then
      return model
    end
  end
  return nil
end

local function resolve_lightweight_model(ctx)
  local active_provider = ctx.model and ctx.model.provider
  if active_provider then
    for _, candidate in ipairs(AUTO_MODEL_CANDIDATES) do
      if candidate.provider == active_provider then
        local model = find_auto_candidate(ctx, candidate.provider, candidate.modelId)
        if model then return model end
      end
    end
  end
  for _, candidate in ipairs(AUTO_MODEL_CANDIDATES) do
    local model = find_auto_candidate(ctx, candidate.provider, candidate.modelId)
    if model then return model end
  end
  if ctx.model then return ctx.model end
  error("No lightweight janitor model is available. Configure OpenAI, Anthropic, Vercel AI Gateway, or select an active Pi model.")
end

local function decider_object(record, args_budget, output_budget)
  local object = {
    id = record.toolCallId, kind = "tool_result", toolName = record.toolName,
    status = record.isError and "error" or "ok", turnIndex = record.turnIndex,
    rawChars = #record.resultText, argsPreview = safe_json(record.args, args_budget),
    outputPreview = truncate_middle(record.resultText, output_budget),
  }
  object.hash = hash_object(object)
  return object
end

local function build_decider_input(records)
  local args_budget, output_budget = 1200, 2000
  local objects, input = {}, ""
  for attempt = 1, 8 do
    objects = {}
    for _, record in ipairs(records) do objects[#objects + 1] = decider_object(record, args_budget, output_budget) end
    input = pi.json.encode({
      instruction = "For each tool_result object, choose action=truncate only if its output is safe to replace with a hidden placeholder in future context. Otherwise choose keep.",
      actions = { "truncate", "keep" },
      objects = objects,
    }, true)
    if #input <= MAX_DECIDER_INPUT_CHARS then break end
    args_budget = math.max(160, math.floor(args_budget * 0.55))
    output_budget = math.max(240, math.floor(output_budget * 0.55))
  end
  if #input > MAX_DECIDER_INPUT_CHARS then error("Janitor decider input is too large (" .. format_chars(#input) .. ").") end
  local candidates = {}
  for _, object in ipairs(objects) do candidates[object.id] = object end
  return input, candidates
end

local function extract_json_object(text)
  local cleaned = text:gsub("^%s*```[jJ][sS][oO][nN]?%s*", ""):gsub("%s*```%s*$", ""):gsub("^%s+", ""):gsub("%s+$", "")
  local ok, parsed = pcall(pi.json.decode, cleaned)
  if ok then return parsed end
  local start = cleaned:find("{")
  local finish = cleaned:len() - (cleaned:reverse():find("}") or 0) + 1
  if start and finish and finish > start then
    local sliced = cleaned:sub(start, finish)
    local ok2, parsed2 = pcall(pi.json.decode, sliced)
    if ok2 then return parsed2 end
  end
  error("Janitor decider returned invalid JSON.")
end

local function parse_decider_actions(raw, candidates)
  if type(raw) ~= "table" or type(raw.actions) ~= "table" then error("Janitor decider JSON must contain an actions array.") end
  local out = {}
  for i = 1, #raw.actions do
    local item = raw.actions[i]
    if type(item) == "table" and type(item.target) == "table" then
      local id = type(item.target.id) == "string" and item.target.id or nil
      local hash = type(item.target.hash) == "string" and item.target.hash or nil
      local action = nil
      if item.action == "truncate" or item.action == "hide" then action = "truncate"
      elseif item.action == "keep" then action = "keep" end
      if id and hash and action then
        local candidate = candidates[id]
        if candidate and candidate.hash == hash then
          local reason = action
          if type(item.reason) == "string" and item.reason:gsub("%s", "") ~= "" then
            reason = item.reason:gsub("^%s+", ""):gsub("%s+$", ""):sub(1, 160)
          end
          out[#out + 1] = { target = { id = id, hash = hash }, action = action, reason = reason }
        end
      end
    end
  end
  return out
end

local function decide_records(ctx, records)
  local model = resolve_lightweight_model(ctx)
  local api_key = ctx.modelRegistry.getApiKeyForProvider(model.provider)
  local input, candidates = build_decider_input(records)
  local response = pi.ai.complete(model, {
    systemPrompt = DECIDER_SYSTEM_PROMPT,
    messages = { { role = "user", content = input, timestamp = pi.now_ms() } },
  }, {
    apiKey = api_key,
    maxTokens = MAX_DECIDER_TOKENS,
    temperature = 0,
  }, nil)

  local text = ""
  if type(response.content) == "table" then
    local parts = {}
    for i = 1, #response.content do
      local part = response.content[i]
      if type(part) == "table" and part.type == "text" and type(part.text) == "string" then parts[#parts + 1] = part.text end
    end
    text = table.concat(parts, "\n"):gsub("^%s+", ""):gsub("%s+$", "")
  end
  if text == "" then error("Janitor decider returned no text.") end

  local raw = extract_json_object(text)
  local actions = parse_decider_actions(raw, candidates)

  local truncate_set = {}
  for _, action in ipairs(actions) do
    if action.action == "truncate" then truncate_set[action.target.id] = action end
  end
  local selected = {}
  for _, record in ipairs(records) do
    local action = truncate_set[record.toolCallId]
    if action then
      local copy = {}
      for k, v in pairs(record) do copy[k] = v end
      copy.hash = action.target.hash
      copy.janitorReason = action.reason
      selected[#selected + 1] = copy
    end
  end
  return {
    records = selected,
    usage = response.usage,
    modelLabel = model.provider .. "/" .. model.id,
  }
end

-- ── UI notice message renderers ────────────────────────────────────
local function hidden_component()
  return { render = function() return {} end }
end

local function notice_component(content)
  return {
    render = function(_, width)
      local lines = {}
      local wrap = pi.tui.text_render(content or "", width, 1, 0)
      for _, l in ipairs(wrap) do lines[#lines + 1] = l end
      return lines
    end,
  }
end

pi.register_message_renderer(SUMMARY_CUSTOM_TYPE, function() return hidden_component() end)
pi.register_message_renderer(NOTICE_CUSTOM_TYPE, function(message)
  local content = type(message.content) == "string" and message.content or text_from_content(message.content)
  return notice_component(content)
end)

-- ── Janitor state & lifecycle ──────────────────────────────────────
local settings = {}
for k, v in pairs(DEFAULT_SETTINGS) do settings[k] = v end
local settings_error
local index = {}
local entries = {}
local restored_summary_ids = {}
local pending_batches = {}
local schedule_timer
local active_abort
local generation = 0
local last_ctx
local status_spinner
local status_spinner_index = 0
local flush_promise = nil

local function abort_background()
  if schedule_timer then
    pi.clear_timeout(schedule_timer)
    schedule_timer = nil
  end
end

local function status_text()
  if not settings.enabled then return STATUS_DISABLED end
  if flush_promise then return "janitor " .. (STATUS_SPINNER_FRAMES[status_spinner_index + 1] or STATUS_SPINNER_FRAMES[1]) end
  return STATUS_ENABLED_IDLE
end

local function stop_status_spinner()
  if status_spinner then
    pi.clear_interval(status_spinner)
    status_spinner = nil
  end
  status_spinner_index = 0
end

local function update_status(ctx)
  ctx = ctx or last_ctx
  if not ctx then return end
  last_ctx = ctx
  local spinning = settings.enabled and flush_promise ~= nil
  if spinning and not status_spinner then
    status_spinner = pi.set_interval(function()
      status_spinner_index = (status_spinner_index + 1) % #STATUS_SPINNER_FRAMES
      update_status()
    end, STATUS_SPINNER_MS)
  elseif not spinning then
    stop_status_spinner()
  end
  ctx.ui.setStatus(STATUS_KEY, status_text())
end

local function reconstruct(ctx)
  index = {}
  entries = {}
  restored_summary_ids = {}
  local seen_summaries = {}
  local branch = ctx.sessionManager.getBranch() or {}
  for i = 1, #branch do
    local entry = branch[i]
    if is_record(entry) and entry.type == "custom" then
      if entry.customType == INDEX_CUSTOM_TYPE then
        local parsed = parse_index_entry(entry.data)
        if parsed and not seen_summaries[parsed.summaryId] then
          seen_summaries[parsed.summaryId] = true
          apply_index_entry(parsed, index, entries)
        end
      elseif entry.customType == RESTORE_CUSTOM_TYPE then
        local parsed = parse_restore_entry(entry.data)
        if parsed then
          for _, sid in ipairs(parsed.summaryIds) do restored_summary_ids[sid] = true end
        end
      end
    end
  end
end

local function janitor_run_notice_text(entry)
  return "Context Janitor truncated " .. #entry.toolCalls .. " tool result(s) (" .. format_chars(entry.rawChars) .. " → " .. format_chars(entry.projectedChars) .. ")."
end

local function schedule_flush(ctx, reason)
  last_ctx = ctx
  if not settings.enabled or #pending_batches == 0 then return end
  if schedule_timer then
    pi.clear_timeout(schedule_timer)
    schedule_timer = nil
  end
  local ready, delay = pending_hysteresis(pending_batches)
  schedule_timer = pi.set_timeout(function()
    schedule_timer = nil
    local latest_ready = pending_hysteresis(pending_batches)
    if not latest_ready then
      schedule_flush(ctx, reason)
      update_status(ctx)
      return
    end
    flush_pending(ctx, reason .. ":" .. latest_ready)
  end, delay)
end

function flush_pending(ctx, reason)
  if flush_promise then return end
  local run_generation = generation
  flush_promise = true
  flush_pending_work(ctx, reason, run_generation)
  -- emulate `.finally`: schedule any follow-up pass and clear the running flag.
  flush_promise = nil
  if settings.enabled and #pending_batches > 0 then
    schedule_flush(last_ctx, "follow-up")
  end
  update_status(last_ctx)
end

local function flush_pending_work(ctx, reason, run_generation)
  if not settings.enabled or #pending_batches == 0 then return end
  local batches = pending_batches
  pending_batches = {}
  local all_records = {}
  for _, batch in ipairs(batches) do
    for _, record in ipairs(batch.toolCalls) do
      if not index[record.toolCallId] then all_records[#all_records + 1] = record end
    end
  end
  local pass = {}
  local rest = {}
  for i = 1, #all_records do
    if i <= MAX_RECORDS_PER_PASS then pass[#pass + 1] = all_records[i] else rest[#rest + 1] = all_records[i] end
  end
  local rest_batch = batch_from_records(rest)
  if rest_batch then pending_batches[#pending_batches + 1] = rest_batch end
  if #pass == 0 then return end

  update_status(ctx)
  local ok, decided = pcall(decide_records, ctx, pass)
  if not ok then
    if run_generation == generation then
      local retry = {}
      for _, r in ipairs(pass) do retry[#retry + 1] = r end
      for _, r in ipairs(rest) do retry[#retry + 1] = r end
      local rebuilt = batch_from_records(retry)
      if rebuilt then pending_batches = { rebuilt } end
    end
    if run_generation == generation and ctx.hasUI then
      ctx.ui.notify("Context Janitor failed: " .. tostring(decided), "warning")
    end
    return
  end

  if run_generation ~= generation then return end
  local selected = decided.records
  if #selected == 0 then return end

  local summary_id = make_id("cj")
  local entry = entry_from_run(summary_id, reason, selected, { usage = decided.usage, modelLabel = decided.modelLabel })
  pi.appendEntry(INDEX_CUSTOM_TYPE, entry)
  apply_index_entry(entry, index, entries)
  pi.sendMessage({
    customType = NOTICE_CUSTOM_TYPE,
    content = janitor_run_notice_text(entry),
    display = true,
    details = { summaryId = entry.summaryId, rawChars = entry.rawChars, projectedChars = entry.projectedChars, toolCalls = #entry.toolCalls },
    attribution = "agent",
  })
end

local function restore_summary_ids(ids, reason, ctx)
  local unique = {}
  local seen = {}
  for _, id in ipairs(ids) do
    local trimmed = (tostring(id)):gsub("%s", "")
    if trimmed ~= "" and not seen[trimmed] then seen[trimmed] = true; unique[#unique + 1] = trimmed end
  end
  local restorable = {}
  for _, sid in ipairs(unique) do
    if entries[sid] and not restored_summary_ids[sid] then restorable[#restorable + 1] = sid end
  end
  if #restorable == 0 then return 0 end
  local restore_entry = {
    version = 1, restoreId = make_id("cj-restore"), createdAt = os.date("!%Y-%m-%dT%H:%M:%SZ"),
    reason = reason, summaryIds = restorable,
  }
  pi.appendEntry(RESTORE_CUSTOM_TYPE, restore_entry)
  for _, sid in ipairs(restorable) do restored_summary_ids[sid] = true end
  pi.sendMessage({
    customType = NOTICE_CUSTOM_TYPE,
    content = "Context Janitor restored " .. #restorable .. " run(s). Future model context will include those raw tool outputs again.",
    display = true, details = { restoreId = restore_entry.restoreId, summaryIds = restorable }, attribution = "user",
  })
  update_status(ctx)
  return #restorable
end

local function undo_run_items()
  local items = {}
  for summary_id, entry in pairs(entries) do
    if not restored_summary_ids[summary_id] then
      items[#items + 1] = {
        summaryId = summary_id,
        label = entry.reason,
        description = format_chars(entry.rawChars) .. " of tool output",
      }
    end
  end
  table.sort(items, function(a, b) return a.summaryId < b.summaryId end)
  return items
end

local function restore_list_text()
  local items = undo_run_items()
  if #items == 0 then return "No janitor runs are currently truncated." end
  local lines = { "Restorable janitor runs:" }
  for _, item in ipairs(items) do
    lines[#lines + 1] = "- " .. item.summaryId .. ": " .. item.label .. " — " .. item.description
  end
  lines[#lines + 1] = ""
  lines[#lines + 1] = "Run /janitor undo in the interactive TUI to restore selected runs."
  return table.concat(lines, "\n")
end

local function open_undo_picker(ctx)
  last_ctx = ctx
  local items = undo_run_items()
  if #items == 0 then
    ctx.ui.notify("Context Janitor: nothing to restore.", "info")
    return
  end
  if not ctx.hasUI then
    ctx.ui.notify(restore_list_text(), "info")
    return
  end
  local r = ctx.ui.custom(function(_, theme, _, done)
    local cursor = 1
    return {
      render = function(_, width)
        local lines = { theme:bold(theme:fg("accent", "Restore janitor runs (↑↓ navigate, enter restore, esc cancel)")) }
        for i, item in ipairs(items) do
          local marker = (i == cursor) and "▸ " or "  "
          lines[#lines + 1] = theme:fg(i == cursor and "accent" or "dim", marker .. item.summaryId .. ": " .. item.label .. " — " .. item.description)
        end
        return pi.tui.text_render(table.concat(lines, "\n"), width, 1, 1)
      end,
      handle_input = function(_, data)
        if data == "up" or data == "\27[A" then
          cursor = cursor - 1
          if cursor < 1 then cursor = #items end
        elseif data == "down" or data == "\27[B" then
          cursor = cursor + 1
          if cursor > #items then cursor = 1 end
        elseif data == "enter" or data == "\r" or data == "\n" then
          done({ items[cursor].summaryId })
        elseif data == "escape" or data == "\27" then
          done(nil)
        end
      end,
      dispose = function() end,
    }
  end, { overlay = true })

  if not r or type(r) ~= "table" or #r == 0 then
    ctx.ui.notify("Context Janitor: restore cancelled/no selection.", "info")
    return
  end
  local selected = {}
  for i = 1, #r do selected[#selected + 1] = r[i] end
  local count = restore_summary_ids(selected, "user-undo", ctx)
  ctx.ui.notify(count > 0 and ("Context Janitor restored " .. count .. " run(s). Future model context will include those raw tool outputs again.") or "Context Janitor: selected run(s) were already restored.", "info")
end

pi.register_command("janitor", {
  description = "Context janitor controls: on, off, undo",
  handler = function(args, ctx)
    last_ctx = ctx
    local parts = {}
    for w in (args or ""):gmatch("%S+") do parts[#parts + 1] = w end
    local sub = parts[1] and parts[1]:lower() or ""
    local ok, err = pcall(function()
      if sub == "on" then
        settings = { enabled = true }
        save_settings(settings)
        settings_error = nil
        update_status(ctx)
        if #pending_batches > 0 then schedule_flush(ctx, "manual-on") end
        ctx.ui.notify("Context Janitor enabled.", "info")
      elseif sub == "off" then
        settings = { enabled = false }
        save_settings(settings)
        generation = generation + 1
        abort_background()
        pending_batches = {}
        settings_error = nil
        update_status(ctx)
        ctx.ui.notify("Context Janitor disabled. Raw tool outputs will remain in model context.", "info")
      elseif sub == "undo" then
        open_undo_picker(ctx)
      elseif sub == "" then
        ctx.ui.notify("Usage: /janitor on | off | undo", settings_error and "warning" or "info")
      else
        ctx.ui.notify("Usage: /janitor on | off | undo", "warning")
      end
    end)
    if not ok then
      ctx.ui.notify("Context Janitor: " .. tostring(err), "error")
    end
  end,
})

pi.on("session_start", function(_event, ctx)
  generation = generation + 1
  abort_background()
  pending_batches = {}
  last_ctx = ctx
  local loaded = load_settings()
  settings = loaded.settings
  settings_error = loaded.error
  reconstruct(ctx)
  update_status(ctx)
  if settings_error and ctx.hasUI then ctx.ui.notify(settings_error, "warning") end
end)

pi.on("session_tree", function(_event, ctx)
  generation = generation + 1
  abort_background()
  pending_batches = {}
  last_ctx = ctx
  reconstruct(ctx)
  update_status(ctx)
end)

pi.on("model_select", function(_event, ctx) update_status(ctx) end)

pi.on("turn_end", function(event, ctx)
  last_ctx = ctx
  if not settings.enabled then update_status(ctx); return end
  local batch = capture_batch(event.turnIndex, event.message, event.toolResults, index)
  if not batch then update_status(ctx); return end
  pending_batches[#pending_batches + 1] = batch
  update_status(ctx)
  schedule_flush(ctx, "turn_end")
end)

pi.on("agent_end", function(_event, ctx)
  last_ctx = ctx
  if settings.enabled and #pending_batches > 0 then schedule_flush(ctx, "agent_end") end
  update_status(ctx)
end)

pi.on("context", function(event)
  local changed = false
  local messages = {}
  for i = 1, #event.messages do
    local message = event.messages[i]
    if is_record(message) and message.role == "custom" and type(message.customType) == "string" and JANITOR_CUSTOM_TYPES[message.customType] then
      changed = true
    elseif settings.enabled and is_record(message) and message.role == "toolResult" and type(message.toolCallId) == "string" then
      local record = index[message.toolCallId]
      if record and not restored_summary_ids[record.summaryId] and entries[record.summaryId] then
        changed = true
        local copy = {}
        for k, v in pairs(message) do copy[k] = v end
        copy.details = nil
        copy.content = { { type = "text", text = CONTEXT_HIDDEN_TEXT } }
        messages[#messages + 1] = copy
      else
        messages[#messages + 1] = message
      end
    else
      messages[#messages + 1] = message
    end
  end
  if not changed then return end
  return { messages = messages }
end)

pi.on("session_shutdown", function(_event, ctx)
  generation = generation + 1
  abort_background()
  stop_status_spinner()
  ctx.ui.setStatus(STATUS_KEY, nil)
end)
