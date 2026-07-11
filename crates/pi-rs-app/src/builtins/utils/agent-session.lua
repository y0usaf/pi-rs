-- agent-session.ts + sdk.ts — the session persistence and restore policy
-- shared by the interactive pack and the one-shot coding-agent pack
-- (PLAN 6.1/6.2).
--
-- Shared fragment: included by both product packs, so it only assumes
-- the chunk argument. The mechanism (JSONL trees, entry shapes, leaf
-- bookkeeping) is `pi.session.*`; this file decides *what* persists and
-- what a reopened session restores.
local pi = ...

-- agent-session.ts _handleAgentEvent — the session-persistence slice:
-- message_end persists user/assistant/toolResult messages as message
-- entries and custom messages as custom_message entries. Other roles
-- (bashExecution, compactionSummary, branchSummary) are persisted
-- elsewhere by their own flows.
local function persist_agent_event(session, event)
  if not session or event.type ~= "message_end" or not event.message then return end
  local message = event.message
  if message.role == "custom" then
    session:append_custom_message_entry(
      message.customType, message.content, message.display, message.details)
  elseif message.role == "user" or message.role == "assistant" or
         message.role == "toolResult" then
    session:append_message(message)
  end
end

-- sdk.ts createAgentSession — the session-restore slice run once at
-- session construction:
--   * model: options.model (the CLI selection) wins; else an existing
--     session's saved model is restored when the registry knows it and
--     auth is configured, with the "Could not restore model" fallback
--     message otherwise; else the caller's findInitialModel fallback.
--   * thinking level: options.thinkingLevel (CLI) wins; else an existing
--     session restores its saved level when a thinking_level_change
--     entry exists (old sessions predate the entry type), else the
--     settings default, else DEFAULT_THINKING_LEVEL ("medium") — then
--     clamped to the resolved model's capabilities ("off" when no model),
--     so the appended entry records the effective level.
--   * messages: an existing session's branch messages seed the agent.
--   * appends: existing sessions backfill a missing thinking entry; new
--     sessions save the initial model and thinking level so resume can
--     restore them.
-- Returns { context, model, thinking_level, fallback_message,
-- has_existing }.
local function session_startup(session, options)
  local context = session:build_session_context()
  local has_existing = #context.messages > 0
  local has_thinking_entry = false
  for _, entry in ipairs(session:get_branch()) do
    if entry.type == "thinking_level_change" then has_thinking_entry = true end
  end

  local model = options.cliModel
  local fallback_message = nil
  if not model and has_existing and context.model then
    local restored = pi.ai.find_model(context.model.provider, context.model.modelId)
    if restored and pi.ai.has_configured_auth(restored) then
      model = restored
    end
    if not model then
      fallback_message = "Could not restore model "
        .. context.model.provider .. "/" .. context.model.modelId
    end
  end
  if not model then
    model = options.fallbackModel
    if model and fallback_message then
      fallback_message = fallback_message .. ". Using " .. model.provider .. "/" .. model.id
    end
  end

  local thinking_level = options.cliThinking
  if thinking_level == nil and has_existing then
    if has_thinking_entry then
      thinking_level = context.thinkingLevel
    else
      thinking_level = options.defaultThinkingLevel
    end
  end
  if thinking_level == nil then thinking_level = options.defaultThinkingLevel end
  -- core/defaults.ts DEFAULT_THINKING_LEVEL.
  if thinking_level == nil then thinking_level = "medium" end
  -- sdk.ts: clamp to model capabilities ("off" when no model resolved).
  if not model then
    thinking_level = "off"
  else
    thinking_level = pi.ai.clamp_thinking_level(model, thinking_level)
  end

  if has_existing then
    if not has_thinking_entry then
      session:append_thinking_level_change(thinking_level)
    end
  else
    if model then
      session:append_model_change(model.provider, model.id)
    end
    session:append_thinking_level_change(thinking_level)
  end

  return {
    context = context,
    model = model,
    thinking_level = thinking_level,
    fallback_message = fallback_message,
    has_existing = has_existing,
  }
end

-- main.ts createSessionManager → sdk.ts createAgentSession: the packs
-- open the CLI-selected session file when one was resolved, else create
-- a fresh session in the effective session dir.
local function construct_session(request)
  if request.sessionFile then
    return pi.session.open({
      path = request.sessionFile,
      sessionDir = request.sessionDir,
      agentDir = request.agentDir,
      -- main.ts: the startup missing-session-cwd prompt reopens the
      -- session with the selected cwd override.
      cwd = request.cwdOverride,
    })
  end
  return pi.session.create({
    cwd = request.cwd,
    sessionDir = request.sessionDir,
    agentDir = request.agentDir,
  })
end

-- The sdk.ts restore inputs as the CLI request carries them: CLI-sourced
-- model/thinking win; the caller's resolved model doubles as the
-- findInitialModel fallback; the thinking default reads the settings
-- store directly (sdk.ts settingsManager.getDefaultThinkingLevel()).
local function session_startup_from_request(session, request)
  return session_startup(session, {
    cliModel = request.modelFromCli and request.model or nil,
    fallbackModel = request.model,
    cliThinking = request.thinkingFromCli and request.thinkingLevel or nil,
    defaultThinkingLevel = pi.settings.default_thinking_level(),
  })
end
