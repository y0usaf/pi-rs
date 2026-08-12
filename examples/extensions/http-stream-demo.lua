-- Exerciser for the abort-aware streaming/fetch HTTP mechanisms:
-- pi.http.stream(url, options?, on_chunk) and pi.http.fetch(url, options?).
--
-- `pi.http.stream` streams body chunks to `on_chunk` as binary-safe Lua
-- strings and cancels the in-flight request when `options.signal` aborts
-- (the Webfetch download / Morph gateway proxy mechanism). `pi.http.fetch`
-- is a one-shot fetch with method/body/headers/timeout.
local pi = ...

pi.register_command("http-stream-demo", {
  description = "Stream a GET body through pi.http.stream and fetch via pi.http.fetch",
  handler = function(arg)
    local url = arg
    local chunks = {}
    local result = pi.http.stream(url, {}, function(chunk)
      chunks[#chunks + 1] = chunk
    end)
    local body = table.concat(chunks)
    return {
      status = result.status,
      ok = result.ok,
      body = body,
      chunk_count = #chunks,
    }
  end,
})

pi.register_command("http-fetch-demo", {
  description = "Perform a POST fetch via pi.http.fetch",
  handler = function(arg)
    local url = arg
    local res = pi.http.fetch(url, {
      method = "POST",
      headers = { ["content-type"] = "application/json" },
      body = [[{"hello":"world"}]],
    })
    return { status = res.status, ok = res.ok, body = res.body }
  end,
})

-- Abort a mid-stream request via an abort signal: the on_chunk callback
-- fires for the leading chunk, then the signal aborts and stream errors.
pi.register_command("http-stream-abort", {
  description = "Abort an in-flight pi.http.stream via an abort signal",
  handler = function(arg)
    local url = arg
    local signal = pi.abort_signal()
    pi.spawn(function()
      -- The server sends a leading chunk then stalls; abort after it lands.
      pi.sleep(50)
      signal:abort()
    end)
    local chunks = {}
    local ok, err = pcall(pi.http.stream, url, { signal = signal }, function(chunk)
      chunks[#chunks + 1] = chunk
    end)
    return {
      aborted = not ok,
      error = ok and "" or tostring(err),
      received = table.concat(chunks),
    }
  end,
})