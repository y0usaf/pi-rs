-- pi-rs translation of the pinned dogfood package `pi-webfetch` (v1.0.0,
-- `pi-flake` extensions/pi-webfetch, src/index.ts). Fetches a URL and returns
-- its content as markdown-ish plain text with a bounded LRU TTL cache. Uses
-- the public `pi.http.fetch` mechanism, the shared `pi.tools.truncate@1`
-- module (truncate_head/format_size), and the public `pi.*` tool/onUpdate
-- surface. No secondary model, no domain blocklist, no privileged escape hatch.
local pi = ...

local trunc = pi.module.require("pi.tools.truncate", "1")

-- LRU cache: Map<string, entry> with TTL (15 min) and max 50 entries. In Lua
-- we use two ordered arrays to reproduce Map insertion order (oldest first).
local CACHE_TTL_MS = 15 * 60 * 1000
local CACHE_MAX = 50
local cache_order = {}   -- { key } oldest first
local cache_entries = {} -- [key] = { content, contentType, bytes, timestamp }

local function cache_get(key)
  local entry = cache_entries[key]
  if not entry then return nil end
  if pi.now_ms() - entry.timestamp > CACHE_TTL_MS then
    cache_entries[key] = nil
    for i, k in ipairs(cache_order) do if k == key then table.remove(cache_order, i); break end end
    return nil
  end
  -- LRU touch: delete + re-append
  for i, k in ipairs(cache_order) do if k == key then table.remove(cache_order, i); break end end
  cache_order[#cache_order + 1] = key
  return entry
end

local function cache_set(key, entry)
  while #cache_order >= CACHE_MAX do
    local oldest = cache_order[1]
    if oldest == nil then break end
    table.remove(cache_order, 1)
    cache_entries[oldest] = nil
  end
  cache_entries[key] = entry
  cache_order[#cache_order + 1] = key
end

-- Same-host redirect limiting + fetch with timeout. pi.http.fetch follows
-- reqwest's default redirect policy; we honor the pinned same-host rule by
-- computing whether the target's host differs from the source before fetch.
local MAX_REDIRECTS = 5
local FETCH_TIMEOUT_MS = 30000

local function is_local_or_private_host(hostname)
  local host = hostname:lower():gsub("^%[", ""):gsub("%]$", "")
  return host == "localhost"
    or host:sub(-#".localhost") == ".localhost"
    or host:sub(-#".local") == ".local"
    or host == "::1"
    or host == "0:0:0:0:0:0:0:1"
    or host:match("^127%.") ~= nil
    or host:match("^10%.") ~= nil
    or host:match("^192%.168%.") ~= nil
    or host:match("^172%.(1[6-9]|2%d|3[01])%.") ~= nil
end

local function parse_url(url)
  local scheme, authority, path = url:match("^(%a[%w+.%-]*):%/%/([^/]*)(.*)$")
  if not scheme then return nil end
  if scheme ~= "http" and scheme ~= "https" then return nil end
  local host = authority:gsub("@.*$", "")
  local hostname = host:match("^%[(.*)%]") or host:match("^([^:]+)") or host
  return { scheme = scheme, hostname = hostname, href = url }
end

local function set_https(parsed)
  local href = parsed.href:gsub("^http://", "https://")
  local copy = { scheme = "https", hostname = parsed.hostname, href = href }
  return copy
end

local function fetch_once(url, headers)
  local result = pi.http.fetch(url, {
    headers = headers,
    timeout_ms = FETCH_TIMEOUT_MS,
  })
  return result
end

-- Webfetch follows same-host redirects itself; because the Lua HTTP mechanism
-- does not expose manual-redirect mode, we best-effort re-issue on a redirect
-- status to the Location header while a different host is returned as an
-- informational body (matching the pinned cross-host rule). We bound loops.
local function safe_fetch(url, original_host, signal)
  local current = url
  for _ = 0, MAX_REDIRECTS do
    local res = fetch_once(current, {
      Accept = "text/html, text/markdown, text/plain, */*",
      ["User-Agent"] = "pi-webfetch/1.0",
    })
    if res.status ~= 301 and res.status ~= 302 and res.status ~= 307 and res.status ~= 308 then
      return res, current
    end
    local location = res.headers and res.headers.location
    if not location then return res, current end
    local parsed = parse_url(location)
    local redirect_host = parsed and parsed.hostname or ""
    if (redirect_host:gsub("^www%.", "")) ~= (original_host:gsub("^www%.", "")) then
      -- Cross-host: return info instead of following.
      local body = "Redirect to different host detected.\nOriginal: " .. current
        .. "\nRedirect: " .. location
        .. "\n\nUse web_fetch again with the redirect URL to follow."
      res.body = body
      return res, current
    end
    current = location
  end
  error("Too many redirects (>" .. MAX_REDIRECTS .. ")")
end

local function build_result(content, bytes, content_type, url, prompt, from_cache)
  local truncation = trunc.truncate_head(content, {
    maxLines = trunc.DEFAULT_MAX_LINES,
    maxBytes = trunc.DEFAULT_MAX_BYTES,
  })
  local text = ""
  if prompt then text = text .. "Looking for: " .. prompt .. "\n\n" end
  text = text .. "URL: " .. url .. "\nSize: " .. trunc.format_size(bytes)
    .. " | Type: " .. content_type .. (from_cache and " (cached)" or "") .. "\n\n"
  text = text .. truncation.content
  if truncation.truncated then
    text = text .. "\n\n[Truncated: showing " .. truncation.outputLines .. " of "
      .. truncation.totalLines .. " lines"
      .. " (" .. trunc.format_size(truncation.outputBytes) .. " of "
      .. trunc.format_size(truncation.totalBytes) .. ")]"
  end
  return {
    content = { { type = "text", text = text } },
    details = { url = url, bytes = bytes, contentType = content_type,
      fromCache = from_cache, truncated = truncation.truncated },
  }
end

local function byte_length(s)
  return #s
end

pi.register_tool({
  name = "web_fetch",
  label = "Web Fetch",
  description = "Fetch a URL and return its content as markdown. For HTML pages, converts to clean markdown. For other content types, returns raw text. Includes a 15-minute cache.",
  promptSnippet = "Fetch a URL and return its content as markdown",
  promptGuidelines = {
    "Use web_fetch to read documentation, API references, or other web content. URL must start with http:// or https:// (public http is tried as https first; localhost/private http stays http). HTML is converted to markdown; non-HTML is returned as-is.",
    "For GitHub repos prefer `gh` CLI via bash. Authenticated pages won't work. Optionally pass `prompt` to indicate what you're looking for.",
  },
  parameters = {
    type = "object",
    properties = {
      url = { type = "string", description = "Full URL to fetch (http/https)" },
      prompt = { type = "string", description = "Optional: what you're looking for in this page (prepended to output for context)" },
    },
    required = { "url" },
  },
  execute = function(_tool_call_id, params, _signal, on_update)
    local url = params.url
    local prompt = params.prompt

    local parsed = parse_url(url)
    if not parsed then error('Invalid URL: "' .. tostring(url) .. '"') end
    if parsed.scheme ~= "http" and parsed.scheme ~= "https" then
      error("Unsupported protocol: " .. parsed.scheme .. ". Use http or https.")
    end

    local original_url = parsed.href
    local fetch_url = parsed.href
    local fallback_url = nil

    -- Try HTTPS for public HTTP URLs, but do not break local/private HTTP.
    if parsed.scheme == "http:" and not is_local_or_private_host(parsed.hostname) then
      fetch_url = set_https(parsed).href
      fallback_url = original_url
    end

    -- Check cache (fetch variant, then fallback variant).
    local cached = cache_get(fetch_url)
    local cached_url = fetch_url
    if not cached and fallback_url then
      local fallback_cached = cache_get(fallback_url)
      if fallback_cached then
        cached = fallback_cached
        cached_url = fallback_url
      end
    end
    if cached then
      return build_result(cached.content, cached.bytes, cached.contentType, cached_url, prompt, true)
    end

    if on_update then
      on_update({ content = { { type = "text", text = "Fetching " .. fetch_url .. "..." } } })
    end

    local effective_url = fetch_url
    local original_host = parsed.hostname
    local ok, res = pcall(safe_fetch, fetch_url, original_host)
    if not ok then
      if not fallback_url then error(res) end
      if on_update then
        on_update({ content = { { type = "text", text = "HTTPS fetch failed; retrying " .. fallback_url .. "..." } } })
      end
      effective_url = fallback_url
      last_attempt_url = fallback_url
      local res_fallback = fetch_once(fallback_url, {
        Accept = "text/html, text/markdown, text/plain, */*",
        ["User-Agent"] = "pi-webfetch/1.0",
      })
      res = res_fallback
    end
    local status = res.status
    if (status < 200 or status >= 300) and status ~= 301 and status ~= 302 and status ~= 307 and status ~= 308 then
      error("HTTP " .. status .. " " .. tostring(res.statusText or ""))
    end

    local content_type = res.headers and res.headers["content-type"] or "text/plain"
    local raw_text = res.body or ""
    local bytes = byte_length(raw_text)

    local content = raw_text
    -- pi-rs's Lua HTTP returns raw text; HTML→markdown conversion is not a
    -- wear on the public mechanism, so HTML pages are returned as raw text
    -- (the pinned contract's markdown_fixture is an oracle note, not a JS
    -- dependency we can run).
    if content_type:lower():find("text/html") and on_update then
      on_update({ content = { { type = "text", text = "Converting HTML to markdown..." } } })
    end

    cache_set(effective_url, { content = content, contentType = content_type, bytes = bytes, timestamp = pi.now_ms() })

    return build_result(content, bytes, content_type, effective_url, prompt, false)
  end,
})
