-- Exercises the pi.auth mechanism bindings (core/auth-storage.ts +
-- utils/oauth as surfaced to Lua): credential CRUD against auth.json,
-- auth status, the OAuth provider registry mirror, and config-value
-- resolution. The login_start handle is exercised by the host tests
-- (it needs a scripted OAuth provider).
local pi = ...

pi.register_command("auth-demo", {
  description = "Exercise the pi.auth credential-storage bindings",
  handler = function(args)
    local request = pi.json.decode(args)
    local provider = request.provider or "demo-provider"

    pi.auth.set(provider, { type = "api_key", key = request.key or "sk-demo" })
    local stored = pi.auth.get(provider)
    local status = pi.auth.get_auth_status(provider)
    local listed = pi.auth.list()
    local had = pi.auth.has(provider)
    pi.auth.remove(provider)

    local oauth_ids = {}
    for _, oauth_provider in ipairs(pi.auth.oauth_providers()) do
      oauth_ids[#oauth_ids + 1] = oauth_provider.id
    end

    return {
      stored = stored,
      status = status,
      listed = listed,
      had = had,
      removed = pi.auth.get(provider) == nil,
      oauth = oauth_ids,
      auth_path = pi.auth.auth_path(),
      resolved = pi.auth.resolve_config_value(request.value or "literal-value"),
    }
  end,
})
