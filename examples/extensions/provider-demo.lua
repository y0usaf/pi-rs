-- provider-demo: exercises pi.register_provider / pi.unregister_provider.
-- Translation of the spec's `pi.registerProvider` doc examples
-- (core/extensions/types.ts): a custom proxy provider with models, a
-- baseUrl-only override of a built-in provider, and an OAuth-shaped
-- config whose functions stay Lua-side (the host mirror strips them,
-- keeping fields like oauth.name).

local pi = ...

-- Register a new provider with custom models.
pi.register_provider("my-proxy", {
  baseUrl = "https://proxy.example.com",
  apiKey = "$PROXY_API_KEY",
  api = "anthropic-messages",
  models = {
    {
      id = "claude-sonnet-4-20250514",
      name = "Claude 4 Sonnet (proxy)",
      reasoning = false,
      input = { "text", "image" },
      cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
      contextWindow = 200000,
      maxTokens = 16384,
    },
  },
})

-- Re-registration merges defined keys over the stored config
-- (spec upsertRegisteredProvider): the models above survive.
pi.register_provider("my-proxy", { name = "My Proxy" })

-- Override baseUrl for an existing provider.
pi.register_provider("anthropic", { baseUrl = "https://proxy.example.com" })

-- OAuth-shaped registration: functions never cross the bridge.
pi.register_provider("corporate-ai", {
  baseUrl = "https://ai.corp.com",
  api = "openai-responses",
  oauth = {
    name = "Corporate AI (SSO)",
    login = function(callbacks) end,
    refreshToken = function(credentials) end,
    getApiKey = function(credentials) return credentials.access end,
  },
})

-- And unregistration: this one never reaches the host mirror.
pi.register_provider("short-lived", { baseUrl = "https://gone.example.com" })
pi.unregister_provider("short-lived")
