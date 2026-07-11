-- core/compaction/compaction.ts + pi-ai utils/overflow.ts — the
-- compaction pipeline (PLAN 6.5): token estimation over usage, cut-point
-- detection, preparation, LLM summarization, and the context-overflow
-- detector the auto-compaction check consumes. Pure policy: the session
-- entry mechanics are `pi.session.*`, the LLM call is
-- `pi.ai.stream_simple`, and the JS-regex overflow patterns run on the
-- `pi.tui.js_regex_search` mechanism (`new RegExp(pattern, "i")`).
--
-- Shared fragment: included after utils/branch-summary.lua (it builds on
-- that fragment's exports) and utils/messages.lua (convert_to_llm).
compaction_lib = (function(pi, bs, convert_to_llm)
  -- ---- DEFAULT_COMPACTION_SETTINGS ----
  local DEFAULT_COMPACTION_SETTINGS =
    { enabled = true, reserveTokens = 16384, keepRecentTokens = 20000 }

  -- ---- token calculation ----

  -- calculateContextTokens: `usage.totalTokens ||` — JS falsy 0 falls back.
  local function calculate_context_tokens(usage)
    local total = usage.totalTokens
    if total and total ~= 0 then return total end
    return (usage.input or 0) + (usage.output or 0)
      + (usage.cacheRead or 0) + (usage.cacheWrite or 0)
  end

  -- getAssistantUsage: aborted/error messages carry no valid usage.
  local function get_assistant_usage(message)
    if message.role == "assistant" and message.usage ~= nil
       and message.stopReason ~= "aborted" and message.stopReason ~= "error" then
      return message.usage
    end
    return nil
  end

  -- getLastAssistantUsage over session entries.
  local function get_last_assistant_usage(entries)
    for i = #entries, 1, -1 do
      local entry = entries[i]
      if entry.type == "message" then
        local usage = get_assistant_usage(entry.message)
        if usage then return usage end
      end
    end
    return nil
  end

  -- estimateContextTokens. `lastUsageIndex` keeps the spec's JS 0-based
  -- index (nil for the spec's null) so replays compare directly.
  local function estimate_context_tokens(messages)
    local usage_info = nil
    for i = #messages, 1, -1 do
      local usage = get_assistant_usage(messages[i])
      if usage then usage_info = { usage = usage, index = i } break end
    end

    if not usage_info then
      local estimated = 0
      for _, message in ipairs(messages) do
        estimated = estimated + bs.estimate_tokens(message)
      end
      return { tokens = estimated, usageTokens = 0,
        trailingTokens = estimated, lastUsageIndex = nil }
    end

    local usage_tokens = calculate_context_tokens(usage_info.usage)
    local trailing = 0
    for i = usage_info.index + 1, #messages do
      trailing = trailing + bs.estimate_tokens(messages[i])
    end
    return { tokens = usage_tokens + trailing, usageTokens = usage_tokens,
      trailingTokens = trailing, lastUsageIndex = usage_info.index - 1 }
  end

  -- shouldCompact.
  local function should_compact(context_tokens, context_window, settings)
    if not settings.enabled then return false end
    return context_tokens > context_window - settings.reserveTokens
  end

  -- ---- cut point detection ----

  -- getMessageFromEntry (the compaction.ts variant: message entries pass
  -- through whole, including toolResult).
  local function get_message_from_entry(entry)
    if entry.type == "message" then
      return entry.message
    elseif entry.type == "custom_message" then
      local message = {
        role = "custom",
        customType = entry.customType,
        content = entry.content,
        display = entry.display,
      }
      if entry.details ~= nil then message.details = entry.details end
      message.timestamp = bs.iso_ms(entry.timestamp)
      return message
    elseif entry.type == "branch_summary" then
      return { role = "branchSummary", summary = entry.summary,
        fromId = entry.fromId, timestamp = bs.iso_ms(entry.timestamp) }
    elseif entry.type == "compaction" then
      return { role = "compactionSummary", summary = entry.summary,
        tokensBefore = entry.tokensBefore, timestamp = bs.iso_ms(entry.timestamp) }
    end
    return nil
  end

  local function get_message_from_entry_for_compaction(entry)
    if entry.type == "compaction" then return nil end
    return get_message_from_entry(entry)
  end

  local CUT_POINT_ROLES = {
    bashExecution = true, custom = true, branchSummary = true,
    compactionSummary = true, user = true, assistant = true,
  }

  -- findValidCutPoints over [start_index, end_index) — 1-based, exclusive
  -- end, mirroring the spec's index arithmetic shifted by one.
  local function find_valid_cut_points(entries, start_index, end_index)
    local cut_points = {}
    for i = start_index, end_index - 1 do
      local entry = entries[i]
      if entry.type == "message" then
        if CUT_POINT_ROLES[entry.message.role] then
          cut_points[#cut_points + 1] = i
        end
      end
      -- branch_summary and custom_message are user-role messages, valid
      -- cut points.
      if entry.type == "branch_summary" or entry.type == "custom_message" then
        cut_points[#cut_points + 1] = i
      end
    end
    return cut_points
  end

  -- findTurnStartIndex (the spec's -1 → nil).
  local function find_turn_start_index(entries, entry_index, start_index)
    for i = entry_index, start_index, -1 do
      local entry = entries[i]
      if entry.type == "branch_summary" or entry.type == "custom_message" then
        return i
      end
      if entry.type == "message" then
        local role = entry.message.role
        if role == "user" or role == "bashExecution" then return i end
      end
    end
    return nil
  end

  -- findCutPoint over [start_index, end_index).
  local function find_cut_point(entries, start_index, end_index, keep_recent_tokens)
    local cut_points = find_valid_cut_points(entries, start_index, end_index)

    if #cut_points == 0 then
      return { firstKeptEntryIndex = start_index, turnStartIndex = nil,
        isSplitTurn = false }
    end

    -- Walk backwards from newest, accumulating estimated message sizes.
    local accumulated = 0
    local cut_index = cut_points[1] -- Default: keep from first message.

    for i = end_index - 1, start_index, -1 do
      local entry = entries[i]
      if entry.type == "message" then
        accumulated = accumulated + bs.estimate_tokens(entry.message)
        if accumulated >= keep_recent_tokens then
          -- Closest valid cut point at or after this entry.
          for c = 1, #cut_points do
            if cut_points[c] >= i then cut_index = cut_points[c] break end
          end
          break
        end
      end
    end

    -- Scan backwards to include leading non-message entries.
    while cut_index > start_index do
      local prev = entries[cut_index - 1]
      if prev.type == "compaction" or prev.type == "message" then break end
      cut_index = cut_index - 1
    end

    local cut_entry = entries[cut_index]
    local is_user_message = cut_entry.type == "message"
      and cut_entry.message.role == "user"
    local turn_start = nil
    if not is_user_message then
      turn_start = find_turn_start_index(entries, cut_index, start_index)
    end

    return {
      firstKeptEntryIndex = cut_index,
      turnStartIndex = turn_start,
      isSplitTurn = not is_user_message and turn_start ~= nil,
    }
  end

  -- ---- compaction preparation ----

  -- extractFileOperations: previous pi-generated compaction details plus
  -- tool calls in the messages.
  local function extract_file_operations(messages, entries, prev_compaction_index)
    local file_ops = bs.create_file_ops()

    if prev_compaction_index then
      local prev = entries[prev_compaction_index]
      if not prev.fromHook and type(prev.details) == "table" then
        local details = prev.details
        if type(details.readFiles) == "table" then
          for _, f in ipairs(details.readFiles) do file_ops.read[f] = true end
        end
        if type(details.modifiedFiles) == "table" then
          for _, f in ipairs(details.modifiedFiles) do file_ops.edited[f] = true end
        end
      end
    end

    for _, message in ipairs(messages) do
      bs.extract_file_ops_from_message(message, file_ops)
    end
    return file_ops
  end

  -- prepareCompaction(pathEntries, settings).
  local function prepare_compaction(path_entries, settings)
    if #path_entries > 0 and path_entries[#path_entries].type == "compaction" then
      return nil
    end

    local prev_compaction_index = nil
    for i = #path_entries, 1, -1 do
      if path_entries[i].type == "compaction" then
        prev_compaction_index = i
        break
      end
    end

    local previous_summary = nil
    local boundary_start = 1
    if prev_compaction_index then
      local prev = path_entries[prev_compaction_index]
      previous_summary = prev.summary
      local first_kept_index = nil
      for i, entry in ipairs(path_entries) do
        if entry.id == prev.firstKeptEntryId then first_kept_index = i break end
      end
      boundary_start = first_kept_index or (prev_compaction_index + 1)
    end
    local boundary_end = #path_entries + 1

    local tokens_before = estimate_context_tokens(
      pi.session.build_context(path_entries).messages).tokens

    local cut_point = find_cut_point(path_entries, boundary_start, boundary_end,
      settings.keepRecentTokens)

    local first_kept = path_entries[cut_point.firstKeptEntryIndex]
    if not first_kept or not first_kept.id then
      return nil -- Session needs migration.
    end

    local history_end = cut_point.isSplitTurn and cut_point.turnStartIndex
      or cut_point.firstKeptEntryIndex

    local messages_to_summarize = {}
    for i = boundary_start, history_end - 1 do
      local msg = get_message_from_entry_for_compaction(path_entries[i])
      if msg then messages_to_summarize[#messages_to_summarize + 1] = msg end
    end

    local turn_prefix_messages = {}
    if cut_point.isSplitTurn then
      for i = cut_point.turnStartIndex, cut_point.firstKeptEntryIndex - 1 do
        local msg = get_message_from_entry_for_compaction(path_entries[i])
        if msg then turn_prefix_messages[#turn_prefix_messages + 1] = msg end
      end
    end

    local file_ops = extract_file_operations(
      messages_to_summarize, path_entries, prev_compaction_index)
    if cut_point.isSplitTurn then
      for _, msg in ipairs(turn_prefix_messages) do
        bs.extract_file_ops_from_message(msg, file_ops)
      end
    end

    return {
      firstKeptEntryId = first_kept.id,
      messagesToSummarize = messages_to_summarize,
      turnPrefixMessages = turn_prefix_messages,
      isSplitTurn = cut_point.isSplitTurn,
      tokensBefore = tokens_before,
      previousSummary = previous_summary,
      fileOps = file_ops,
      settings = settings,
    }
  end

  -- ---- summarization ----

  local SUMMARIZATION_PROMPT = [[The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.

Use this EXACT format:

## Goal
[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]

## Constraints & Preferences
- [Any constraints, preferences, or requirements mentioned by user]
- [Or "(none)" if none were mentioned]

## Progress
### Done
- [x] [Completed tasks/changes]

### In Progress
- [ ] [Current work]

### Blocked
- [Issues preventing progress, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [Ordered list of what should happen next]

## Critical Context
- [Any data, examples, or references needed to continue]
- [Or "(none)" if not applicable]

Keep each section concise. Preserve exact file paths, function names, and error messages.]]

  local UPDATE_SUMMARIZATION_PROMPT = [[The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.

Update the existing structured summary with new information. RULES:
- PRESERVE all existing information from the previous summary
- ADD new progress, decisions, and context from the new messages
- UPDATE the Progress section: move items from "In Progress" to "Done" when completed
- UPDATE "Next Steps" based on what was accomplished
- PRESERVE exact file paths, function names, and error messages
- If something is no longer relevant, you may remove it

Use this EXACT format:

## Goal
[Preserve existing goals, add new ones if the task expanded]

## Constraints & Preferences
- [Preserve existing, add new ones discovered]

## Progress
### Done
- [x] [Include previously done items AND newly completed items]

### In Progress
- [ ] [Current work - update based on progress]

### Blocked
- [Current blockers - remove if resolved]

## Key Decisions
- **[Decision]**: [Brief rationale] (preserve all previous, add new)

## Next Steps
1. [Update based on current state]

## Critical Context
- [Preserve important context, add new if needed]

Keep each section concise. Preserve exact file paths, function names, and error messages.]]

  local TURN_PREFIX_SUMMARIZATION_PROMPT = [[This is the PREFIX of a turn that was too large to keep. The SUFFIX (recent work) is retained.

Summarize the prefix to provide context for the retained suffix:

## Original Request
[What did the user ask for in this turn?]

## Early Progress
- [Key decisions and work done in the prefix]

## Context for Suffix
- [Information needed to understand the retained recent work]

Be concise. Focus on what's needed to understand the kept suffix.]]

  -- createSummarizationOptions + completeSummarization: the spec's
  -- optional streamFn (opts.stream_fn) wins, else completeSimple —
  -- pi.ai.stream_simple.
  local function complete_summarization(model, prompt_text, max_tokens, opts)
    local options = {
      maxTokens = max_tokens,
      signal = opts.signal,
      apiKey = opts.apiKey,
    }
    if model.reasoning and opts.thinkingLevel and opts.thinkingLevel ~= "off" then
      options.reasoning = opts.thinkingLevel
    end
    local context = {
      systemPrompt = bs.SUMMARIZATION_SYSTEM_PROMPT,
      messages = { { role = "user",
        content = { { type = "text", text = prompt_text } },
        timestamp = opts.now_ms and opts.now_ms() or pi.now_ms() } },
    }
    if opts.stream_fn then return opts.stream_fn(model, context, options) end
    return pi.ai.stream_simple(model, context, options, opts.on_event or function() end)
  end

  -- Aborted responses pass through (partial text): the spec detects
  -- cancellation from the signal after compact() returns, not here.
  local function response_text(response, label)
    if response.stopReason == "error" then
      error(label .. " failed: " .. (response.errorMessage or "Unknown error"), 0)
    end
    local texts = {}
    for _, block in ipairs(response.content or {}) do
      if block.type == "text" then texts[#texts + 1] = block.text end
    end
    return table.concat(texts, "\n")
  end

  -- generateSummary.
  local function generate_summary(current_messages, model, reserve_tokens, opts)
    local max_tokens = math.floor(0.8 * reserve_tokens)
    if model.maxTokens and model.maxTokens > 0 and model.maxTokens < max_tokens then
      max_tokens = model.maxTokens
    end

    local base_prompt = opts.previousSummary
      and UPDATE_SUMMARIZATION_PROMPT or SUMMARIZATION_PROMPT
    if opts.customInstructions then
      base_prompt = base_prompt .. "\n\nAdditional focus: " .. opts.customInstructions
    end

    local llm_messages = convert_to_llm(current_messages)
    local conversation_text = bs.serialize_conversation(llm_messages)

    local prompt_text = "<conversation>\n" .. conversation_text .. "\n</conversation>\n\n"
    if opts.previousSummary then
      prompt_text = prompt_text
        .. "<previous-summary>\n" .. opts.previousSummary .. "\n</previous-summary>\n\n"
    end
    prompt_text = prompt_text .. base_prompt

    local response = complete_summarization(model, prompt_text, max_tokens, opts)
    return response_text(response, "Summarization")
  end

  -- generateTurnPrefixSummary.
  local function generate_turn_prefix_summary(messages, model, reserve_tokens, opts)
    local max_tokens = math.floor(0.5 * reserve_tokens)
    if model.maxTokens and model.maxTokens > 0 and model.maxTokens < max_tokens then
      max_tokens = model.maxTokens
    end
    local llm_messages = convert_to_llm(messages)
    local conversation_text = bs.serialize_conversation(llm_messages)
    local prompt_text = "<conversation>\n" .. conversation_text
      .. "\n</conversation>\n\n" .. TURN_PREFIX_SUMMARIZATION_PROMPT
    local response = complete_summarization(model, prompt_text, max_tokens, opts)
    return response_text(response, "Turn prefix summarization")
  end

  -- compact(preparation, model, …) — the split-turn pair runs like the
  -- spec's Promise.all (pi.parallel; first settled failure rejects).
  local function compact(preparation, model, opts)
    local settings = preparation.settings
    local summary

    if preparation.isSplitTurn and #preparation.turnPrefixMessages > 0 then
      local history_result, prefix_result
      local completed = pi.parallel({
        function()
          if #preparation.messagesToSummarize == 0 then return "No prior history." end
          return generate_summary(preparation.messagesToSummarize, model,
            settings.reserveTokens, {
              apiKey = opts.apiKey, signal = opts.signal,
              customInstructions = opts.customInstructions,
              previousSummary = preparation.previousSummary,
              thinkingLevel = opts.thinkingLevel,
              now_ms = opts.now_ms, on_event = opts.on_event,
              stream_fn = opts.stream_fn,
            })
        end,
        function()
          return generate_turn_prefix_summary(preparation.turnPrefixMessages,
            model, settings.reserveTokens, {
              apiKey = opts.apiKey, signal = opts.signal,
              thinkingLevel = opts.thinkingLevel,
              now_ms = opts.now_ms, on_event = opts.on_event,
              stream_fn = opts.stream_fn,
            })
        end,
      })
      for _, entry in ipairs(completed) do
        if not entry.ok then error(entry.error, 0) end
        if entry.index == 1 then history_result = entry.value
        else prefix_result = entry.value end
      end
      summary = history_result
        .. "\n\n---\n\n**Turn Context (split turn):**\n\n" .. prefix_result
    else
      summary = generate_summary(preparation.messagesToSummarize, model,
        settings.reserveTokens, {
          apiKey = opts.apiKey, signal = opts.signal,
          customInstructions = opts.customInstructions,
          previousSummary = preparation.previousSummary,
          thinkingLevel = opts.thinkingLevel,
          now_ms = opts.now_ms, on_event = opts.on_event,
          stream_fn = opts.stream_fn,
        })
    end

    local read_files, modified_files = bs.compute_file_lists(preparation.fileOps)
    summary = summary .. bs.format_file_operations(read_files, modified_files)

    -- details persists to the session file: the file lists must encode
    -- as JSON arrays even when empty (pi writes `[]`).
    local function json_array(list)
      return setmetatable(list, { __pi_rs_json_array = true })
    end
    return {
      summary = summary,
      firstKeptEntryId = preparation.firstKeptEntryId,
      tokensBefore = preparation.tokensBefore,
      details = {
        readFiles = json_array(read_files),
        modifiedFiles = json_array(modified_files),
      },
    }
  end

  -- ---- pi-ai utils/overflow.ts isContextOverflow ----

  local OVERFLOW_PATTERNS = {
    "prompt is too long", -- Anthropic token overflow
    "request_too_large", -- Anthropic request byte-size overflow (HTTP 413)
    "input is too long for requested model", -- Amazon Bedrock
    "exceeds the context window", -- OpenAI (Completions & Responses API)
    "exceeds (?:the )?(?:model'?s )?maximum context length of [\\d,]+ tokens?", -- LiteLLM
    "input token count.*exceeds the maximum", -- Google (Gemini)
    "maximum prompt length is \\d+", -- xAI (Grok)
    "reduce the length of the messages", -- Groq
    "maximum context length is \\d+ tokens", -- OpenRouter (most backends)
    "exceeds (?:the )?maximum allowed input length of [\\d,]+ tokens?", -- OpenRouter/Poolside
    "input \\(\\d+ tokens\\) is longer than the model'?s context length \\(\\d+ tokens\\)", -- Together AI
    "exceeds the limit of \\d+", -- GitHub Copilot
    "exceeds the available context size", -- llama.cpp server
    "greater than the context length", -- LM Studio
    "context window exceeds limit", -- MiniMax
    "exceeded model token limit", -- Kimi For Coding
    "too large for model with \\d+ maximum context length", -- Mistral
    "model_context_window_exceeded", -- z.ai
    "prompt too long; exceeded (?:max )?context length", -- Ollama
    "context[_ ]length[_ ]exceeded", -- Generic fallback
    "too many tokens", -- Generic fallback
    "token limit exceeded", -- Generic fallback
    "^4(?:00|13)\\s*(?:status code)?\\s*\\(no body\\)", -- Cerebras
  }

  local NON_OVERFLOW_PATTERNS = {
    "^(Throttling error|Service unavailable):", -- AWS Bedrock (formatBedrockError)
    "rate limit", -- Generic rate limiting
    "too many requests", -- Generic HTTP 429 style
  }

  local function any_pattern_matches(patterns, text)
    for _, pattern in ipairs(patterns) do
      if pi.tui.js_regex_search(pattern, text) ~= nil then return true end
    end
    return false
  end

  local function is_context_overflow(message, context_window)
    -- Case 1: error message patterns (non-overflow exclusions first).
    if message.stopReason == "error" and message.errorMessage then
      if not any_pattern_matches(NON_OVERFLOW_PATTERNS, message.errorMessage)
         and any_pattern_matches(OVERFLOW_PATTERNS, message.errorMessage) then
        return true
      end
    end

    -- Case 2: silent overflow (z.ai style).
    if context_window and context_window ~= 0 and message.stopReason == "stop" then
      local input = (message.usage.input or 0) + (message.usage.cacheRead or 0)
      if input > context_window then return true end
    end

    -- Case 3: length-stop overflow (Xiaomi MiMo style).
    if context_window and context_window ~= 0 and message.stopReason == "length"
       and (message.usage.output or 0) == 0 then
      local input = (message.usage.input or 0) + (message.usage.cacheRead or 0)
      if input >= context_window * 0.99 then return true end
    end

    return false
  end

  return {
    DEFAULT_COMPACTION_SETTINGS = DEFAULT_COMPACTION_SETTINGS,
    calculate_context_tokens = calculate_context_tokens,
    get_assistant_usage = get_assistant_usage,
    get_last_assistant_usage = get_last_assistant_usage,
    estimate_context_tokens = estimate_context_tokens,
    should_compact = should_compact,
    find_turn_start_index = find_turn_start_index,
    find_cut_point = find_cut_point,
    prepare_compaction = prepare_compaction,
    generate_summary = generate_summary,
    compact = compact,
    is_context_overflow = is_context_overflow,
    get_message_from_entry = get_message_from_entry,
  }
end)(pi, branch_summary_lib, convert_to_llm)
