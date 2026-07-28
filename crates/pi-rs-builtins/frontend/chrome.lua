-- Header, status, footer, and guidance text for the shipped frontend.
--
-- Every string below is product policy. The kernel reports mechanism failures
-- as opaque reasons; deciding that a missing model or a rejected credential
-- deserves a specific actionable line is this module's job.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.frontend.chrome",
  version = "1",
  factory = function()
    local STATUS_TEXT = {
      idle = "idle",
      streaming = "streaming…",
      cancelled = "cancelled",
      error = "error",
      exiting = "exiting",
    }

    local HINTS = "enter send · alt+enter newline · ctrl+c interrupt · ctrl+d exit"

    -- Guidance is matched on the agent's reason string, which is ordinary
    -- data: a replacement agent may report other reasons and a replacement
    -- frontend may phrase them differently.
    local AUTH_PATTERNS = {
      "api key",
      "apikey",
      "api_key",
      "credential",
      "unauthorized",
      "authentication",
      "401",
      "403",
    }

    local Chrome = {}
    Chrome.__index = Chrome

    function Chrome.new()
      return setmetatable({
        model = nil,
        status = "idle",
        guidance = nil,
      }, Chrome)
    end

    function Chrome:set_model(label)
      self.model = label and tostring(label) or nil
    end

    function Chrome:set_status(state)
      self.status = STATUS_TEXT[state] and state or "idle"
    end

    function Chrome:set_guidance(text)
      self.guidance = text and tostring(text) or nil
    end

    function Chrome:clear_guidance()
      self.guidance = nil
    end

    function Chrome:guidance_for(reason)
      local text = tostring(reason or ""):lower()
      if text == "missing_model" or text:find("missing model", 1, true) then
        return "no model selected: configure a provider model before sending a prompt"
      end
      for _, pattern in ipairs(AUTH_PATTERNS) do
        if text:find(pattern, 1, true) then
          return "provider credentials missing or rejected: add a working API key, then retry"
        end
      end
      if text:find("request_limit", 1, true) or text:find("tool_iteration_limit", 1, true) then
        return "turn stopped at its declared limit: send a narrower prompt"
      end
      return "provider request failed: " .. tostring(reason)
    end

    function Chrome:header()
      local model = self.model or "no model"
      return "pi · " .. model .. " · " .. (STATUS_TEXT[self.status] or self.status)
    end

    function Chrome:footer()
      return HINTS
    end

    function Chrome:guidance_row()
      return self.guidance
    end

    return {
      new = Chrome.new,
      hints = HINTS,
      status_text = STATUS_TEXT,
    }
  end,
})
