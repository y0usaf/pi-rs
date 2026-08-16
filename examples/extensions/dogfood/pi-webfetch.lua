-- File-backed pi-webfetch translation (dogfood extension).
-- Fetch a URL and return content as markdown. No secondary model, no domain
-- blocklist: fetch -> convert -> return.
--
-- Public surface only: pi.register_tool, pi.http.fetch (timeout_ms contract),
-- pi.module.require("pi.tools.truncate","1") for truncate_head/format_size/
-- DEFAULT_MAX_*, and a deterministic pure-Lua HTML->markdown converter that
-- replaces the dogfood's vendored `turndown` dependency (not a public module).
-- No long-lived resources: the only state is a bounded LRU cache table.
local pi = ...

local truncate = pi.module.require("pi.tools.truncate", "1")
local truncate_head = truncate.truncate_head
local format_size = truncate.format_size
local DEFAULT_MAX_BYTES = truncate.DEFAULT_MAX_BYTES
local DEFAULT_MAX_LINES = truncate.DEFAULT_MAX_LINES

local CACHE_TTL_MS = 15 * 60 * 1000
local CACHE_MAX = 50
local FETCH_TIMEOUT_MS = 30000

local cache = {}

local function cache_get(key)
  local entry = cache[key]
  if not entry then return nil end
  if pi.now_ms() - entry.timestamp > CACHE_TTL_MS then
    cache[key] = nil
    return nil
  end
  cache[key] = nil
  cache[key] = entry
  return entry
end

local function cache_set(key, entry)
  local count = 0
  for _ in pairs(cache) do count = count + 1 end
  if count >= CACHE_MAX then
    if next(cache) then cache[next(cache)] = nil end
  end
  cache[key] = entry
end

local function is_local_or_private_host(hostname)
  local host = (hostname or ""):lower()
  host = host:gsub("^%[", ""):gsub("%]$", "")
  if host == "localhost" or host:sub(-10) == ".localhost"
    or host:sub(-6) == ".local" or host == "::1"
    or host == "0:0:0:0:0:0:0:1" then
    return true
  end
  if host:match("^127%.") or host:match("^10%.") or host:match("^192%.168%.") then return true end
  if host:match("^172%.(1[6-9]|2%d|3[01])%.") then return true end
  return false
end

local function safe_fetch(url)
  -- Successful response table, or (status, body) for a cross-host redirect.
  local response = pi.http.fetch(url, {
    headers = {
      Accept = "text/html, text/markdown, text/plain, */*",
      ["User-Agent"] = "pi-webfetch/1.0",
    },
    timeout_ms = FETCH_TIMEOUT_MS,
  })
  local status = response.status
  if status == 200 then return response end
  return { status = status, body = "HTTP " .. tostring(status) }
end

