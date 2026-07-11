-- core/compaction/utils.ts + core/compaction/branch-summarization.ts —
-- the shared summarization machinery (PLAN 6.4); the compaction pipeline
-- (utils/compaction.lua, PLAN 6.5) builds on the exports. One
-- chunk-global export table keeps the concatenated pack under Lua's
-- active-local budget.
--
-- Shared fragment: included by the interactive pack after
-- utils/messages.lua (it closes over that fragment's convert_to_llm).
branch_summary_lib = (function(pi, convert_to_llm)
  -- ---- compaction.ts estimateTokens (the entry-token slice) ----

  -- JS String.length (UTF-16 code units) for the chars/4 heuristic.
  local function js_string_length(text)
    if not utf8.len(text) then return #text end
    local units = 0
    for _, code in utf8.codes(text) do
      units = units + (code >= 0x10000 and 2 or 1)
    end
    return units
  end

  local ESTIMATED_IMAGE_CHARS = 4800

  local function estimate_text_and_image_chars(content)
    if type(content) == "string" then return js_string_length(content) end
    local chars = 0
    for _, block in ipairs(content or {}) do
      if block.type == "text" and block.text then
        chars = chars + js_string_length(block.text)
      elseif block.type == "image" then
        chars = chars + ESTIMATED_IMAGE_CHARS
      end
    end
    return chars
  end

  local function estimate_tokens(message)
    local role = message.role
    if role == "user" or role == "custom" or role == "toolResult" then
      return math.ceil(estimate_text_and_image_chars(message.content) / 4)
    elseif role == "assistant" then
      local chars = 0
      for _, block in ipairs(message.content or {}) do
        if block.type == "text" then
          chars = chars + js_string_length(block.text or "")
        elseif block.type == "thinking" then
          chars = chars + js_string_length(block.thinking or "")
        elseif block.type == "toolCall" then
          chars = chars + js_string_length(block.name or "")
            + js_string_length(pi.json.encode(block.arguments or {}))
        end
      end
      return math.ceil(chars / 4)
    elseif role == "bashExecution" then
      return math.ceil((js_string_length(message.command or "")
        + js_string_length(message.output or "")) / 4)
    elseif role == "branchSummary" or role == "compactionSummary" then
      return math.ceil(js_string_length(message.summary or "") / 4)
    end
    return 0
  end

  -- ---- utils.ts file-operation tracking ----

  local function create_file_ops()
    return { read = {}, written = {}, edited = {} }
  end

  -- Set insertion order is irrelevant: computeFileLists sorts.
  local function extract_file_ops_from_message(message, file_ops)
    if message.role ~= "assistant" then return end
    if type(message.content) ~= "table" then return end
    for _, block in ipairs(message.content) do
      if type(block) == "table" and block.type == "toolCall"
         and block.arguments ~= nil and block.name ~= nil then
        local path = type(block.arguments) == "table" and block.arguments.path or nil
        if type(path) == "string" and path ~= "" then
          if block.name == "read" then file_ops.read[path] = true
          elseif block.name == "write" then file_ops.written[path] = true
          elseif block.name == "edit" then file_ops.edited[path] = true end
        end
      end
    end
  end

  local function compute_file_lists(file_ops)
    local modified = {}
    for path in pairs(file_ops.edited) do modified[path] = true end
    for path in pairs(file_ops.written) do modified[path] = true end
    local read_only, modified_files = {}, {}
    for path in pairs(file_ops.read) do
      if not modified[path] then read_only[#read_only + 1] = path end
    end
    for path in pairs(modified) do modified_files[#modified_files + 1] = path end
    table.sort(read_only)
    table.sort(modified_files)
    return read_only, modified_files
  end

  local function format_file_operations(read_files, modified_files)
    local sections = {}
    if #read_files > 0 then
      sections[#sections + 1] =
        "<read-files>\n" .. table.concat(read_files, "\n") .. "\n</read-files>"
    end
    if #modified_files > 0 then
      sections[#sections + 1] =
        "<modified-files>\n" .. table.concat(modified_files, "\n") .. "\n</modified-files>"
    end
    if #sections == 0 then return "" end
    return "\n\n" .. table.concat(sections, "\n\n")
  end

  -- ---- utils.ts message serialization ----

  local TOOL_RESULT_MAX_CHARS = 2000

  -- JS slice/length semantics run on UTF-16 units; conversation text is
  -- sliced with the same unit arithmetic.
  local function js_slice(text, max_units)
    if js_string_length(text) <= max_units then return text, false end
    local units, out = 0, {}
    for _, code in utf8.codes(text) do
      units = units + (code >= 0x10000 and 2 or 1)
      if units > max_units then break end
      out[#out + 1] = utf8.char(code)
    end
    return table.concat(out), true
  end

  local function truncate_for_summary(text, max_chars)
    local length = js_string_length(text)
    if length <= max_chars then return text end
    local kept = js_slice(text, max_chars)
    local truncated_chars = length - max_chars
    return kept .. "\n\n[... " .. truncated_chars .. " more characters truncated]"
  end

  -- JS [[OwnPropertyKeys]] iteration for Object.entries: the wire order
  -- recorded at the JSON→Lua boundary (convert.rs metatable), then any
  -- Lua-added remainder sorted — the same contract pi.json.encode replays.
  local function js_object_keys(value)
    local keys, seen = {}, {}
    local meta = getmetatable(value)
    local order = meta and meta.__pi_rs_json_key_order
    if order then
      for _, key in ipairs(order) do
        if value[key] ~= nil then
          keys[#keys + 1] = key
          seen[key] = true
        end
      end
    end
    local rest = {}
    for key in pairs(value) do
      if not seen[key] then rest[#rest + 1] = tostring(key) end
    end
    table.sort(rest)
    for _, key in ipairs(rest) do keys[#keys + 1] = key end
    return keys
  end

  local function block_texts(content, field)
    local out = {}
    for _, block in ipairs(content or {}) do
      if block.type == (field == "thinking" and "thinking" or "text") then
        out[#out + 1] = block[field] or ""
      end
    end
    return out
  end

  local function serialize_conversation(messages)
    local parts = {}
    for _, msg in ipairs(messages) do
      if msg.role == "user" then
        local content
        if type(msg.content) == "string" then
          content = msg.content
        else
          content = table.concat(block_texts(msg.content, "text"))
        end
        if content ~= "" then parts[#parts + 1] = "[User]: " .. content end
      elseif msg.role == "assistant" then
        local text_parts, thinking_parts, tool_calls = {}, {}, {}
        for _, block in ipairs(msg.content or {}) do
          if block.type == "text" then
            text_parts[#text_parts + 1] = block.text
          elseif block.type == "thinking" then
            thinking_parts[#thinking_parts + 1] = block.thinking
          elseif block.type == "toolCall" then
            local args = block.arguments or {}
            local entries = {}
            for _, key in ipairs(js_object_keys(args)) do
              entries[#entries + 1] = key .. "=" .. pi.json.encode(args[key])
            end
            tool_calls[#tool_calls + 1] = block.name .. "(" .. table.concat(entries, ", ") .. ")"
          end
        end
        if #thinking_parts > 0 then
          parts[#parts + 1] = "[Assistant thinking]: " .. table.concat(thinking_parts, "\n")
        end
        if #text_parts > 0 then
          parts[#parts + 1] = "[Assistant]: " .. table.concat(text_parts, "\n")
        end
        if #tool_calls > 0 then
          parts[#parts + 1] = "[Assistant tool calls]: " .. table.concat(tool_calls, "; ")
        end
      elseif msg.role == "toolResult" then
        local content = table.concat(block_texts(msg.content, "text"))
        if content ~= "" then
          parts[#parts + 1] = "[Tool result]: "
            .. truncate_for_summary(content, TOOL_RESULT_MAX_CHARS)
        end
      end
    end
    return table.concat(parts, "\n\n")
  end

  local SUMMARIZATION_SYSTEM_PROMPT =
    "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary."

  -- ---- branch-summarization.ts ----

  local BRANCH_SUMMARY_PREAMBLE =
    "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n"

  local BRANCH_SUMMARY_PROMPT = [[Create a structured summary of this conversation branch for context when returning later.

Use this EXACT format:

## Goal
[What was the user trying to accomplish in this branch?]

## Constraints & Preferences
- [Any constraints, preferences, or requirements mentioned]
- [Or "(none)" if none were mentioned]

## Progress
### Done
- [x] [Completed tasks/changes]

### In Progress
- [ ] [Work that was started but not finished]

### Blocked
- [Issues preventing progress, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [What should happen next to continue this work]

Keep each section concise. Preserve exact file paths, function names, and error messages.]]

  -- collectEntriesForBranchSummary over the pi.session handle.
  local function collect_entries_for_branch_summary(session_manager, old_leaf_id, target_id)
    if not old_leaf_id then return {}, nil end

    local old_path = {}
    for _, entry in ipairs(session_manager:get_branch(old_leaf_id)) do
      old_path[entry.id] = true
    end
    local target_path = session_manager:get_branch(target_id)

    local common_ancestor_id = nil
    for i = #target_path, 1, -1 do
      if old_path[target_path[i].id] then
        common_ancestor_id = target_path[i].id
        break
      end
    end

    local entries = {}
    local current = old_leaf_id
    while current and current ~= common_ancestor_id do
      local entry = session_manager:get_entry(current)
      if not entry then break end
      entries[#entries + 1] = entry
      current = entry.parentId
    end

    -- Reverse to chronological order.
    local reversed = {}
    for i = #entries, 1, -1 do reversed[#reversed + 1] = entries[i] end
    return reversed, common_ancestor_id
  end

  -- messages.ts createCustomMessage / createBranchSummaryMessage /
  -- createCompactionSummaryMessage — entry timestamps parse with JS
  -- Date semantics (NaN → nil, like JSON.stringify(NaN) → null).
  local function iso_ms(timestamp)
    return pi.session.parse_iso_ms(timestamp or "")
  end

  -- getMessageFromEntry.
  local function get_message_from_entry(entry)
    if entry.type == "message" then
      if entry.message.role == "toolResult" then return nil end
      return entry.message
    elseif entry.type == "custom_message" then
      local message = {
        role = "custom",
        customType = entry.customType,
        content = entry.content,
        display = entry.display,
      }
      if entry.details ~= nil then message.details = entry.details end
      message.timestamp = iso_ms(entry.timestamp)
      return message
    elseif entry.type == "branch_summary" then
      return { role = "branchSummary", summary = entry.summary,
        fromId = entry.fromId, timestamp = iso_ms(entry.timestamp) }
    elseif entry.type == "compaction" then
      return { role = "compactionSummary", summary = entry.summary,
        tokensBefore = entry.tokensBefore, timestamp = iso_ms(entry.timestamp) }
    end
    return nil
  end

  -- prepareBranchEntries.
  local function prepare_branch_entries(entries, token_budget)
    token_budget = token_budget or 0
    local messages = {}
    local file_ops = create_file_ops()
    local total_tokens = 0

    -- First pass: cumulative file ops from pi-generated summaries.
    for _, entry in ipairs(entries) do
      if entry.type == "branch_summary" and not entry.fromHook
         and type(entry.details) == "table" then
        local details = entry.details
        if type(details.readFiles) == "table" then
          for _, path in ipairs(details.readFiles) do file_ops.read[path] = true end
        end
        if type(details.modifiedFiles) == "table" then
          for _, path in ipairs(details.modifiedFiles) do file_ops.edited[path] = true end
        end
      end
    end

    -- Second pass: newest to oldest until the token budget.
    for i = #entries, 1, -1 do
      local entry = entries[i]
      local message = get_message_from_entry(entry)
      if message then
        extract_file_ops_from_message(message, file_ops)
        local tokens = estimate_tokens(message)
        if token_budget > 0 and total_tokens + tokens > token_budget then
          if entry.type == "compaction" or entry.type == "branch_summary" then
            if total_tokens < token_budget * 0.9 then
              table.insert(messages, 1, message)
              total_tokens = total_tokens + tokens
            end
          end
          break
        end
        table.insert(messages, 1, message)
        total_tokens = total_tokens + tokens
      end
    end

    return messages, file_ops, total_tokens
  end

  -- generateBranchSummary — async (awaits pi.ai.stream_simple, the
  -- session streamFn seam: apiKey resolved per the current model's
  -- provider like every product request).
  local function generate_branch_summary(entries, options)
    local reserve_tokens = options.reserveTokens or 16384
    local context_window = options.model.contextWindow or 128000
    if context_window == 0 then context_window = 128000 end
    local token_budget = context_window - reserve_tokens

    local messages, file_ops = prepare_branch_entries(entries, token_budget)
    if #messages == 0 then
      return { summary = "No content to summarize" }
    end

    local llm_messages = convert_to_llm(messages)
    local conversation_text = serialize_conversation(llm_messages)

    local instructions
    if options.replaceInstructions and options.customInstructions then
      instructions = options.customInstructions
    elseif options.customInstructions then
      instructions = BRANCH_SUMMARY_PROMPT .. "\n\nAdditional focus: " .. options.customInstructions
    else
      instructions = BRANCH_SUMMARY_PROMPT
    end
    local prompt_text = "<conversation>\n" .. conversation_text .. "\n</conversation>\n\n"
      .. instructions

    local response = pi.ai.stream_simple(options.model, {
      systemPrompt = SUMMARIZATION_SYSTEM_PROMPT,
      messages = { { role = "user",
        content = { { type = "text", text = prompt_text } },
        timestamp = options.now_ms and options.now_ms() or (os.time() * 1000) } },
    }, {
      apiKey = options.apiKey,
      signal = options.signal,
      maxTokens = 2048,
    }, options.on_event or function() end)

    if response.stopReason == "aborted" then return { aborted = true } end
    if response.stopReason == "error" then
      return { error = response.errorMessage or "Summarization failed" }
    end

    local texts = {}
    for _, block in ipairs(response.content or {}) do
      if block.type == "text" then texts[#texts + 1] = block.text end
    end
    local summary = BRANCH_SUMMARY_PREAMBLE .. table.concat(texts, "\n")

    local read_files, modified_files = compute_file_lists(file_ops)
    summary = summary .. format_file_operations(read_files, modified_files)

    if summary == "" then summary = "No summary generated" end
    return { summary = summary, readFiles = read_files, modifiedFiles = modified_files }
  end

  return {
    collect_entries_for_branch_summary = collect_entries_for_branch_summary,
    prepare_branch_entries = prepare_branch_entries,
    generate_branch_summary = generate_branch_summary,
    serialize_conversation = serialize_conversation,
    -- Shared with the compaction port (utils/compaction.lua, PLAN 6.5).
    estimate_tokens = estimate_tokens,
    create_file_ops = create_file_ops,
    extract_file_ops_from_message = extract_file_ops_from_message,
    compute_file_lists = compute_file_lists,
    format_file_operations = format_file_operations,
    iso_ms = iso_ms,
    SUMMARIZATION_SYSTEM_PROMPT = SUMMARIZATION_SYSTEM_PROMPT,
  }
end)(pi, convert_to_llm)
