-- Exercises the pi.ai model-registry bridge (core/model-registry.ts as
-- surfaced to Lua) plus pi.auth.get_api_key: catalog lookup, availability
-- filtered by configured auth, refresh after a credential change, and
-- per-provider API-key resolution — the seams the interactive /model
-- surface and the agent's per-request auth run on.
local pi = ...

pi.register_command("model-registry-demo", {
  description = "Exercise the pi.ai model-registry bindings",
  handler = function(args)
    local request = pi.json.decode(args)
    local provider = request.provider or "anthropic"
    local model_id = request.model or "claude-opus-4-8"

    local found = pi.ai.find_model(provider, model_id)
    local missing = pi.ai.find_model(provider, "no-such-model")

    local function provider_available()
      for _, model in ipairs(pi.ai.available_models()) do
        if model.provider == provider then return true end
      end
      return false
    end

    local before = provider_available()
    pi.auth.set(provider, { type = "api_key", key = request.key or "sk-demo" })
    pi.ai.registry_refresh()
    local after = provider_available()
    local has_auth = found and pi.ai.has_configured_auth(found) or false
    local api_key = pi.auth.get_api_key(provider)
    local subscription = found and pi.ai.is_using_oauth(found) or false
    pi.auth.remove(provider)
    pi.ai.registry_refresh()

    -- Thinking-level vocabulary (PLAN 7.2): getSupportedThinkingLevels
    -- honors thinkingLevelMap explicit nulls (level unsupported) and the
    -- xhigh-only-when-mapped rule; clampThinkingLevel finds the nearest
    -- supported level, searching upward first.
    local mapped_model = {
      reasoning = true,
      thinkingLevelMap = { xhigh = "max" },
    }
    local plain_model = { reasoning = true }
    local basic_model = { reasoning = false }

    return {
      found = found and { provider = found.provider, id = found.id, name = found.name } or nil,
      missing = missing == nil,
      available_before = before,
      available_after = after,
      has_configured_auth = has_auth,
      api_key = api_key,
      subscription = subscription,
      registry_error = pi.ai.registry_error(),
      mapped_levels = pi.ai.supported_thinking_levels(mapped_model),
      plain_levels = pi.ai.supported_thinking_levels(plain_model),
      basic_levels = pi.ai.supported_thinking_levels(basic_model),
      clamped_up = pi.ai.clamp_thinking_level(plain_model, "xhigh"),
      clamped_off = pi.ai.clamp_thinking_level(basic_model, "medium"),
    }
  end,
})
