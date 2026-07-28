-- Walking skeleton: minimal application root proving the interactive loop.
--
-- This package demonstrates the generic product loop end to end: startup
-- renders an input-ready frame, typed input echoes through a retained
-- display, and a shutdown action exits cleanly. It also proves the public
-- effect and model bindings: process execution, missing-model diagnosis,
-- cancellation through the versioned effect surface, and deterministic
-- fixture-provider streaming with incremental rendered frames.

local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local effects = pi.effects.v1
local models = pi.models.v1

-- Shared display and input buffer survive across dispatches via the
-- package's Lua upvalues (standard Lua closure semantics).
local display = nil
local input = nil

local function ensure_display(columns, rows)
  if not display then
    display = terminal.display()
    input = terminal.input_buffer()
  end
  return display
end

local function render_frame(text, ready)
  local d = ensure_display()
  local label = ready and "pi> " .. text or text
  local frame = d:submit({
    version = terminal.display_schema_version,
    viewport = { columns = 80, rows = 1 },
    root = 1,
    nodes = { {
      id = 1,
      rect = { x = 0, y = 0, width = 80, height = 1 },
      content = {
        kind = "text",
        runs = { { text = label } },
      },
    } },
  })
  if frame.ansi and #frame.ansi > 0 then
    roots.action("ansi", { data = frame.ansi })
  end
  return frame
end

-- Run a bounded process effect and render its output.
local function run_effect_demo()
  render_frame("[running echo effect]", false)
  local result = effects.process.run("echo", { "hello-from-effect" }, {
    timeout_ms = 5000,
  })
  if result and result.stdout then
    local trimmed = result.stdout:gsub("%s+$", "")
    render_frame("effect: " .. trimmed, true)
  else
    render_frame("effect: no output", true)
  end
end

-- Attempt to find a model that does not exist; diagnose the miss.
local function run_missing_model_demo()
  render_frame("[looking up model]", false)
  local found = models.find("nonexistent-provider", "nonexistent-model")
  if found == nil then
    render_frame("model: not found (expected)", true)
  else
    render_frame("model: unexpectedly found", true)
  end
end

-- Start a timer and cancel it before it fires.
local function run_cancellation_demo()
  render_frame("[starting timer]", false)
  local signal = effects.cancellation.new()
  -- Abort immediately; the sleep should observe the cancellation.
  signal:abort()
  local ok, err = pcall(function()
    effects.timer.sleep(60000, { signal = signal })
  end)
  if not ok or (signal and signal:is_aborted()) then
    render_frame("timer: cancelled", true)
  else
    render_frame("timer: completed (unexpected)", true)
  end
end


-- Stream a deterministic fixture provider and render each text delta as
-- an incremental frame. The fixture endpoint is an ordinary local HTTP
-- server written by the PTY harness; the port arrives through the public
-- filesystem effect, so no private channel exists between test and Lua.
local function run_stream_demo()
  render_frame("[streaming fixture provider]", false)
  local port_text = effects.fs.read("fixture_port.txt")
  local port = port_text:match("(%d+)")
  if not port then
    render_frame("stream: no fixture port", true)
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
        render_frame("stream> " .. accumulated, false)
      end
    end)
  end)

  if not ok then
    render_frame("stream: error " .. tostring(result_or_err), true)
    return
  end
  local message = result_or_err
  if message and message.stopReason == "stop" and #accumulated > 0 then
    render_frame("stream done: " .. accumulated, true)
  else
    local reason = message and tostring(message.stopReason) or "nil"
    render_frame("stream: unexpected stop " .. reason, true)
  end
end
roots.register({
  kind = "application",
  id = "walking-skeleton",
  dispatch = function(snapshot)
    local kind = snapshot.event.kind

    if kind == "startup" then
      render_frame("", true)
      return
    end

    if kind == "input" then
      local data = snapshot.event.data or ""
      -- Parse input events through the bounded stdin buffer.
      local events = input:feed(data)
      for _, event in ipairs(events) do
        if event.kind == "data" then
          local ch = event.data
          if ch == "q" then
            roots.action("shutdown", { reason = "user quit" })
            return
          elseif ch == "r" then
            run_effect_demo()
          elseif ch == "m" then
            run_missing_model_demo()
          elseif ch == "t" then
            run_cancellation_demo()
          elseif ch == "s" then
            run_stream_demo()
          else
            -- Echo any other input back through the display.
            render_frame(ch, true)
          end
        end
      end
      return
    end
  end,
})
