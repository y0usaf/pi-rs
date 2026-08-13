-- Translation of Pi v0.79.0 examples/extensions/provider-payload.ts.
-- Logs provider request payloads and response headers from the
-- before_provider_request / after_provider_response hooks.
local pi = ...

local function log_file()
  return pi.path.join(pi.cwd(), ".pi", "provider-payload.log")
end

pi.on("before_provider_request", function(event)
  pi.fs.append_file(log_file(), pi.json.encode(event.payload) .. "\n\n")

  -- Optional: replace the payload instead of only logging it.
  -- return { ...event.payload, temperature: 0 }
end)

pi.on("after_provider_response", function(event)
  pi.fs.append_file(log_file(), "[" .. event.status .. "] " .. pi.json.encode(event.headers) .. "\n\n")
end)