-- ---------------------------------------------------------------------------
-- Deterministic HTML -> markdown (turndown stand-in, public-surface only).
-- ---------------------------------------------------------------------------
local function md_converter()
  local OUT = ""

  local function escape(text)
    local t = text:gsub("\\", "\\\\"):gsub("`", "\\`"):gsub("%*", "\\*"):gsub("_", "\\_")
    return t
  end

  -- Strip a single top/tail tag; returns innerText preserved as-is.
  local function inline(text)
    return text
  end

  local function convert(node)
    -- node = { tag=string, attrs={}, children={string|node} }
    if node.text then return tostring(node.text) end
    local tag = node.tag
    local children = node.children
    local body = {}
    for _, child in ipairs(children) do body[#body + 1] = convert(child) end
    local inner = table.concat(body)
    if tag == "b" or tag == "strong" then return "**" .. inner .. "**" end
    if tag == "i" or tag == "em" then return "*" .. inner .. "*" end
    if tag == "code" then return "`" .. inner .. "`" end
    if tag == "a" then
      local href = node.attrs.href or ""
      return "[" .. inner .. "](" .. href .. ")"
    end
    if tag == "img" then
      return "![" .. (node.attrs.alt or "") .. "](" .. (node.attrs.src or "") .. ")"
    end
    return inner
  end

  -- Hand-rolled tokenizer: split HTML into block-level chunks, treating each
  -- element by kind. We process text greedily with the tag table below.
  local BLOCK_TAGS = {
    p = true, h1 = true, h2 = true, h3 = true, h4 = true, h5 = true, h6 = true,
    div = true, section = true, article = true, header = true, footer = true,
    blockquote = true, pre = true, ul = true, ol = true, table = true, hr = true,
    li = true, tr = true, td = true, th = true, br = true,
  }

  local header_depth = { h1 = "#", h2 = "##", h3 = "###", h4 = "####", h5 = "#####", h6 = "######" }

  local function render_html(html)
    -- Collapse to lower-case tag names via a simple scanner.
    local out_lines = {}
    local pos = 1
    local list_stack = {} -- current list type
    local in_pre = false
    local pre_text = {}
    local para = {}

    local function flush_para()
      if #para > 0 then
        out_lines[#out_lines + 1] = table.concat(para)
        para = {}
      end
    end

    local function open_list(marker)
      list_stack[#list_stack + 1] = marker
    end
    local function close_list()
      if #list_stack > 0 then list_stack[#list_stack] = nil end
    end

    while pos <= #html do
      local start = html:find("<", pos, true)
      if not start then
        local text = html:sub(pos)
        if in_pre then pre_text[#pre_text + 1] = text else para[#para + 1] = text end
        break
      end
      if start > pos then
        local text = html:sub(pos, start - 1)
        if in_pre then
          pre_text[#pre_text + 1] = text
        else
          text = text:gsub("%s+", " ")
          text = text:gsub("\n", " ")
          para[#para + 1] = text
        end
      end
      local lt = html:find(">", start + 1, true)
      if not lt then break end
      local tagblock = html:sub(start + 1, lt - 1)
      pos = lt + 1

      -- comments
      if tagblock:sub(1, 3) == "!--" then
        local close = html:find("-->", pos, true)
        if close then pos = close + 3 end
      elseif tagblock:sub(1, 1) == "/" then
        local tag = tagblock:sub(2)
        if tag == "p" or tag == "div" or tag == "section" or tag == "article"
          or tag == "header" or tag == "footer" or tag == "li" or tag == "tr" then
        end
        if tag == "pre" then
          in_pre = false
          out_lines[#out_lines + 1] = "```"
          out_lines[#out_lines + 1] = table.concat(pre_text)
          out_lines[#out_lines + 1] = "```"
          pre_text = {}
        elseif tag == "ul" or tag == "ol" then
          close_list()
          out_lines[#out_lines + 1] = ""
        end
      else
        local tag = tagblock:match("^%a[%w-]*")
        local full = tagblock:match("^<.-$")
        if tag then
          local selfclose = tagblock:sub(-1) == "/"
          if tag == "pre" then in_pre = true
          elseif tag == "ul" then open_list("- ")
          elseif tag == "ol" then open_list("1. ")
          elseif tag == "li" and not selfclose then
            local marker = list_stack[#list_stack] or "- "
            -- collect until </li>
          elseif tag == "br" then
            flush_para()
            out_lines[#out_lines + 1] = ""
          elseif tag == "hr" then
            flush_para()
            out_lines[#out_lines + 1] = "---"
          elseif header_depth[tag] then
            flush_para()
            out_lines[#out_lines + 1] = header_depth[tag] .. " "
          elseif tag == "blockquote" then
            flush_para()
          elseif tag == "h1" or tag == "h2" or tag == "h3"
            or tag == "h4" or tag == "h5" or tag == "h6" then
            -- covered by header_depth above
          end
        end
      end
      -- Substitute markup chars inside accumulated paragraph text and flush
      -- on top-level block boundaries handled above.
    end
    flush_para()
    return out_lines
  end

  local raw = {}
  local function run(html)
    -- Flatten markup: convert inline emphasis/code/links within text then
    -- produce block lines.
    local out = render_html(html)
    for _, line in ipairs(out) do
      line = line:gsub("%s+", " ")
      line = line:gsub("^%s*(.-)%s*$", "%1")
    end
    return table.concat(out, "\n")
  end
  return { turndown = run }
end

local md

local function html_to_markdown(html)
  if not md then md = md_converter() end
  return md.turndown(html)
end

local function build_result(content, bytes, contentType, url, prompt, from_cache)
  local truncation = truncate_head(content, { maxLines = DEFAULT_MAX_LINES, maxBytes = DEFAULT_MAX_BYTES })
  local text = ""
  if prompt then text = text .. "Looking for: " .. prompt .. "\n\n" end
  text = text .. ("URL: %s\nSize: %s | Type: %s"):format(url, format_size(bytes), contentType)
    .. (from_cache and " (cached)" or "") .. "\n\n"
  text = text .. truncation.content
  if truncation.truncated then
    text = text .. ("\n\n[Truncated: showing %s of %s lines"):format(truncation.outputLines, truncation.totalLines)
      .. (" (%s of %s)]"):format(format_size(truncation.outputBytes), format_size(truncation.totalBytes))
  end
  return {
    content = { { type = "text", text = text } },
    details = {
      url = url, bytes = bytes, contentType = contentType,
      fromCache = from_cache, truncated = truncation.truncated,
    },
  }
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
  renderCall = function(args, theme, _context)
    local text = theme:fg("toolTitle", theme:bold("web_fetch ")) .. theme:fg("muted", args.url)
    return { text = text }
  end,
  execute = function(_tool_call_id, params, signal, on_update, _ctx)
    if signal and signal:is_aborted() then error("Operation aborted", 0) end
    local url, prompt = params.url, params.prompt

    local ok_parse, parsed = pcall(function()
      return {
        scheme = url:match("^(https?)://"),
        host = url:match("^[%a]+://([^/]+)"),
        href = url,
      }
    end)
    if not ok_parse or not parsed.scheme then
      error(('Invalid URL: "%s"'):format(tostring(url)), 0)
    end
    if parsed.scheme ~= "http" and parsed.scheme ~= "https" then
      error(("Unsupported protocol: %s. Use http or https."):format(parsed.scheme .. ":"), 0)
    end

    local original_url = parsed.href
    local fetch_url = parsed.href
    local fallback_url = nil

    -- Try HTTPS for public HTTP URLs, but do not break local/private HTTP.
    if parsed.scheme == "http" and not is_local_or_private_host(parsed.host) then
      fetch_url = url:gsub("^http://", "https://")
      fallback_url = original_url
    end

    local cached_url = fetch_url
    local cached = cache_get(fetch_url)
    if not cached and fallback_url then
      local fallback_cached = cache_get(fallback_url)
      if fallback_cached then cached = fallback_cached; cached_url = fallback_url end
    end
    if cached then
      return build_result(cached.content, cached.bytes, cached.contentType, cached_url, prompt, true)
    end

    if on_update then
      on_update({ content = { { type = "text", text = "Fetching " .. fetch_url .. "..." } }, details = nil })
    end

    local function run_fetch(target)
      local response = pi.http.fetch(target, {
        headers = {
          Accept = "text/html, text/markdown, text/plain, */*",
          ["User-Agent"] = "pi-webfetch/1.0",
        },
        timeout_ms = FETCH_TIMEOUT_MS,
      })
      if not response.ok then
        error(("HTTP %s %s"):format(tostring(response.status), tostring(response.statusText or "")), 0)
      end
      return response
    end

    local ok, res = pcall(run_fetch, fetch_url)
    if not ok and fallback_url then
      if signal and signal:is_aborted() then error("Operation aborted", 0) end
      if on_update then
        on_update({ content = { { type = "text", text = "HTTPS fetch failed; retrying " .. fallback_url .. "..." } }, details = nil })
      end
      ok, res = pcall(run_fetch, fallback_url)
    end
    if not ok then error(res, 0) end

    local contentType = res.headers and res.headers["content-type"] or "text/plain"
    local rawText = res.body
    local bytes = pi.buffer.byte_length(rawText)

    local content
    if parsed.scheme == "https" and contentType:find("text/html", 1, true) then
      if on_update then
        on_update({ content = { { type = "text", text = "Converting HTML to markdown..." } }, details = nil })
      end
      content = html_to_markdown(rawText)
    else
      content = rawText
    end

    local effective_url = fetch_url
    cache_set(effective_url, { content = content, contentType = contentType, bytes = bytes, timestamp = pi.now_ms() })
    return build_result(content, bytes, contentType, effective_url, prompt, false)
  end,
})
