-- Walking skeleton agent root: owns turn logic over the public effect and
-- model bindings. Rendering goes through the frontend root via the public
-- roots.v1.dispatch seam; the agent republishes the frontend's ANSI actions
-- into its own batch so the coordinating application can forward them.

local pi = ...
local roots = pi.roots.v1
local effects = pi.effects.v1
local models = pi.models.v1

-- Render text through the frontend root and republish its ANSI actions into
-- this dispatch's batch. Cross-root batches never publish implicitly.
local function render(text, ready)
  local batch = roots.dispatch("frontend", {
    kind = "render",
    text = text,
    ready = ready,
  })
  for _, action in ipairs(batch.actions) do
    roots.action(action.kind, action.payload)
  end
end

-- Run a bounded process effect and render its output.
local function run_effect_demo()
  render("[running echo effect]", false)
  local result = effects.process.run("echo", { "hello-from-effect" }, {
    timeout_ms = 5000,
  })
  if result and result.stdout then
    local trimmed = result.stdout:gsub("%s+$", "")
    render("effect: " .. trimmed, true)
  else
    render("effect: no output", true)
  end
end

-- Attempt to find a model that does not exist; diagnose the miss.
local function run_missing_model_demo()
  render("[looking up model]", false)
  local found = models.find("nonexistent-provider", "nonexistent-model")
  if found == nil then
    render("model: not found (expected)", true)
  else
    render("model: unexpectedly found", true)
  end
end

-- Start a timer and cancel it before it fires.
local function run_cancellation_demo()
  render("[starting timer]", false)
  local signal = effects.cancellation.new()
  -- Abort immediately; the sleep should observe the cancellation.
  signal:abort()
  local ok, err = pcall(function()
    effects.timer.sleep(60000, { signal = signal })
  end)
  if not ok or (signal and signal:is_aborted()) then
    render("timer: cancelled", true)
  else
    render("timer: completed (unexpected)", true)
  end
end

-- Stream a deterministic fixture provider and render each text delta as
-- an incremental frame. The fixture endpoint is an ordinary local HTTP
-- server written by the PTY harness; the port arrives through the public
-- filesystem effect, so no private channel exists between test and Lua.
local function run_stream_demo()
  render("[streaming fixture provider]", false)
  local port_text = effects.fs.read("fixture_port.txt")
  local port = port_text:match("(%d+)")
  if not port then
    render("stream: no fixture port", true)
    return
  end

  local model = {
    id = "fixture-model",
    name = "Fixture Model",
    api = "openai-completions",
    provider = "fixture",
    baseUrl = "http://127.0.0.1:" .. port,
    reasoning = false,
    input = { "text" },
    cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0 },
    contextWindow = 4096,
    maxTokens = 64,
  }
  local context = {
    messages = {
      { role = "user", content = "hello", timestamp = 0 },
    },
  }

  local accumulated = ""
  local ok, result_or_err = pcall(function()
    return models.stream(model, context, { apiKey = "fixture-key" }, function(event)
      if event.type == "text_delta" and event.delta then
        accumulated = accumulated .. event.delta
        render("stream> " .. accumulated, false)
      end
    end)
  end)

  if not ok then
    render("stream: error " .. tostring(result_or_err), true)
    return
  end
  local message = result_or_err
  if message and message.stopReason == "stop" and #accumulated > 0 then
    render("stream done: " .. accumulated, true)
  else
    local reason = message and tostring(message.stopReason) or "nil"
    render("stream: unexpected stop " .. reason, true)
  end
end

roots.register({
  kind = "agent",
  id = "walking-skeleton-agent",
  dispatch = function(snapshot)
    local kind = snapshot.event.kind

    if kind == "turn" then
      local key = snapshot.event.key
      if key == "r" then
        run_effect_demo()
      elseif key == "m" then
        run_missing_model_demo()
      elseif key == "t" then
        run_cancellation_demo()
      elseif key == "s" then
        run_stream_demo()
      else
        render(key, true)
      end
      return
    end
  end,
})
