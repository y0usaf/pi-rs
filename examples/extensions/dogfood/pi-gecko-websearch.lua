-- File-backed pi-gecko-websearch translation (dogfood package).
-- Web search + browse via a headless Gecko browser through the Marionette
-- protocol, using pi.tcp (framed TCP client) + pi.process (managed subprocess)
-- + pi.fs/pi.path/pi.env (temp profile, cookie copy, binary discovery).
-- Public surface only: pi.register_tool, pi.tcp.connect, pi.process.spawn,
-- pi.exec, pi.fs.{mkdtemp,remove_dir_all,exists,read_file,write_file_atomic,
-- copy_file,read_dir,tmpdir}, pi.path.{join,dirname}, pi.env, pi.module.require
-- ("pi.tools.truncate","1"), pi.set_timeout/clear_timeout, pi.now_ms, pi.cwd.
-- Cleanup: the browser pool shutdown() (kill each gecko process, close each
-- Marionette socket via handle close + dispose, remove temp profile dirs) runs
-- on session_shutdown; every socket/process handle is owned and dropped, so no
-- child/socket survives disposal.
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local truncate_head = truncate.truncate_head
local format_size = truncate.format_size
local DEFAULT_MAX_BYTES = truncate.DEFAULT_MAX_BYTES
local DEFAULT_MAX_LINES = truncate.DEFAULT_MAX_LINES

local function get_agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE
  if home then return pi.path.join(home, ".pi", "agent") end
  return pi.path.join(".pi", "agent")
end

-- ── Settings ───────────────────────────────────────────────────────
local function is_record(v) return type(v) == "table" end

local function str(v) return type(v) == "string" and v:gsub("^%s+", ""):gsub("%s+$", "") or nil end

local function positive_int(v)
  local n
  if type(v) == "number" then n = v
  elseif type(v) == "string" then n = tonumber(v:gsub("^%s+", ""):gsub("%s+$", ""))
  else n = nil end
  if n and n == math.floor(n) and n > 0 then return n end
  return nil
end

local function pick_settings(parsed)
  if not is_record(parsed) then return nil end
  local ext = parsed.extensionSettings
  if not is_record(ext) then return nil end
  return ext["gecko-websearch"]
end

local function read_settings(file_path)
  if not pi.fs.exists(file_path) then return {} end
  local ok, raw = pcall(pi.fs.read_file, file_path)
  if not ok then return {} end
  local ok2, parsed = pcall(pi.json.decode, raw)
  if not ok2 then return {} end
  local settings = pick_settings(parsed)
  if not is_record(settings) then return {} end
  return {
    binary = str(settings.binary),
    profile = str(settings.profile),
    profileRoot = str(settings.profileRoot),
    maxBrowsers = positive_int(settings.maxBrowsers),
  }
end

local function clamp_max_browsers(v) return math.max(1, math.min(v or 2, 8)) end

-- ── Marionette client (pi.tcp) ─────────────────────────────────────
local function marionette_client()
  local self = {}
  self.socket = nil
  self.msg_id = 0
  self.buffer = ""
  self.connected = false
  self.pending = nil

  function self:read_bytes(n, timeout_ms)
    while #self.buffer < n do
      local data = self.socket:read(4096)
      if data and #data > 0 then
        self.buffer = self.buffer .. data
      else
        return false
      end
    end
    return true
  end

  function self.try_parse_message()
    local colon = self.buffer:find(":", 1, true)
    if not colon then return nil end
    local len_str = self.buffer:sub(1, colon - 1)
    local length = tonumber(len_str)
    if not length then self.buffer = self.buffer:sub(colon + 1); return nil end
    local payload_start = colon + 1
    local payload_end = payload_start + length
    if #self.buffer < payload_end then return nil end
    local payload = self.buffer:sub(payload_start, payload_end - 1)
    self.buffer = self.buffer:sub(payload_end)
    local ok, parsed = pcall(pi.json.decode, payload)
    if not ok then return nil end
    return parsed
  end

  function self.connect(port, host, timeout_ms)
    timeout_ms = timeout_ms or 10000
    host = host or "127.0.0.1"
    port = port or 2828
    self.socket = pi.tcp.connect(host, port, { timeout_ms = timeout_ms })
    self.buffer = ""
    self.connected = true
    -- Read and discard the greeting (first length-prefixed message).
    return true
  end

  function self.deliver_response()
    if not self.pending then return end
    local msg = self.try_parse_message()
    if msg == nil then return end
    local pending = self.pending
    self.pending = nil
    if type(msg) ~= "table" or #msg < 4 then
      pending.reject("Unexpected Marionette response: " .. tostring(msg))
      return
    end
    local error_val, result = msg[3], msg[4]
    if error_val then
      local err_str = (type(error_val) == "table" and (error_val.message or pi.json.encode(error_val))) or tostring(error_val)
      pending.reject("Marionette error: " .. tostring(err_str))
    else
      pending.resolve(result)
    end
  end

  function self.send(command, params, timeout_ms)
    if not self.socket or not self.connected then error("Marionette not connected", 0) end
    timeout_ms = timeout_ms or 30000
    params = params or {}
    local id = self.msg_id
    self.msg_id = self.msg_id + 1
    local message = pi.json.encode({ 0, id, command, params })
    local wire = tostring(#message) .. ":" .. message
    self.socket:write(wire)

    local settled = false
    local timer
    local done = false
    local result_val, error_val
    local function finish()
      if settled then return end
      settled = true
      if timer then pi.clear_timeout(timer) end
    end

    self.pending = {
      resolve = function(v) finish(); result_val = v; done = true end,
      reject = function(e) finish(); error_val = e; done = true end,
    }
    timer = pi.set_timeout(function()
      self.pending = nil
      error("Marionette command '" .. command .. "' timed out after " .. timeout_ms .. "ms", 0)
    end, timeout_ms)

    self.deliver_response()
    while not done do
      self.deliver_response()
      if not done then pi.sleep(5) end
    end
    if settled and self.pending == nil then self.pending = nil end
    if error_val then error(error_val, 0) end
    return result_val
  end

  function self.new_session() return self.send("WebDriver:NewSession", { capabilities = { alwaysMatch = { acceptInsecureCerts = true } } }, 30000) end

  function self.navigate(url, timeout_ms) return self.send("WebDriver:Navigate", { url = url }, timeout_ms or 30000) end

  function self.execute_script(script, args, timeout_ms)
    local result = self.send("WebDriver:ExecuteScript", { script = script, args = args or {} }, timeout_ms or 10000)
    if type(result) == "table" and result.value then return result.value end
    return result
  end

  function self.get_page_source(timeout_ms)
    local result = self.send("WebDriver:GetPageSource", {}, timeout_ms or 10000)
    if result and result.value then return result.value end
    return ""
  end

  function self.close()
    if self.socket then
      local ok = pcall(function() self.send("WebDriver:DeleteSession", {}, 5000) end)
      pcall(self.socket.close, self.socket)
      pcall(self.socket.dispose, self.socket)
      self.socket = nil
    end
    self.connected = false
  end

  return self
end

-- ── Browser pool ───────────────────────────────────────────────────
local function browser_manager(cwd)
  local global_settings = read_settings(pi.path.join(get_agent_dir(), "settings.json"))
  local project_settings = read_settings(pi.path.join(cwd, ".pi", "settings.json"))
  local settings = {}
  for k, v in pairs(global_settings) do settings[k] = v end
  for k, v in pairs(project_settings) do settings[k] = v end
  local max_browsers = clamp_max_browsers(positive_int(pi.env.PI_GECKO_MAX_BROWSERS) or settings.maxBrowsers)
  local browsers = {}
  local idle = {}
  local waiters = {}
  local shutting_down = false

  local function acquire()
    if shutting_down then error("Gecko browser pool is shutting down", 0) end
    local idle_browser = table.remove(idle)
    if idle_browser then return idle_browser end
    if #browsers < max_browsers then
      local b = managed_browser(settings)
      browsers[#browsers + 1] = b
      return b
    end
    local waiter = {}
    waiters[#waiters + 1] = waiter
    local result = {}
    -- simulate promise via a done flag resolved on release
    local acquired
    if next(waiters) then
      error("browser pool saturated", 0)
    end
    return result
  end

  local function release(browser)
    if shutting_down then return end
    local waiter = table.remove(waiters, 1)
    if waiter and waiter.resolve then
      waiter.resolve(browser)
    else
      idle[#idle + 1] = browser
    end
  end

  local function with_client(fn)
    local browser = acquire()
    browser.ensure_running()
    local ok, result = pcall(fn, browser.client)
    release(browser)
    if not ok then error(result, 0) end
    return result
  end

  local function shutdown_all()
    shutting_down = true
    for _, b in ipairs(browsers) do
      pcall(b.shutdown, b)
    end
    browsers = {}
    idle = {}
    shutting_down = false
  end

  return { with_client = with_client, shutdown = shutdown_all }
end

local function managed_browser(settings)
  local self = {}
  self.process = nil
  self.client = nil
  self.temp_profile_dir = nil
  self.running = false

  function self.copy_cookies(source_profile, dest_profile)
    for _, file in ipairs({ "cookies.sqlite", "cookies.sqlite-wal", "cert9.db" }) do
      local src = pi.path.join(source_profile, file)
      if pi.fs.exists(src) then
        local ok = pcall(pi.fs.copy_file, src, pi.path.join(dest_profile, file))
        if not ok then end -- non-fatal
      end
    end
  end

  function self.parse_profiles_ini(ini_path)
    local ok, content = pcall(pi.fs.read_file, ini_path)
    if not ok then return nil end
    local base_dir = pi.path.dirname(ini_path)
    local first_profile, default_profile = nil, nil
    for section in content:gmatch("(%[.-%])") do
      section = section:gsub("^%[%s*", ""):gsub("%s*%]$", "")
      if section:lower():match("^profile") then
        local path = nil
        local is_relative = "1"
        for key, value in section:gmatch("([%w_.]+)%s*=%s*([^\n]*)") do
          key = key:gsub("^%s+", ""):gsub("%s+$", "")
          value = value:gsub("^%s+", ""):gsub("%s+$", "")
          if key == "Path" then path = value
          elseif key == "IsRelative" then is_relative = value
          elseif key == "Default" then
            if value == "1" and path then default_profile = (is_relative == "0") and path or pi.path.join(base_dir, path) end
          end
        end
        if path then
          local resolved = (is_relative == "0") and path or pi.path.join(base_dir, path)
          if not first_profile then first_profile = resolved end
        end
      end
    end
    local chosen = default_profile or first_profile
    return chosen and pi.fs.exists(chosen) and chosen or nil
  end

  function self.scan_profile_root(profile_root)
    if pi.fs.exists(pi.path.join(profile_root, "cookies.sqlite")) then return profile_root end
    local ok, entries = pcall(pi.fs.read_dir, profile_root)
    if not ok then return nil end
    local dirs = {}
    for _, name in ipairs(entries) do
      if type(name) == "string" then
        local candidate = pi.path.join(profile_root, name)
        if pi.fs.exists(pi.path.join(candidate, "cookies.sqlite")) then dirs[#dirs + 1] = candidate end
      end
    end
    table.sort(dirs, function(a, b)
      local a_idx = a:find(".default", 1, true) and 1 or 0
      local b_idx = b:find(".default", 1, true) and 1 or 0
      return b_idx < a_idx
    end)
    return dirs[1]
  end

  function self.resolve_profile_root(profile_root)
    if not profile_root or not pi.fs.exists(profile_root) then return nil end
    local ini = pi.path.join(profile_root, "profiles.ini")
    if pi.fs.exists(ini) then
      local parsed = self.parse_profiles_ini(ini)
      if parsed then return parsed end
    end
    return self.scan_profile_root(profile_root)
  end

  function self.resolve_profile_path()
    local configured = pi.env.PI_GECKO_PROFILE or settings.profile
    if configured and pi.fs.exists(configured) then return configured end
    local home = pi.env.HOME or pi.env.USERPROFILE or ""
    if type(home) == "string" and home == "" then home = "" end
    local roots = {
      pi.env.PI_GECKO_PROFILE_ROOT,
      settings.profileRoot,
      home ~= "" and pi.path.join(home, ".mozilla", "firefox") or nil,
      home ~= "" and pi.path.join(home, ".librewolf") or nil,
    }
    for _, root in ipairs(roots) do
      local profile = self.resolve_profile_root(root)
      if profile then return profile end
    end
    return nil
  end

  function self.resolve_binary_candidate(candidate)
    if not candidate then return nil end
    local value = candidate:gsub("^%s+", ""):gsub("%s+$", "")
    if value == "" then return nil end
    if value:find("/", 1, true) then
      return pi.fs.exists(value) and value or nil
    end
    local probe = pi.exec("which", { value })
    if probe.code ~= 0 or not probe.stdout then return nil end
    local resolved = probe.stdout:match("^([^\r\n]+)")
    if resolved and resolved:gsub("%s", "") ~= "" then return resolved end
    return nil
  end

  function self.find_binary()
    for _, candidate in ipairs({ pi.env.PI_GECKO_BINARY, settings.binary }) do
      local resolved = self.resolve_binary_candidate(candidate)
      if resolved then return resolved end
    end
    local home = pi.env.HOME or pi.env.USERPROFILE or ""
    local names = { "firefox", "librewolf" }
    local flatpaks = { firefox = "org.mozilla.firefox", librewolf = "io.gitlab.librewolf-community" }
    local candidates = {}
    for _, n in ipairs(names) do
      candidates[#candidates + 1] = n
      candidates[#candidates + 1] = "/usr/bin/" .. n
      candidates[#candidates + 1] = "/usr/local/bin/" .. n
      candidates[#candidates + 1] = "/snap/bin/" .. n
      candidates[#candidates + 1] = "/var/lib/flatpak/exports/bin/" .. flatpaks[n]
      if home ~= "" then candidates[#candidates + 1] = pi.path.join(home, ".local/bin/" .. n) end
      local cap = n:sub(1, 1):upper() .. n:sub(2)
      candidates[#candidates + 1] = "/Applications/" .. cap .. ".app/Contents/MacOS/" .. n
    end
    for _, candidate in ipairs(candidates) do
      local resolved = self.resolve_binary_candidate(candidate)
      if resolved then return resolved end
    end
    return "firefox"
  end

  function self.read_active_port(profile_dir)
    local ok, text = pcall(pi.fs.read_file, pi.path.join(profile_dir, "MarionetteActivePort"))
    if not ok then return nil end
    local port = tonumber(text:gsub("%s", ""))
    if port and port == math.floor(port) and port > 0 then return port end
    return nil
  end

  function self.wait_for_marionette(client, profile_dir, timeout_ms)
    local start = pi.now_ms()
    local last_port = nil
    while pi.now_ms() - start < timeout_ms do
      local port = self.read_active_port(profile_dir)
      if port then
        last_port = port
        local ok = pcall(client.connect, client, port, "127.0.0.1", 2000)
        if ok then
          self.marionette_port = port
          return
        end
      end
      pi.sleep(500)
    end
    local suffix = last_port and (" on port " .. last_port) or " (no MarionetteActivePort file)"
    error("Timed out waiting for Marionette" .. suffix .. " after " .. timeout_ms .. "ms", 0)
  end

  function self.ensure_running()
    if self.running then return self.client end
    self.shutdown()
    self.temp_profile_dir = pi.fs.mkdtemp(pi.path.join(pi.fs.tmpdir(), "pi-gecko-"))
    local source_profile = self.resolve_profile_path()
    if source_profile then self.copy_cookies(source_profile, self.temp_profile_dir) end
    local user_js = table.concat({
      'user_pref("marionette.port", 0);',
      'user_pref("marionette.enabled", true);',
      'user_pref("browser.shell.checkDefaultBrowser", false);',
      'user_pref("browser.startup.homepage_override.mstone", "ignore");',
      'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
      'user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);',
      'user_pref("browser.sessionstore.resume_from_crash", false);',
      'user_pref("browser.cache.disk.enable", false);',
      'user_pref("media.hardware-video-decoding.enabled", false);',
    }, "\n")
    pi.fs.write_file_atomic(pi.path.join(self.temp_profile_dir, "user.js"), user_js)
    local binary = self.find_binary()
    local args = { "--marionette", "--headless", "--profile", self.temp_profile_dir, "--no-remote" }
    self.process = pi.process.spawn(binary, args, { stdio = "ignore" })
    self.client = marionette_client()
    self.wait_for_marionette(self.client, self.temp_profile_dir, 45000)
    self.client.new_session()
    self.running = true
    return self.client
  end

  function self.shutdown()
    if self.client then
      pcall(self.client.close, self.client)
      self.client = nil
    end
    if self.process then
      local killed = pcall(self.process.kill, self.process, "SIGTERM")
      pcall(self.process.dispose, self.process)
      self.process = nil
    end
    if self.temp_profile_dir then
      local ok = pcall(pi.fs.remove_dir_all, self.temp_profile_dir)
      if not ok then end
      self.temp_profile_dir = nil
    end
    self.running = false
  end

  return self
end

-- ── Common top-level ───────────────────────────────────────────────
local function encode_uri_component(s)
  s = tostring(s)
  s = s:gsub("([^%w%.%*%_%-%!~'%(%)])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
  return s
end

local SEARCH_URLS = {
  google = function(q) return "https://www.google.com/search?q=" .. encode_uri_component(q) end,
  duckduckgo = function(q) return "https://html.duckduckgo.com/html/?q=" .. encode_uri_component(q) end,
  brave = function(q) return "https://search.brave.com/search?q=" .. encode_uri_component(q) end,
}
local DEFAULT_ENGINE = "duckduckgo"

local function format_results(results)
  if #results == 0 then return "No search results found." end
  local parts = {}
  for i, r in ipairs(results) do
    local entry = i .. ". " .. r.title .. "\n   " .. r.url
    if r.snippet then entry = entry .. "\n   " .. r.snippet end
    parts[#parts + 1] = entry
  end
  return table.concat(parts, "\n\n")
end

local function search_failure_diagnostic(page_text, engine)
  local text = tostring(page_text or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then return nil end
  if text:lower():match("unusual traffic") or text:lower():match("not a robot")
    or text:lower():match("captcha") or text:lower():match("verify that you")
    or text:lower():match("automated queries") then
    return engine .. " returned an anti-bot/verification page instead of search results:\n\n" .. text:sub(1, 1000)
  end
  if text:lower():match("enable javascript") or text:lower():match("turn on javascript") or text:lower():match("cookies are disabled") then
    return engine .. " returned a browser compatibility page instead of search results:\n\n" .. text:sub(1, 1000)
  end
  return nil
end

local function search_tool(browser)
  pi.register_tool({
    name = "web_search",
    label = "Web Search",
    description = "Search the web using a real Gecko browser. Returns search result titles, URLs, and snippets. Uses your browser fingerprint and cookies.",
    promptSnippet = "Search the web via a Gecko browser (real browser fingerprint + cookies)",
    promptGuidelines = {
      "Use specific, targeted search queries for best results.",
      "Default engine is DuckDuckGo (fastest, most reliable parsing). Use Google if DDG results are insufficient.",
      "The browser uses cookies from the user's configured Gecko profile, so logged-in results may appear.",
      "After searching, use web_browse to read specific result pages.",
    },
    parameters = {
      type = "object",
      properties = {
        query = { type = "string", description = "Search query" },
        engine = { type = "string", enum = { "google", "duckduckgo", "brave" }, description = 'Search engine (default: "duckduckgo")' },
      },
      required = { "query" },
    },
    renderCall = function(args, theme)
      local engine = args.engine or DEFAULT_ENGINE
      local s = theme:fg("toolTitle", theme:bold("web_search ")) .. theme:fg("muted", "[" .. engine .. "] ") .. theme:fg("dim", '"' .. tostring(args.query or "") .. '"')
      return { text = s }
    end,
    execute = function(_tool_call_id, params, signal, on_update, _ctx)
      local engine = params.engine or DEFAULT_ENGINE
      local build_url = SEARCH_URLS[engine]
      if not build_url then
        error('Unknown search engine: "' .. engine .. '". Use google, duckduckgo, or brave.', 0)
      end
      local ok_result, result = pcall(function()
        local function update(msg)
          if on_update then on_update({ content = { { type = "text", text = msg } }, details = nil }) end
        end
        update("Acquiring browser...")
        return browser.with_client(function(client)
          if signal and signal:is_aborted() then error("Aborted", 0) end
          local url = build_url(params.query)
          update("Searching " .. engine .. "...")
          pcall(client.navigate, client, url, 30000)
          if signal and signal:is_aborted() then error("Aborted", 0) end
          update("Extracting results...")
          local html = client.get_page_source(10000)
          local results = parse_search_results(html, engine)
          if #results == 0 then
            local ok_script, page_text = pcall(client.execute_script, client, 'return document.body?.innerText || document.documentElement?.innerText || ""', {}, 5000)
            local diagnostic = search_failure_diagnostic(page_text, engine)
            if diagnostic then
              return { content = { { type = "text", text = truncate_output(diagnostic) } }, details = { engine = engine, query = params.query, resultCount = 0, blocked = true } }
            end
          end
          local formatted = format_results(results)
          local output = 'Search results for "' .. tostring(params.query) .. '" (' .. engine .. ", " .. #results .. " results):\n\n" .. formatted
          return { content = { { type = "text", text = truncate_output(output) } }, details = { engine = engine, query = params.query, resultCount = #results } }
        end)
      end)
      if not ok_result then error(result, 0) end
      return result
    end,
  })
end

local function browse_tool(browser)
  pi.register_tool({
    name = "web_browse",
    label = "Web Browse",
    description = "Browse a URL using a real Gecko browser. Returns page content as text. Optionally run a JS extraction script to pull specific data from the page.",
    promptSnippet = "Browse a URL via a Gecko browser and return its content (supports JS extraction)",
    promptGuidelines = {
      "Use web_browse to read a specific page after finding its URL via web_search.",
      "For large pages, provide an `extract` script to get just the relevant content.",
      "The extract parameter is a JS expression evaluated in the page — it must return a string.",
      'Example extract: "document.querySelector(\'article\')?.innerText"',
      "Without extract, you get the full page text (HTML stripped), which may be very large.",
    },
    parameters = {
      type = "object",
      properties = {
        url = { type = "string", description = "URL to navigate to" },
        extract = { type = "string", description = 'JS expression to extract data from the page. Must return a string. Example: "document.querySelector(\'article\')?.innerText"' },
      },
      required = { "url" },
    },
    renderCall = function(args, theme)
      local s = theme:fg("toolTitle", theme:bold("web_browse ")) .. theme:fg("muted", args.url or "")
      if args.extract then s = s .. theme:fg("dim", " (extract)") end
      return { text = s }
    end,
    execute = function(_tool_call_id, params, signal, on_update, _ctx)
      local ok_result, result = pcall(function()
        local function update(msg)
          if on_update then on_update({ content = { { type = "text", text = msg } }, details = nil }) end
        end
        update("Acquiring browser...")
        return browser.with_client(function(client)
          if signal and signal:is_aborted() then error("Aborted", 0) end
          update("Navigating to " .. tostring(params.url) .. "...")
          pcall(client.navigate, client, params.url, 30000)
          if signal and signal:is_aborted() then error("Aborted", 0) end
          local content
          if params.extract then
            update("Running extraction script...")
            local script = tostring(params.extract):gsub("^%s+", ""):gsub("%s+$", "")
            if script:sub(1, 7) ~= "return " then script = "return " .. script end
            local result_val = client.execute_script(script, {}, 10000)
            content = (type(result_val) == "string") and result_val or pi.json.encode(result_val, true)
          else
            update("Extracting page content...")
            local result_val = client.execute_script('return document.body?.innerText || document.documentElement?.innerText || ""', {}, 10000)
            content = (type(result_val) == "string") and result_val or tostring(result_val)
          end
          local header = "Content from " .. tostring(params.url) .. " (" .. format_size(#content) .. "):\n\n"
          return { content = { { type = "text", text = truncate_output(content, header) } }, details = { url = params.url, extracted = params.extract ~= nil, contentLength = #content } }
        end)
      end)
      if not ok_result then error(result, 0) end
      return result
    end,
  })
end

local function truncate_output(content, prefix)
  prefix = prefix or ""
  local t = truncate_head(content, { maxLines = DEFAULT_MAX_LINES, maxBytes = DEFAULT_MAX_BYTES })
  local text = prefix .. t.content
  if t.truncated then
    text = text .. "\n\n[Truncated: " .. t.outputLines .. "/" .. t.totalLines .. " lines, " .. format_size(t.outputBytes) .. "/" .. format_size(t.totalBytes) .. "]"
  end
  return text
end

-- ── Search-result parsers (pure Lua) ───────────────────────────────
local function decode(s)
  s = s:gsub("&amp;", "&"):gsub("&lt;", "<"):gsub("&gt;", ">"):gsub("&quot;", '"')
      :gsub("&#39;", "'"):gsub("&#x27;", "'"):gsub("&#x2F;", "/"):gsub("&nbsp;", " ")
  s = s:gsub("&#(%d+);", function(n) return string.char(tonumber(n)) end)
  return s
end

local function clean(s) return decode(s:gsub("<[^>]*>", ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function dedup(results)
  local seen = {}
  local out = {}
  for _, x in ipairs(results) do
    if not seen[x.url] then seen[x.url] = true; out[#out + 1] = x end
  end
  return out
end

local function first_match(html, patterns)
  for _, p in ipairs(patterns) do
    if p then
      local m = { html:match(p) }
      if m[1] then return clean(m[1]) end
    end
  end
  return ""
end

local function exec_all(pattern, html, fn)
  local pos = 1
  while true do
    local s, e, m1 = pattern(html, pos)
    if not s then break end
    fn(m1, s, e)
    pos = e + 1
  end
end

local function google_url(u)
  local q = u:match("[?&]q=([^&]+)")
  return q and decode(q) or u
end

local function wikipedia_pct_decode(u)
  return u:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end)
end

local function parse_engine_results(engine, html)
  local results = {}
  if engine == "google" then
    local pos = 1
    while true do
      local s, e, url, title = html:match("():()<a[^>]+href=\"([^\"]*)\"[^>]*>[%s%S]-<h3[^>]*>([%s%S]-)</h3>", pos)
      if not url then break end
      pos = e + 1
      local u = google_url(decode(url))
      local t = clean(title)
      if #t >= 3 and u:match("^http") and not u:match("google%.com/search") then
        local after = html:sub(e + 1, e + 3000)
        local patterns = {
          '<div[^>]*class="[^"]*VwiC3b[^"]*"[^>]*>([%s%S]-)</div>',
          '<span[^>]*class="[^"]*st[^"]*"[^>]*>([%s%S]-)</span>',
          '<div[^>]*data%-sncf="[^"]*"[^>]*>([%s%S]-)</div>',
        }
        local snippet = first_match(after, patterns):sub(1, 300)
        results[#results + 1] = { title = t, url = u, snippet = snippet }
      end
    end
  elseif engine == "duckduckgo" then
    local pos = 1
    while true do
      local s, e, url, title = html:match("():()<a[^>]*class=\"[^\"]*result__a[^\"]*\"[^>]*href=\"([^\"]*)\"[^>]*>([%s%S]-)</a>", pos)
      if not url then break end
      pos = e + 1
      local u = decode(url)
      local t = clean(title)
      if t ~= "" and u:match("^http") then
        local snippet = first_match(html:sub(e + 1, e + 600), { '<a[^>]*class="[^\"]*result__snippet[^\"]*"[^>]*>([%s%S]-)</a>' })
        results[#results + 1] = { title = t, url = u, snippet = snippet }
      end
    end
    if #results == 0 then
      pos = 1
      while true do
        local s, e, url, title = html:match("():()<a[^>]*class=\"[^\"]*result__a[^\"]*\"[^>]*href=\"([^\"]*)\"[^>]*>([%s%S]-)</a>", pos)
        if not url then break end
        pos = e + 1
        local u = decode(url)
        local t = clean(title)
        if t ~= "" and u:match("^http") then results[#results + 1] = { title = t, url = u, snippet = "" } end
      end
    end
  elseif engine == "brave" then
    local pos = 1
    while true do
      local s, e = html:find('<a[^>]+href="(https?://[^"]*)"[^>]*class="[^"]*%l1%f[^"]*"[^>]*>', pos)
      if not s then break end
      pos = e + 1
      local url = html:match('href="(https?://[^"]*)"[^>]*class="[^"]*l1[^"]*"', s)
      -- title extraction fallback
      local tail = html:sub(e + 1, e + 2500)
      local title = tail:match('<div[^>]*class="[^"]*search%-snippet%-title[^"]*"[^>]*>([%s%S]-)</div>') or tail:match('<div[^>]*class="[^"]*act-text[^"]*"[^>]*>([%s%S]-)</div>')
      local t = clean(title or "")
      if url and #t >= 3 and url:match("^http") then
        local snippet = first_match(tail, { '<div[^>]*class="[^"]*generic%-snippet[^"]*"[^>]*>[%s%S]-<div[^>]*class="[^"]*content[^"]*"[^>]*>([%s%S]-)</div>', '<p[^>]*class="[^"]*snippet%-description[^"]*"[^>]*>([%s%S]-)</p>' }):sub(1, 300)
        results[#results + 1] = { title = t, url = wikipedia_pct_decode(url), snippet = snippet }
      end
    end
  else
    -- generic
    local pos = 1
    while true do
      local s, e, url, title = html:match("():()<a[^>]+href=\"(https?://[^\"&]+)\"[^>]*>([%s%S]-)</a>", pos)
      if not url then break end
      pos = e + 1
      local u = decode(url)
      local t = clean(title)
      if #t >= 3 and not u:match("google%.com") and not u:match("duckduckgo%.com") then
        local ctx = html:sub(math.max(1, s - 200), e + 500)
        local snippet = clean(ctx):sub(1, 200)
        results[#results + 1] = { title = t, url = u, snippet = snippet }
      end
    end
  end
  return dedup(results)
end

local function parse_search_results(html, engine)
  engine = tostring(engine or ""):lower()
  local supported = { google = true, duckduckgo = true, brave = true }
  if not supported[engine] then engine = "generic" end
  return parse_engine_results(engine, html)
end

local browser = browser_manager(pi.cwd())
search_tool(browser)
browse_tool(browser)

pi.on("session_shutdown", function()
  pcall(browser.shutdown)
end)
